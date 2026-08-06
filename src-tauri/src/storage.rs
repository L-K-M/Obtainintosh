use crate::models::{App, AppData, Settings};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct Storage {
    file_path: PathBuf,
    data: Mutex<AppData>,
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

        // Create directory if it doesn't exist
        fs::create_dir_all(&app_support)
            .context("Failed to create application support directory")?;

        Self::load_from_path(app_support.join("apps.json"))
    }

    /// The half of `new` that does not depend on the user's home directory, so
    /// the load and recovery paths are testable against a temporary file.
    fn load_from_path(file_path: PathBuf) -> Result<Self> {
        // Load existing data or create new
        let mut data = if file_path.exists() {
            let contents = fs::read(&file_path).context("Failed to read apps.json")?;
            match serde_json::from_slice(&contents) {
                Ok(data) => data,
                // A file we cannot parse is not a reason to refuse to start —
                // that would leave the user with an app that never opens again
                // and no way to fix it from inside. Preserve the original
                // bytes, then carry on from defaults.
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
        } else {
            AppData::default()
        };

        let seeded = seed_self_entry(&mut data);

        let storage = Self {
            file_path,
            data: Mutex::new(data),
        };
        if seeded {
            storage
                .save()
                .context("Failed to save the seeded self-entry")?;
        }
        Ok(storage)
    }

    fn save(&self) -> Result<()> {
        let data = self.data.lock().unwrap();
        let json = serde_json::to_string_pretty(&*data).context("Failed to serialize data")?;

        // Atomic write: write to temp file then rename
        let temp_path = self.file_path.with_extension("json.tmp");
        fs::write(&temp_path, json).context("Failed to write temp file")?;
        fs::rename(&temp_path, &self.file_path).context("Failed to rename temp file")?;

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
        data.apps.push(app.clone());
        drop(data);

        self.save()?;
        Ok(app)
    }

    pub fn update_app(&self, updated_app: App) -> Result<()> {
        let mut data = self.data.lock().unwrap();

        if let Some(app) = data.apps.iter_mut().find(|a| a.id == updated_app.id) {
            *app = updated_app;
        } else {
            anyhow::bail!("App not found: {}", updated_app.id);
        }

        drop(data);
        self.save()
    }

    pub fn remove_app(&self, id: &str) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        data.apps.retain(|app| app.id != id);
        drop(data);

        self.save()
    }

    pub fn get_settings(&self) -> Result<Settings> {
        let data = self.data.lock().unwrap();
        Ok(data.settings.clone())
    }

    pub fn update_settings(&self, settings: Settings) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        data.settings = settings;
        drop(data);

        self.save()
    }
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

        let storage = Storage::load_from_path(file_path.clone()).unwrap();

        let backup_path = temp_dir.path().join("apps.json.corrupt-backup");
        assert_eq!(fs::read(backup_path).unwrap(), malformed);
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
                username: None,
                access_token: None,
            });
            assert!(seed_self_entry(&mut data)); // still marks the file as seeded
            assert_eq!(data.apps.len(), 1, "duplicated for {}", source_url);
            assert_eq!(data.apps[0].id, "existing");
            assert!(data.self_entry_seeded);
        }
    }
}
