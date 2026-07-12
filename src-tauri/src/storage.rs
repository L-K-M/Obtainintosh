use crate::models::{App, AppData, Settings};
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
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
    pub fn new() -> Result<Self> {
        let app_support = dirs::home_dir()
            .context("Failed to get home directory")?
            .join("Library")
            .join("Application Support")
            .join("Obtainintosh");

        // Create directory if it doesn't exist
        fs::create_dir_all(&app_support)
            .context("Failed to create application support directory")?;

        let file_path = app_support.join("apps.json");

        // Load existing data or create new
        let mut data = if file_path.exists() {
            let contents = fs::read_to_string(&file_path).context("Failed to read apps.json")?;
            serde_json::from_str(&contents).context("Failed to parse apps.json")?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{App, SourceType};

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
            });
            assert!(seed_self_entry(&mut data)); // still marks the file as seeded
            assert_eq!(data.apps.len(), 1, "duplicated for {}", source_url);
            assert_eq!(data.apps[0].id, "existing");
            assert!(data.self_entry_seeded);
        }
    }
}
