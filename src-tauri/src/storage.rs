use crate::models::{App, AppData, Settings, SourceType};
use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

pub struct Storage {
    file_path: PathBuf,
    data: Mutex<AppData>,
}

/// Obtainintosh tracks itself by default: put the app's own entry at the top of
/// fresh (or pre-seeding) data files. Runs once per data file — after the
/// `self_entry_seeded` marker is set, removing the entry sticks. An existing
/// entry for the same repository (added by hand) is left alone. Returns whether
/// `data` changed and needs saving.
fn seed_self_entry(data: &mut AppData) -> bool {
    if data.self_entry_seeded {
        return false;
    }
    if !data.apps.iter().any(crate::updates::is_self_app) {
        data.apps.insert(0, crate::updates::self_app_entry());
    }
    data.self_entry_seeded = true;
    true
}

fn prepare_storage_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(PRIVATE_DIRECTORY_MODE);
        builder
            .create(path)
            .context("Failed to create application support directory")?;
        fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE))
            .context("Failed to secure application support directory")?;
    }

    #[cfg(not(unix))]
    fs::create_dir_all(path).context("Failed to create application support directory")?;

    Ok(())
}

fn tighten_storage_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))
        .context("Failed to secure apps.json")?;

    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

struct PrivateTempFile {
    path: PathBuf,
    file: Option<fs::File>,
    cleanup: bool,
}

impl PrivateTempFile {
    fn create(destination: &Path) -> io::Result<Self> {
        let file_name = destination.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Storage path has no file name")
        })?;
        let mut options = fs::OpenOptions::new();
        // Exclusive creation atomically rejects pre-existing files and symlinks.
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(PRIVATE_FILE_MODE);

        loop {
            let mut temp_name = file_name.to_os_string();
            temp_name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
            let path = destination.with_file_name(temp_name);
            match options.open(&path) {
                Ok(file) => {
                    let temp = Self {
                        path,
                        file: Some(file),
                        cleanup: true,
                    };
                    #[cfg(unix)]
                    temp.file
                        .as_ref()
                        .unwrap()
                        .set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
                    return Ok(temp);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }

    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }

    fn write_all(&mut self, contents: &[u8]) -> io::Result<()> {
        self.file.as_mut().unwrap().write_all(contents)
    }

    fn replace(mut self, destination: &Path) -> io::Result<()> {
        self.file.take();
        fs::rename(&self.path, destination)?;
        self.cleanup = false;
        Ok(())
    }
}

impl Drop for PrivateTempFile {
    fn drop(&mut self) {
        self.file.take();
        if self.cleanup {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Storage {
    pub fn new() -> Result<Self> {
        let app_support = dirs::home_dir()
            .context("Failed to get home directory")?
            .join("Library")
            .join("Application Support")
            .join("Obtainintosh");

        prepare_storage_directory(&app_support)?;

        Self::load_from_path(app_support.join("apps.json"))
    }

    fn load_from_path(file_path: PathBuf) -> Result<Self> {
        // Load existing data or create new
        let mut data = match fs::symlink_metadata(&file_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    anyhow::bail!("Refusing to use apps.json because it is a symbolic link");
                }
                if !metadata.file_type().is_file() {
                    anyhow::bail!("Refusing to use apps.json because it is not a regular file");
                }

                tighten_storage_file_permissions(&file_path)?;
                let contents = fs::read(&file_path).context("Failed to read apps.json")?;
                match serde_json::from_slice(&contents) {
                    Ok(data) => data,
                    Err(error) => {
                        let backup_path = backup_corrupt_file(&file_path).with_context(|| {
                            format!("Failed to preserve unreadable apps.json after: {error}")
                        })?;
                        eprintln!(
                            "Obtainintosh could not load persisted JSON at {} ({error}). The original data was preserved at {}. Starting with defaults.",
                            file_path.display(),
                            backup_path.display()
                        );
                        AppData::default()
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => AppData::default(),
            Err(error) => return Err(error).context("Failed to inspect apps.json"),
        };

        let seeded = seed_self_entry(&mut data);

        let storage = Self {
            file_path,
            data: Mutex::new(data),
        };
        if seeded {
            let data = storage.data.lock().unwrap();
            storage
                .persist(&data)
                .context("Failed to save the seeded self-entry")?;
        }
        Ok(storage)
    }

    fn persist(&self, data: &AppData) -> Result<()> {
        let json = serde_json::to_string_pretty(data).context("Failed to serialize data")?;

        // Atomic write: write to a private sibling file then rename.
        let mut temp =
            PrivateTempFile::create(&self.file_path).context("Failed to write temp file")?;
        temp.write_all(json.as_bytes())
            .context("Failed to write temp file")?;
        temp.replace(&self.file_path)
            .context("Failed to rename temp file")?;

        Ok(())
    }

    pub fn get_all_apps(&self) -> Result<Vec<App>> {
        let data = self.data.lock().unwrap();
        Ok(data.apps.clone())
    }

    pub fn get_app(&self, id: &str) -> Result<Option<App>> {
        let data = self.data.lock().unwrap();
        Ok(data.apps.iter().find(|app| app.id == id).cloned())
    }

    pub fn add_app(&self, mut app: App) -> Result<App> {
        let new_identity = match app.source_type {
            SourceType::GitHub => crate::sources::normalize_repo_url(&app.source_url)?,
            SourceType::GitLab => anyhow::bail!(
                "GitLab repositories are not supported; enter a GitHub repository URL instead"
            ),
        };

        // Generate UUID if not provided
        if app.id.is_empty() {
            app.id = uuid::Uuid::new_v4().to_string();
        }

        let mut data = self.data.lock().unwrap();
        if has_duplicate_github_source(&data.apps, &new_identity, None) {
            anyhow::bail!("This repository is already being tracked");
        }
        let mut proposed = data.clone();
        proposed.apps.push(app.clone());

        self.persist(&proposed)?;
        *data = proposed;
        Ok(app)
    }

    pub fn update_app(&self, updated_app: App) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let index = data
            .apps
            .iter()
            .position(|app| app.id == updated_app.id)
            .with_context(|| format!("App not found: {}", updated_app.id))?;

        match updated_app.source_type {
            SourceType::GitHub => {
                let identity = crate::sources::normalize_repo_url(&updated_app.source_url)?;
                if has_duplicate_github_source(&data.apps, &identity, Some(&updated_app.id)) {
                    anyhow::bail!("This repository is already being tracked");
                }
            }
            SourceType::GitLab
                if !matches!(data.apps[index].source_type, SourceType::GitLab)
                    || data.apps[index].source_url != updated_app.source_url =>
            {
                anyhow::bail!(
                    "GitLab repositories are not supported; enter a GitHub repository URL instead"
                );
            }
            SourceType::GitLab => {}
        }

        let mut proposed = data.clone();
        proposed.apps[index] = updated_app;

        self.persist(&proposed)?;
        *data = proposed;
        Ok(())
    }

    pub fn remove_app(&self, id: &str) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let mut proposed = data.clone();
        proposed.apps.retain(|app| app.id != id);

        self.persist(&proposed)?;
        *data = proposed;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<Settings> {
        let data = self.data.lock().unwrap();
        Ok(data.settings.clone())
    }

    pub fn update_settings(&self, settings: Settings) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let mut proposed = data.clone();
        proposed.settings = settings;

        self.persist(&proposed)?;
        *data = proposed;
        Ok(())
    }
}

fn backup_corrupt_file(file_path: &Path) -> Result<PathBuf> {
    let file_name = file_path
        .file_name()
        .context("Storage path has no file name")?;

    for index in 0_u64.. {
        let mut backup_name = file_name.to_os_string();
        backup_name.push(".corrupt-backup");
        if index > 0 {
            backup_name.push(format!(".{index}"));
        }
        let backup_path = file_path.with_file_name(backup_name);
        match fs::hard_link(file_path, &backup_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("Failed to create corrupt storage backup"),
        }
        fs::remove_file(file_path).context("Failed to remove storage after backing it up")?;
        return Ok(backup_path);
    }

    unreachable!("u64 backup suffixes were exhausted")
}

fn has_duplicate_github_source(apps: &[App], identity: &str, excluding_id: Option<&str>) -> bool {
    apps.iter().any(|app| {
        excluding_id != Some(app.id.as_str())
            && matches!(app.source_type, SourceType::GitHub)
            && crate::sources::normalize_repo_url(&app.source_url)
                .is_ok_and(|existing| existing == identity)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{App, SourceType};
    use std::path::Path;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "obtainintosh-storage-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn storage_temp_paths(file_path: &Path) -> Vec<PathBuf> {
        let prefix = format!("{}.", file_path.file_name().unwrap().to_string_lossy());
        let mut paths = fs::read_dir(file_path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy();
                (name.starts_with(&prefix) && name.ends_with(".tmp")).then_some(path)
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    fn test_storage() -> (Storage, TestDir) {
        let temp_dir = TestDir::new();
        let storage = Storage {
            file_path: temp_dir.path().join("apps.json"),
            data: Mutex::new(AppData::default()),
        };
        (storage, temp_dir)
    }

    fn failing_storage(data: AppData) -> (Storage, TestDir) {
        let temp_dir = TestDir::new();
        let storage = Storage {
            file_path: temp_dir.path().join("missing").join("apps.json"),
            data: Mutex::new(data),
        };
        (storage, temp_dir)
    }

    fn app(id: &str, source_type: SourceType, source_url: &str) -> App {
        App {
            id: id.to_string(),
            name: id.to_string(),
            source_type,
            source_url: source_url.to_string(),
            current_version: None,
            latest_version: None,
            install_path: None,
            last_checked: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn storage_directory_is_owner_only_when_created_or_tightened() {
        let temp_dir = TestDir::new();
        let new_path = temp_dir.path().join("new").join("Obtainintosh");
        prepare_storage_directory(&new_path).unwrap();
        assert_eq!(mode(&new_path), PRIVATE_DIRECTORY_MODE);

        let existing_path = temp_dir.path().join("existing");
        fs::create_dir(&existing_path).unwrap();
        set_mode(&existing_path, 0o777);
        prepare_storage_directory(&existing_path).unwrap();
        assert_eq!(mode(&existing_path), PRIVATE_DIRECTORY_MODE);
    }

    #[cfg(unix)]
    #[test]
    fn startup_tightens_an_existing_apps_file_without_rewriting_it() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        let data = AppData {
            self_entry_seeded: true,
            ..AppData::default()
        };
        let original = serde_json::to_vec(&data).unwrap();
        fs::write(&file_path, &original).unwrap();
        set_mode(&file_path, 0o666);

        Storage::load_from_path(file_path.clone()).unwrap();

        assert_eq!(fs::read(&file_path).unwrap(), original);
        assert_eq!(mode(&file_path), PRIVATE_FILE_MODE);
    }

    #[cfg(unix)]
    #[test]
    fn startup_rejects_an_apps_symlink_without_touching_its_target() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        let target_path = temp_dir.path().join("target.json");
        let data = AppData {
            self_entry_seeded: true,
            ..AppData::default()
        };
        let original = serde_json::to_vec(&data).unwrap();
        fs::write(&target_path, &original).unwrap();
        set_mode(&target_path, 0o640);
        std::os::unix::fs::symlink(&target_path, &file_path).unwrap();

        let error = Storage::load_from_path(file_path.clone())
            .map(|_| ())
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Refusing to use apps.json because it is a symbolic link"
        );
        assert_eq!(fs::read(&target_path).unwrap(), original);
        assert_eq!(mode(&target_path), 0o640);
        assert!(fs::symlink_metadata(&file_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!temp_dir.path().join("apps.json.corrupt-backup").exists());
    }

    #[test]
    fn startup_rejects_a_non_regular_apps_path() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        fs::create_dir(&file_path).unwrap();

        let error = Storage::load_from_path(file_path).map(|_| ()).unwrap_err();

        assert_eq!(
            error.to_string(),
            "Refusing to use apps.json because it is not a regular file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn atomic_rewrite_replaces_a_loose_apps_file_with_an_owner_only_file() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        let data = AppData {
            self_entry_seeded: true,
            ..AppData::default()
        };
        fs::write(&file_path, serde_json::to_vec(&data).unwrap()).unwrap();
        let storage = Storage::load_from_path(file_path.clone()).unwrap();
        set_mode(&file_path, 0o666);

        storage
            .update_settings(Settings {
                github_token: Some("secret".to_string()),
                gitlab_token: None,
            })
            .unwrap();

        assert_eq!(mode(&file_path), PRIVATE_FILE_MODE);
        assert!(storage_temp_paths(&file_path).is_empty());
    }

    #[test]
    fn private_temp_files_are_unique_siblings_and_cleanup_on_drop() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        let first = PrivateTempFile::create(&file_path).unwrap();
        let second = PrivateTempFile::create(&file_path).unwrap();
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();

        assert_ne!(first_path, second_path);
        assert_eq!(first_path.parent(), file_path.parent());
        assert_eq!(second_path.parent(), file_path.parent());
        #[cfg(unix)]
        {
            assert_eq!(mode(&first_path), PRIVATE_FILE_MODE);
            assert_eq!(mode(&second_path), PRIVATE_FILE_MODE);
        }
        assert_eq!(storage_temp_paths(&file_path).len(), 2);

        drop(first);
        drop(second);
        assert!(storage_temp_paths(&file_path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn fixed_temp_symlink_is_not_followed() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        let fixed_temp_path = file_path.with_extension("json.tmp");
        let victim_path = temp_dir.path().join("victim");
        fs::write(&victim_path, b"untouched").unwrap();
        std::os::unix::fs::symlink(&victim_path, &fixed_temp_path).unwrap();
        let storage = Storage {
            file_path: file_path.clone(),
            data: Mutex::new(AppData::default()),
        };

        storage.persist(&AppData::default()).unwrap();

        assert_eq!(fs::read(&victim_path).unwrap(), b"untouched");
        assert!(fs::symlink_metadata(&fixed_temp_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(storage_temp_paths(&file_path), vec![fixed_temp_path]);
        assert_eq!(mode(&file_path), PRIVATE_FILE_MODE);
    }

    #[test]
    fn failed_atomic_replacement_cleans_unique_temp_file() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        fs::create_dir(&file_path).unwrap();
        let storage = Storage {
            file_path: file_path.clone(),
            data: Mutex::new(AppData::default()),
        };

        let error = storage.persist(&AppData::default()).unwrap_err();

        assert!(
            error.to_string().contains("rename"),
            "unexpected error: {error}"
        );
        assert!(storage_temp_paths(&file_path).is_empty());
    }

    #[test]
    fn loads_older_data_with_missing_non_identity_fields() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        fs::write(
            &file_path,
            r#"{
                "apps": [{
                    "id": "legacy-id",
                    "name": "Legacy App",
                    "source_type": "github",
                    "source_url": "https://github.com/owner/legacy"
                }],
                "settings": { "github_token": "legacy-token" }
            }"#,
        )
        .unwrap();

        let storage = Storage::load_from_path(file_path).unwrap();

        let legacy = storage.get_app("legacy-id").unwrap().unwrap();
        assert_eq!(legacy.name, "Legacy App");
        assert_eq!(legacy.source_url, "https://github.com/owner/legacy");
        assert_eq!(legacy.current_version, None);
        assert_eq!(legacy.latest_version, None);
        assert_eq!(legacy.install_path, None);
        assert_eq!(legacy.last_checked, None);
        let settings = storage.get_settings().unwrap();
        assert_eq!(settings.github_token.as_deref(), Some("legacy-token"));
        assert_eq!(settings.gitlab_token, None);
    }

    #[test]
    fn malformed_json_is_backed_up_before_recovery() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        let malformed = b"{ not valid json";
        fs::write(&file_path, malformed).unwrap();
        #[cfg(unix)]
        set_mode(&file_path, 0o666);

        let storage = Storage::load_from_path(file_path.clone()).unwrap();

        let backup_path = temp_dir.path().join("apps.json.corrupt-backup");
        assert_eq!(fs::read(&backup_path).unwrap(), malformed);
        #[cfg(unix)]
        {
            assert_eq!(mode(&backup_path), PRIVATE_FILE_MODE);
            assert_eq!(mode(&file_path), PRIVATE_FILE_MODE);
        }
        assert!(storage
            .get_all_apps()
            .unwrap()
            .iter()
            .any(crate::updates::is_self_app));
        serde_json::from_slice::<AppData>(&fs::read(file_path).unwrap()).unwrap();
    }

    #[test]
    fn corrupt_backups_do_not_collide() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        fs::write(&file_path, b"first malformed file").unwrap();
        Storage::load_from_path(file_path.clone()).unwrap();

        fs::write(&file_path, b"second malformed file").unwrap();
        Storage::load_from_path(file_path).unwrap();

        assert_eq!(
            fs::read(temp_dir.path().join("apps.json.corrupt-backup")).unwrap(),
            b"first malformed file"
        );
        assert_eq!(
            fs::read(temp_dir.path().join("apps.json.corrupt-backup.1")).unwrap(),
            b"second malformed file"
        );
    }

    #[test]
    fn valid_data_loads_without_rewriting_or_backup() {
        let temp_dir = TestDir::new();
        let file_path = temp_dir.path().join("apps.json");
        let valid = r#"{"apps":[{"id":"valid-id","name":"Valid App","source_type":"github","source_url":"https://github.com/owner/valid","current_version":"1.0.0","latest_version":null,"install_path":"/Applications/Valid.app","last_checked":null}],"settings":{"github_token":null,"gitlab_token":null},"self_entry_seeded":true}"#;
        fs::write(&file_path, valid).unwrap();

        let storage = Storage::load_from_path(file_path.clone()).unwrap();

        assert_eq!(storage.get_all_apps().unwrap().len(), 1);
        assert_eq!(
            storage.get_app("valid-id").unwrap().unwrap().name,
            "Valid App"
        );
        assert_eq!(fs::read_to_string(&file_path).unwrap(), valid);
        assert!(!temp_dir.path().join("apps.json.corrupt-backup").exists());
    }

    #[test]
    fn seeds_self_entry_into_fresh_data() {
        let mut data = AppData::default();
        assert!(seed_self_entry(&mut data));
        assert_eq!(data.apps.len(), 1);
        assert_eq!(data.apps[0].name, "Obtainintosh");
        assert_eq!(data.apps[0].source_url, crate::updates::self_repo_url());
        assert!(data.self_entry_seeded);
    }

    #[test]
    fn seeding_runs_once_so_removal_sticks() {
        let mut data = AppData::default();
        seed_self_entry(&mut data);
        data.apps.clear(); // the user removes the entry
        assert!(!seed_self_entry(&mut data)); // later launches leave it removed
        assert!(data.apps.is_empty());
    }

    #[test]
    fn does_not_duplicate_a_manually_added_self_entry() {
        // Same repo, different spellings: matching uses the same canonical
        // GitHub identity as Storage::add_app's dedupe.
        // Derived from self_repo_url() so these keep exercising the match
        // paths if OWNER/REPO ever change.
        let variants = [
            format!("{}/", crate::updates::self_repo_url().to_uppercase()),
            format!("{}.git", crate::updates::self_repo_url()),
        ];
        for source_url in variants {
            let mut data = AppData::default();
            data.apps.push(App {
                id: "existing".to_string(),
                name: "Obtainintosh (mine)".to_string(),
                source_type: SourceType::GitHub,
                source_url: source_url.clone(),
                current_version: None,
                latest_version: None,
                install_path: None,
                last_checked: None,
            });
            assert!(seed_self_entry(&mut data)); // still marks the file as seeded
            assert_eq!(data.apps.len(), 1, "duplicated for {}", source_url);
            assert_eq!(data.apps[0].id, "existing");
            assert!(data.self_entry_seeded);
        }
    }

    #[test]
    fn add_rejects_equivalent_github_urls() {
        let (storage, _temp_dir) = test_storage();
        storage
            .add_app(app(
                "first",
                SourceType::GitHub,
                "https://github.com/Owner/Repo",
            ))
            .unwrap();

        let error = storage
            .add_app(app(
                "second",
                SourceType::GitHub,
                "https://www.github.com/owner/repo.git/releases/latest?download=1#asset",
            ))
            .unwrap_err()
            .to_string();

        assert_eq!(error, "This repository is already being tracked");
        assert_eq!(storage.get_all_apps().unwrap().len(), 1);
    }

    #[test]
    fn edit_rejects_a_duplicate_github_identity_atomically() {
        let (storage, _temp_dir) = test_storage();
        storage
            .add_app(app(
                "first",
                SourceType::GitHub,
                "https://github.com/owner/first",
            ))
            .unwrap();
        storage
            .add_app(app(
                "second",
                SourceType::GitHub,
                "https://github.com/owner/second",
            ))
            .unwrap();

        let mut second = storage.get_app("second").unwrap().unwrap();
        second.source_url =
            "https://www.github.com/OWNER/FIRST.git/tree/main?tab=readme#files".to_string();
        let error = storage.update_app(second).unwrap_err().to_string();

        assert_eq!(error, "This repository is already being tracked");
        assert_eq!(
            storage.get_app("second").unwrap().unwrap().source_url,
            "https://github.com/owner/second"
        );
    }

    #[test]
    fn add_rejects_gitlab_but_existing_gitlab_records_remain_editable() {
        let (storage, _temp_dir) = test_storage();
        let gitlab: App = serde_json::from_value(serde_json::json!({
            "id": "legacy",
            "name": "legacy",
            "source_type": "gitlab",
            "source_url": "https://gitlab.com/owner/repo",
            "current_version": null,
            "latest_version": null,
            "install_path": null,
            "last_checked": null
        }))
        .unwrap();
        let error = storage.add_app(gitlab.clone()).unwrap_err().to_string();
        assert!(error.contains("GitLab repositories are not supported"));

        storage.data.lock().unwrap().apps.push(gitlab);
        let mut existing = storage.get_app("legacy").unwrap().unwrap();
        existing.name = "Renamed legacy app".to_string();
        storage.update_app(existing).unwrap();

        let preserved = storage.get_app("legacy").unwrap().unwrap();
        assert_eq!(preserved.source_type, SourceType::GitLab);
        assert_eq!(preserved.name, "Renamed legacy app");
    }

    #[test]
    fn failed_add_does_not_change_memory() {
        let (storage, _temp_dir) = failing_storage(AppData::default());

        let error = storage
            .add_app(app(
                "new",
                SourceType::GitHub,
                "https://github.com/owner/new",
            ))
            .unwrap_err()
            .to_string();

        assert_eq!(error, "Failed to write temp file");
        assert!(storage.get_all_apps().unwrap().is_empty());
    }

    #[test]
    fn failed_update_does_not_change_memory() {
        let mut data = AppData::default();
        data.apps.push(app(
            "existing",
            SourceType::GitHub,
            "https://github.com/owner/existing",
        ));
        let (storage, _temp_dir) = failing_storage(data);
        let mut updated = storage.get_app("existing").unwrap().unwrap();
        updated.name = "Changed".to_string();

        let error = storage.update_app(updated).unwrap_err().to_string();

        assert_eq!(error, "Failed to write temp file");
        assert_eq!(
            storage.get_app("existing").unwrap().unwrap().name,
            "existing"
        );
    }

    #[test]
    fn failed_remove_does_not_change_memory() {
        let mut data = AppData::default();
        data.apps.push(app(
            "existing",
            SourceType::GitHub,
            "https://github.com/owner/existing",
        ));
        let (storage, _temp_dir) = failing_storage(data);

        let error = storage.remove_app("existing").unwrap_err().to_string();

        assert_eq!(error, "Failed to write temp file");
        assert!(storage.get_app("existing").unwrap().is_some());
    }

    #[test]
    fn failed_settings_update_does_not_change_memory() {
        let mut data = AppData::default();
        data.settings.github_token = Some("old-token".to_string());
        let (storage, _temp_dir) = failing_storage(data);
        let settings = Settings {
            github_token: Some("new-token".to_string()),
            gitlab_token: Some("new-gitlab-token".to_string()),
        };

        let error = storage.update_settings(settings).unwrap_err().to_string();

        assert_eq!(error, "Failed to write temp file");
        let settings = storage.get_settings().unwrap();
        assert_eq!(settings.github_token.as_deref(), Some("old-token"));
        assert_eq!(settings.gitlab_token, None);
    }
}
