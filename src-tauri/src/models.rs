use serde::{Deserialize, Serialize};

/// Stand-in printed in place of a secret by the hand-written `Debug` impls
/// below, so a stray `{:?}` can't put an application key in the log.
pub(crate) const REDACTED: &str = "[redacted]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    GitHub,
    GitLab,
    Forgejo,
}

/// `Debug` is hand-written below rather than derived, so that the application
/// key is redacted.
#[derive(Clone, Serialize, Deserialize)]
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

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("source_type", &self.source_type)
            .field("source_url", &self.source_url)
            .field("current_version", &self.current_version)
            .field("latest_version", &self.latest_version)
            .field("install_path", &self.install_path)
            .field("last_checked", &self.last_checked)
            .field("username", &self.username)
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| REDACTED),
            )
            .finish()
    }
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
