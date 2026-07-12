use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    GitHub,
    GitLab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub source_type: SourceType,
    pub source_url: String,
    #[serde(default)]
    pub current_version: Option<String>,
    #[serde(default)]
    pub latest_version: Option<String>,
    #[serde(default)]
    pub install_path: Option<String>,
    #[serde(default)]
    pub last_checked: Option<String>, // ISO 8601 timestamp
    #[serde(default)]
    pub last_check_attempt: Option<CheckAttempt>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_check_messages_are_single_line_and_bounded() {
        let input = format!("request\nfailed\t{}", "x".repeat(300));
        let message = bounded_check_message(&input);

        assert_eq!(message.chars().count(), 240);
        assert!(!message.contains(['\n', '\r', '\t']));
        assert!(message.ends_with("..."));
    }
}
