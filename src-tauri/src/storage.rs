use crate::models::{App, AppData, CheckAttempt, CheckAttemptState, Settings};
use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// apps.json holds a GitHub token and, since Forgejo support, per-instance
/// application keys. Nothing but the owner has any business reading either, so
/// the directory and every file written into it are owner-only.
#[cfg(unix)]
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

pub struct Storage {
    file_path: PathBuf,
    data: Mutex<AppData>,
}

/// What happened when a check result was applied. A batch check runs against a
/// snapshot taken before the network work started, so by the time a result
/// lands the user may have removed or edited the app. Writing anyway would
/// resurrect a deleted program or overwrite a fresh edit with stale data, so
/// those cases are reported rather than applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingResultApplication {
    Applied,
    AppRemoved,
    DependenciesChanged,
}

/// The fields a completed check owns. Deliberately narrow: a check has no
/// business writing the name, source, or credentials, so it cannot clobber an
/// edit the user made while it was running.
pub struct CheckOwnedUpdate {
    pub current_version: Option<String>,
    pub install_path: Option<String>,
    pub latest_version: Option<String>,
    pub attempt: CheckAttempt,
}

/// Obtainintosh tracks itself by default: put the app's own entry at the top
/// of any data file that hasn't been seeded yet. That is deliberately not just
/// fresh installs — a data file from a pre-self-tracking version gets the
/// entry once, on the first launch after upgrading. Runs once per data file:
/// after the `self_entry_seeded` marker is set, removing the entry sticks. An
/// existing entry for the same repository (added by hand) is left alone.
/// Returns whether `data` changed and needs saving — setting the marker is
/// itself such a change, even when no entry was inserted.
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

/// Creates the support directory owner-only, and tightens it if a previous
/// version (or the user) left it readable by others.
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

/// The scratch file an atomic save writes before renaming into place.
///
/// It matters that this is created fresh under a unique name rather than
/// written to a fixed `apps.json.tmp`: `fs::write` follows symlinks, so a
/// symlink sitting at a predictable temp path would redirect the save — tokens
/// and all — wherever it pointed. `create_new` refuses to open anything that
/// already exists, symlink included, and the name carries a UUID so two saves
/// cannot pick the same one. The file is born with owner-only permissions
/// instead of being tightened after the fact, so the contents are never briefly
/// world-readable.
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
                    // The mode above applies only at creation, and umask can
                    // still clear bits from it; set it explicitly as well.
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
    /// Loads (or initializes) the data file. Note the write side effect: the
    /// one-time self-entry seeding persists to apps.json, so constructing a
    /// Storage can mutate the real data file — anything building a Storage
    /// against a user's environment (tests, fixtures) inherits that.
    pub fn new() -> Result<Self> {
        let app_support = dirs::home_dir()
            .context("Failed to get home directory")?
            .join("Library")
            .join("Application Support")
            .join("Obtainintosh");

        prepare_storage_directory(&app_support)?;

        Self::load_from_path(app_support.join("apps.json"))
    }

    /// The half of `new` that does not depend on the user's home directory, so
    /// the load and recovery paths are testable against a temporary file.
    fn load_from_path(file_path: PathBuf) -> Result<Self> {
        // symlink_metadata rather than exists(): the check has to see the entry
        // itself, not what it might point at.
        let mut data = match fs::symlink_metadata(&file_path) {
            Ok(metadata) => {
                // Following a symlink here would write the user's tokens
                // wherever it pointed, and a rename onto it would replace the
                // link rather than its target. Neither is a state to guess at.
                if metadata.file_type().is_symlink() {
                    anyhow::bail!("Refusing to use apps.json because it is a symbolic link");
                }
                if !metadata.file_type().is_file() {
                    anyhow::bail!("Refusing to use apps.json because it is not a regular file");
                }

                // A file written by an earlier version predates these
                // permissions; tighten it on the way in.
                tighten_storage_file_permissions(&file_path)?;
                let contents = fs::read(&file_path).context("Failed to read apps.json")?;
                match serde_json::from_slice(&contents) {
                    Ok(data) => data,
                    // A file we cannot parse is not a reason to refuse to start
                    // — that would leave the user with an app that never opens
                    // again and no way to fix it from inside. Preserve the
                    // original bytes, then carry on from defaults.
                    Err(error) => {
                        let backup_path = backup_corrupt_file(&file_path).with_context(|| {
                            format!("Failed to preserve unreadable apps.json after: {error}")
                        })?;
                        log::error!(
                            "Could not load persisted data at {} ({error}). The original file was preserved at {}. Starting with defaults.",
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

    /// Writes a snapshot to disk. Takes the data by reference rather than
    /// reading `self.data`, so a caller can persist a *proposed* state before
    /// deciding to adopt it — see the mutation methods below.
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
        // Generate UUID if not provided
        if app.id.is_empty() {
            app.id = uuid::Uuid::new_v4().to_string();
        }

        let new_url = crate::sources::normalize_repo_url(&app.source_url);
        let mut data = self.data.lock().unwrap();
        if data
            .apps
            .iter()
            .any(|a| crate::sources::normalize_repo_url(&a.source_url) == new_url)
        {
            anyhow::bail!("This repository is already being tracked");
        }
        // Build the state we want, write it, and only then adopt it. Mutating
        // in place first would leave memory ahead of disk whenever the write
        // fails — the app would show a program it did not persist, and the
        // discrepancy would only surface on the next launch.
        let mut proposed = data.clone();
        proposed.apps.push(app.clone());

        self.persist(&proposed)?;
        *data = proposed;
        Ok(app)
    }

    pub fn update_app(&self, updated_app: App) -> Result<()> {
        let mut data = self.data.lock().unwrap();

        let Some(index) = data.apps.iter().position(|a| a.id == updated_app.id) else {
            anyhow::bail!("App not found: {}", updated_app.id);
        };

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

    /// Applies the outcome of a check that ran against `snapshot`, writing only
    /// the fields a check owns and only if the record still matches what was
    /// checked.
    pub fn apply_check_result(
        &self,
        snapshot: &App,
        update: CheckOwnedUpdate,
    ) -> Result<PendingResultApplication> {
        let mut data = self.data.lock().unwrap();
        let Some(index) = data.apps.iter().position(|app| app.id == snapshot.id) else {
            return Ok(PendingResultApplication::AppRemoved);
        };
        let current = &data.apps[index];
        if current.name != snapshot.name
            || !same_source_identity(current, snapshot)
            || !same_check_metadata(current, snapshot)
        {
            return Ok(PendingResultApplication::DependenciesChanged);
        }

        let mut proposed = data.clone();
        let current = &mut proposed.apps[index];
        current.current_version = update.current_version;
        current.install_path = update.install_path;
        // A failed check must not touch latest_version or last_checked: the
        // previously known version stays, flagged as stale by the attempt
        // rather than silently replaced with nothing.
        if update.attempt.state == CheckAttemptState::Succeeded {
            current.latest_version = update.latest_version;
            current.last_checked = Some(update.attempt.attempted_at.clone());
        }
        current.last_check_attempt = Some(update.attempt);

        self.persist(&proposed)?;
        *data = proposed;
        Ok(PendingResultApplication::Applied)
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

/// Whether two records still point at the same repository, compared the way
/// the dedupe in `add_app` does so the two cannot disagree.
fn same_source_identity(left: &App, right: &App) -> bool {
    left.source_type == right.source_type
        && crate::sources::normalize_repo_url(&left.source_url)
            == crate::sources::normalize_repo_url(&right.source_url)
}

/// Whether the check-owned fields are still as the snapshot left them. If they
/// moved, another check already landed and this result is the older one.
fn same_check_metadata(left: &App, right: &App) -> bool {
    left.latest_version == right.latest_version
        && left.last_checked == right.last_checked
        && left.last_check_attempt == right.last_check_attempt
}

/// Moves an unparseable data file aside and returns where it went, so the
/// caller can start from defaults without destroying whatever the user had.
///
/// The move is a hard link followed by an unlink rather than a rename: linking
/// fails with `AlreadyExists` instead of clobbering, which is what makes the
/// suffix search safe against an earlier backup. Repeated failures therefore
/// accumulate as `.corrupt-backup`, `.corrupt-backup.1`, and so on, rather than
/// the newest corruption overwriting the one copy that might still be
/// salvageable.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{App, SourceType};

    /// A scratch directory that cleans itself up, so the storage tests can
    /// exercise the real filesystem paths without touching the user's data.
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
            // Deliberately not unwrapped: a panic here during an already
            // failing test would replace the real assertion message.
            let _ = fs::remove_dir_all(&self.0);
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

    /// Every scratch file a save could have left behind next to `file_path`.
    fn storage_temp_paths(file_path: &Path) -> Vec<PathBuf> {
        let prefix = format!("{}.", file_path.file_name().unwrap().to_string_lossy());
        let mut paths = fs::read_dir(file_path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                (name.starts_with(&prefix) && name.ends_with(".tmp")).then_some(path)
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
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

    /// The regression this whole type exists for: the old save wrote to a
    /// fixed `apps.json.tmp` with `fs::write`, which follows symlinks.
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

    /// A storage whose file lives under a directory that does not exist, so
    /// every write fails. Lets the rollback tests drive the failure path
    /// without depending on filesystem permissions.
    fn failing_storage(data: AppData) -> (Storage, TestDir) {
        let temp_dir = TestDir::new();
        let storage = Storage {
            file_path: temp_dir.path().join("missing").join("apps.json"),
            data: Mutex::new(data),
        };
        (storage, temp_dir)
    }

    fn test_app(id: &str, source_url: &str) -> App {
        App {
            id: id.to_string(),
            name: id.to_string(),
            source_type: SourceType::GitHub,
            source_url: source_url.to_string(),
            current_version: None,
            latest_version: None,
            install_path: None,
            last_checked: None,
            last_check_attempt: None,
            username: None,
            access_token: None,
        }
    }

    #[test]
    fn failed_add_does_not_change_memory() {
        let (storage, _temp_dir) = failing_storage(AppData::default());

        let error = storage
            .add_app(test_app("new", "https://github.com/owner/new"))
            .unwrap_err()
            .to_string();

        assert_eq!(error, "Failed to write temp file");
        assert!(storage.get_all_apps().unwrap().is_empty());
    }

    #[test]
    fn failed_update_does_not_change_memory() {
        let mut data = AppData::default();
        data.apps
            .push(test_app("existing", "https://github.com/owner/existing"));
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
        data.apps
            .push(test_app("existing", "https://github.com/owner/existing"));
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
        assert_eq!(legacy.access_token, None);
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
        // The file is tightened before it is read, so the hard link that
        // preserves it inherits owner-only permissions rather than the 0666 it
        // was found with.
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
        // The recovered file has to be loadable in its own right, or the next
        // launch backs it up all over again.
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
    fn loads_a_data_file_written_before_forgejo_support() {
        // The credential fields were added to `App` after these files were
        // written, so an upgrade must not fail to parse its own apps.json.
        let stored = r#"{
            "apps": [
                {
                    "id": "abc",
                    "name": "Obtainintosh",
                    "source_type": "github",
                    "source_url": "https://github.com/L-K-M/Obtainintosh",
                    "current_version": "1.2.1",
                    "latest_version": null,
                    "install_path": null,
                    "last_checked": null
                }
            ],
            "settings": {"github_token": null, "gitlab_token": null},
            "self_entry_seeded": true
        }"#;

        let data: AppData = serde_json::from_str(stored).unwrap();
        assert_eq!(data.apps.len(), 1);
        assert!(data.apps[0].username.is_none());
        assert!(data.apps[0].access_token.is_none());
    }

    #[test]
    fn app_debug_output_redacts_the_application_key() {
        let mut app = crate::updates::self_app_entry();
        app.username = Some("alice".to_string());
        app.access_token = Some("key123".to_string());

        let rendered = format!("{:?}", app);
        assert!(!rendered.contains("key123"), "leaked the key: {rendered}");
        assert!(rendered.contains(crate::models::REDACTED), "{rendered}");
        assert!(rendered.contains("alice"), "{rendered}");
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
        // Same repo, different spellings: matching is case-insensitive and
        // ignores a trailing slash or `.git`, like Storage::add_app's dedupe.
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
                last_check_attempt: None,
                username: None,
                access_token: None,
            });
            assert!(seed_self_entry(&mut data)); // still marks the file as seeded
            assert_eq!(data.apps.len(), 1, "duplicated for {}", source_url);
            assert_eq!(data.apps[0].id, "existing");
            assert!(data.self_entry_seeded);
        }
    }

    fn succeeded_update(latest: &str) -> CheckOwnedUpdate {
        CheckOwnedUpdate {
            current_version: Some("1.0.0".to_string()),
            install_path: Some("/Applications/Existing.app".to_string()),
            latest_version: Some(latest.to_string()),
            attempt: CheckAttempt::succeeded("2026-08-06T00:00:00Z".to_string()),
        }
    }

    fn failed_update() -> CheckOwnedUpdate {
        CheckOwnedUpdate {
            current_version: Some("1.0.0".to_string()),
            install_path: Some("/Applications/Existing.app".to_string()),
            latest_version: None,
            attempt: CheckAttempt::unsuccessful(
                "2026-08-06T00:00:00Z".to_string(),
                CheckAttemptState::Failed,
                "the network went away",
            ),
        }
    }

    fn storage_with_one_app() -> (Storage, TestDir, App) {
        let temp_dir = TestDir::new();
        let app = test_app("existing", "https://github.com/owner/existing");
        let mut data = AppData::default();
        data.apps.push(app.clone());
        let storage = Storage {
            file_path: temp_dir.path().join("apps.json"),
            data: Mutex::new(data),
        };
        (storage, temp_dir, app)
    }

    #[test]
    fn a_successful_check_records_its_version_and_timestamp() {
        let (storage, _temp, snapshot) = storage_with_one_app();

        let applied = storage
            .apply_check_result(&snapshot, succeeded_update("2.0.0"))
            .unwrap();

        assert_eq!(applied, PendingResultApplication::Applied);
        let stored = storage.get_app("existing").unwrap().unwrap();
        assert_eq!(stored.latest_version.as_deref(), Some("2.0.0"));
        assert_eq!(stored.last_checked.as_deref(), Some("2026-08-06T00:00:00Z"));
        assert_eq!(
            stored.last_check_attempt.unwrap().state,
            CheckAttemptState::Succeeded
        );
    }

    #[test]
    fn a_failed_check_keeps_the_last_known_version_and_marks_it_stale() {
        let (storage, _temp, snapshot) = storage_with_one_app();
        storage
            .apply_check_result(&snapshot, succeeded_update("2.0.0"))
            .unwrap();
        let snapshot = storage.get_app("existing").unwrap().unwrap();

        storage
            .apply_check_result(&snapshot, failed_update())
            .unwrap();

        let stored = storage.get_app("existing").unwrap().unwrap();
        // The figure the user can still act on survives; only the attempt
        // records that it is no longer fresh.
        assert_eq!(stored.latest_version.as_deref(), Some("2.0.0"));
        assert_eq!(stored.last_checked.as_deref(), Some("2026-08-06T00:00:00Z"));
        let attempt = stored.last_check_attempt.unwrap();
        assert_eq!(attempt.state, CheckAttemptState::Failed);
        assert_eq!(attempt.message.as_deref(), Some("the network went away"));
    }

    #[test]
    fn a_result_for_a_removed_app_is_not_written_back() {
        let (storage, _temp, snapshot) = storage_with_one_app();
        storage.remove_app("existing").unwrap();

        let applied = storage
            .apply_check_result(&snapshot, succeeded_update("2.0.0"))
            .unwrap();

        assert_eq!(applied, PendingResultApplication::AppRemoved);
        assert!(storage.get_app("existing").unwrap().is_none());
    }

    #[test]
    fn a_result_does_not_overwrite_an_edit_made_while_it_ran() {
        let (storage, _temp, snapshot) = storage_with_one_app();
        let mut renamed = snapshot.clone();
        renamed.name = "Renamed".to_string();
        storage.update_app(renamed).unwrap();

        let applied = storage
            .apply_check_result(&snapshot, succeeded_update("2.0.0"))
            .unwrap();

        assert_eq!(applied, PendingResultApplication::DependenciesChanged);
        let stored = storage.get_app("existing").unwrap().unwrap();
        assert_eq!(stored.name, "Renamed");
        assert_eq!(stored.latest_version, None);
    }

    #[test]
    fn an_older_result_loses_to_one_that_already_landed() {
        let (storage, _temp, snapshot) = storage_with_one_app();
        storage
            .apply_check_result(&snapshot, succeeded_update("3.0.0"))
            .unwrap();

        // The stale snapshot still has no latest_version, so its result is
        // recognised as the older one and discarded.
        let applied = storage
            .apply_check_result(&snapshot, succeeded_update("2.0.0"))
            .unwrap();

        assert_eq!(applied, PendingResultApplication::DependenciesChanged);
        assert_eq!(
            storage
                .get_app("existing")
                .unwrap()
                .unwrap()
                .latest_version
                .as_deref(),
            Some("3.0.0")
        );
    }
}
