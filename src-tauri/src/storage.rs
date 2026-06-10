use crate::models::{App, AppData, Settings};
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Storage {
    file_path: PathBuf,
    data: Mutex<AppData>,
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
        let data = if file_path.exists() {
            let contents = fs::read_to_string(&file_path)
                .context("Failed to read apps.json")?;
            serde_json::from_str(&contents)
                .context("Failed to parse apps.json")?
        } else {
            AppData::default()
        };

        Ok(Self {
            file_path,
            data: Mutex::new(data),
        })
    }

    fn save(&self) -> Result<()> {
        let data = self.data.lock().unwrap();
        let json = serde_json::to_string_pretty(&*data)
            .context("Failed to serialize data")?;
        
        // Atomic write: write to temp file then rename
        let temp_path = self.file_path.with_extension("json.tmp");
        fs::write(&temp_path, json)
            .context("Failed to write temp file")?;
        fs::rename(&temp_path, &self.file_path)
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

        let mut data = self.data.lock().unwrap();
        if data.apps.iter().any(|a| {
            a.source_url.trim_end_matches('/').eq_ignore_ascii_case(app.source_url.trim_end_matches('/'))
        }) {
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
