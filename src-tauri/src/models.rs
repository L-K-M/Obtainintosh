use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    GitHub,
    GitLab,
    Forgejo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub source_type: SourceType,
    pub source_url: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub install_path: Option<String>,
    pub last_checked: Option<String>, // ISO 8601 timestamp
    /// Credentials for a forge instance that needs them — currently only
    /// Forgejo, where a private instance rejects anonymous API reads. Stored
    /// per app rather than globally because every self-hosted instance issues
    /// its own application key. `#[serde(default)]` keeps data files written
    /// before these fields existed loadable.
    #[serde(default)]
    pub username: Option<String>,
    /// Forgejo calls personal access tokens "applications" (Settings →
    /// Applications → Generate New Token); the UI asks for an "application
    /// key" to match that wording.
    #[serde(default)]
    pub access_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub download_url: String,
    pub file_name: String,
    pub file_size: Option<u64>,
    pub checksum: Option<String>,
    pub release_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub github_token: Option<String>,
    pub gitlab_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemColors {
    pub accent_color: Option<String>,
    pub accent_text_color: Option<String>,
    pub highlight_color: Option<String>,
    pub highlight_text_color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppData {
    pub apps: Vec<App>,
    pub settings: Settings,
    /// One-shot marker: Obtainintosh's own entry has been seeded into this data
    /// file. Once set, a user who removes the entry keeps it removed — it is
    /// never re-added on later launches.
    #[serde(default)]
    pub self_entry_seeded: bool,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            apps: Vec::new(),
            settings: Settings {
                github_token: None,
                gitlab_token: None,
            },
            self_entry_seeded: false,
        }
    }
}
