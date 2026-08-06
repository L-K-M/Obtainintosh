use serde::{Deserialize, Serialize};

/// Stand-in printed in place of a secret by the hand-written `Debug` impls
/// below, so a stray `{:?}` can't put an application key in the log.
pub(crate) const REDACTED: &str = "[redacted]";

/// How the last update check for an app turned out. Stored on the app so the
/// UI can tell "checked, up to date" apart from "the check never completed" —
/// which a bare `latest_version` cannot express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckAttemptState {
    Succeeded,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckAttempt {
    pub attempted_at: String,
    pub state: CheckAttemptState,
    #[serde(default)]
    pub message: Option<String>,
}

impl CheckAttempt {
    pub fn succeeded(attempted_at: String) -> Self {
        Self {
            attempted_at,
            state: CheckAttemptState::Succeeded,
            message: None,
        }
    }

    pub fn unsuccessful(attempted_at: String, state: CheckAttemptState, message: &str) -> Self {
        debug_assert!(state != CheckAttemptState::Succeeded);
        Self {
            attempted_at,
            state,
            message: Some(bounded_check_message(message)),
        }
    }
}

/// Collapses whitespace and caps length. These messages come from network and
/// forge errors, go into the data file, and are rendered in a tooltip — none of
/// which wants an unbounded multi-line string.
pub fn bounded_check_message(message: &str) -> String {
    const MAX_CHARS: usize = 240;
    const ELLIPSIS: &str = "...";

    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }

    let mut bounded = normalized
        .chars()
        .take(MAX_CHARS - ELLIPSIS.len())
        .collect::<String>();
    bounded.push_str(ELLIPSIS);
    bounded
}

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
    /// Everything below is state Obtainintosh recomputes — an installed
    /// version it re-detects, a release it re-fetches. Defaulting them keeps a
    /// data file written by an older version loadable instead of sending it
    /// down the corrupt-file recovery path over a field that was simply absent.
    /// The identity fields above stay required: a record without them is not a
    /// tracked program, and silently defaulting one would invent an entry.
    #[serde(default)]
    pub current_version: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub install_path: Option<String>,
    #[serde(default)]
    pub last_checked: Option<String>, // ISO 8601 timestamp
    /// The most recent check attempt, successful or not. `last_checked` only
    /// moves on success, so this is what distinguishes a stale
    /// `latest_version` from a current one.
    #[serde(default)]
    pub last_check_attempt: Option<CheckAttempt>,
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
            .field("last_check_attempt", &self.last_check_attempt)
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub gitlab_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemColors {
    pub accent_color: Option<String>,
    pub accent_text_color: Option<String>,
    pub highlight_color: Option<String>,
    pub highlight_text_color: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppData {
    pub apps: Vec<App>,
    #[serde(default)]
    pub settings: Settings,
    /// One-shot marker: Obtainintosh's own entry has been seeded into this data
    /// file. Once set, a user who removes the entry keeps it removed — it is
    /// never re-added on later launches.
    #[serde(default)]
    pub self_entry_seeded: bool,
}
