//! Export and import of the tracked-program list as a JSON file.
//!
//! The file carries what identifies each program — its name, forge,
//! repository URL, and the Forgejo username where there is one — and none of
//! the state Obtainintosh recomputes (installed and latest versions, check
//! timestamps, cached downloads). Application keys are never written: the
//! file is meant to travel between machines and to be handed to other people,
//! and a secret inside it would leak the moment it is shared.
//!
//! An import merges into the existing list. It adds the programs that are not
//! tracked yet and leaves everything else alone; nothing is ever removed or
//! overwritten by an import.

use crate::models::{App, SourceType};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The extension an exported list carries, so the file is recognisably
/// Obtainintosh's own rather than an anonymous `.json` — and so the open
/// dialog can filter for it.
pub const FILE_EXTENSION: &str = "obtainintosh";

/// How the file filter is labelled in the native dialogs.
pub const FILE_TYPE_LABEL: &str = "Obtainintosh program list";

/// Marks a JSON document as an Obtainintosh program list, so the import can
/// tell a stray JSON file apart from one this module wrote.
const FORMAT: &str = "obtainintosh-app-list";

/// The format version this build writes, and the newest it reads. Bumped only
/// when a change would make an older build misread the file; adding fields an
/// older build ignores is not such a change.
const FORMAT_VERSION: u32 = 1;

/// Files bigger than this are refused before they are read. A list of
/// thousands of programs is well under a megabyte, so anything near the cap is
/// not a program list — a mis-picked disk image, say — and reading it whole
/// only to fail JSON parsing would spend memory for nothing.
pub const MAX_FILE_SIZE: u64 = 16 * 1024 * 1024;

/// What the file records about one program: the fields the Add Program dialog
/// collects, and nothing derived from them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedApp {
    pub name: String,
    /// Always written by the export. On import it may be left out to have
    /// the forge detected from the URL, as the dialog's "Detect
    /// automatically" does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<SourceType>,
    pub source_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Never written — see the module docs — but accepted on import, so a
    /// hand-edited file can restore a private instance in one go. Honoured
    /// only for a forge that authenticates with it, like the dialog.
    #[serde(default, skip_serializing)]
    pub access_token: Option<String>,
}

impl ExportedApp {
    fn from_app(app: &App) -> Self {
        Self {
            name: app.name.clone(),
            source_type: Some(app.source_type),
            source_url: app.source_url.clone(),
            username: app.username.clone(),
            access_token: None,
        }
    }
}

/// The document as written. Reading goes through [`Header`] first so a file
/// that is not a program list at all gets a plain answer rather than a
/// missing-field error naming an internal marker.
#[derive(Debug, Serialize, Deserialize)]
struct AppListFile {
    format: String,
    format_version: u32,
    #[serde(default)]
    exported_at: String,
    #[serde(default)]
    exported_by: String,
    apps: Vec<ExportedApp>,
}

/// The lenient first pass over an import: only the marker and the version,
/// each optional so their absence can be reported in words.
struct Header {
    format: Option<String>,
    format_version: Option<u32>,
}

impl Header {
    /// Reads the marker fields off any JSON value, each on its own, so a
    /// version written as `"1"` is reported as a version problem rather
    /// than hiding the marker that sits next to it. A document that is not
    /// an object at all — an array, a number — is simply a document without
    /// either, and gets the same "not a program list" answer as an object
    /// that lacks them.
    fn of(document: &serde_json::Value) -> Self {
        Self {
            format: document
                .get("format")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            format_version: document
                .get("format_version")
                .and_then(serde_json::Value::as_u64)
                .and_then(|version| u32::try_from(version).ok()),
        }
    }
}

/// Serializes the tracked list into the file contents. Done before any dialog
/// is shown, so a failure here never asks the user for a path first.
pub fn render(apps: &[App]) -> Result<String> {
    let file = AppListFile {
        format: FORMAT.to_string(),
        format_version: FORMAT_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339(),
        exported_by: crate::sources::USER_AGENT.to_string(),
        apps: apps.iter().map(ExportedApp::from_app).collect(),
    };
    let mut json =
        serde_json::to_string_pretty(&file).context("Failed to serialize the program list")?;
    json.push('\n');
    Ok(json)
}

/// Parses file contents into the entries they list, refusing anything that is
/// not a program list this build understands.
pub fn parse(contents: &str) -> Result<Vec<ExportedApp>> {
    // Some editors save UTF-8 with a byte-order mark, which serde_json does
    // not skip; a hand-edited list should not fail as "not valid JSON" over
    // an invisible first character.
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let document: serde_json::Value = serde_json::from_str(contents)
        .map_err(|error| anyhow::anyhow!("The file is not valid JSON: {error}"))?;
    let header = Header::of(&document);
    if header.format.as_deref() != Some(FORMAT) {
        anyhow::bail!("The file is not an Obtainintosh program list");
    }
    let version = header
        .format_version
        .context("The file does not say which program list format it uses")?;
    if version > FORMAT_VERSION {
        anyhow::bail!(
            "The file was written by a newer version of Obtainintosh (program list format \
             {version}; this version reads format {FORMAT_VERSION} and older). Update \
             Obtainintosh to import it."
        );
    }
    // Structural problems in an entry fail the whole file, with serde's line
    // and column pointing at the spot — for a hand-edited file that is more
    // useful than importing around the entry and reporting it by position.
    let file: AppListFile = serde_json::from_str(contents)
        .map_err(|error| anyhow::anyhow!("The program list could not be read: {error}"))?;
    Ok(file.apps)
}

/// An entry that could not become a tracked program, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RejectedEntry {
    /// The entry's name, or its position in the file when it has none.
    pub label: String,
    pub reason: String,
}

/// The entries of a file sorted into programs ready to be added (without
/// ids; storage assigns them) and entries that were turned away.
#[derive(Debug, Default)]
pub struct ImportPlan {
    pub apps: Vec<App>,
    pub rejected: Vec<RejectedEntry>,
}

/// Applies the Add Program dialog's rules to each entry: a name and a URL are
/// required, the forge is detected from the URL when the entry does not name
/// one, and credentials are kept only for a forge that uses them. Duplicates
/// are not decided here — that happens under the storage lock, where the
/// answer cannot go stale.
pub fn plan_import(entries: Vec<ExportedApp>) -> ImportPlan {
    let mut plan = ImportPlan::default();
    for (index, entry) in entries.into_iter().enumerate() {
        let name = entry.name.trim().to_string();
        let label = if name.is_empty() {
            format!("entry {}", index + 1)
        } else {
            bounded_excerpt(&name)
        };
        match app_from_entry(name, entry) {
            Ok(app) => plan.apps.push(app),
            Err(reason) => plan.rejected.push(RejectedEntry { label, reason }),
        }
    }
    plan
}

/// Collapses whitespace and caps the length of a value quoted back at the
/// user from the file — an entry's name, or its URL inside a reason. Those
/// come straight from a file that may have been shared around, and the
/// notification they end up in must not grow with them.
fn bounded_excerpt(text: &str) -> String {
    const MAX_CHARS: usize = 60;
    const ELLIPSIS: char = '…';

    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }
    let mut bounded: String = normalized.chars().take(MAX_CHARS - 1).collect();
    bounded.push(ELLIPSIS);
    bounded
}

fn app_from_entry(name: String, entry: ExportedApp) -> Result<App, String> {
    if name.is_empty() {
        return Err("The program name is empty".to_string());
    }
    let source_url = entry.source_url.trim().to_string();
    if source_url.is_empty() {
        return Err("The repository URL is empty".to_string());
    }
    let source_type = entry
        .source_type
        .or_else(|| crate::sources::detect_source_type(&source_url))
        .ok_or_else(|| {
            format!(
                "Unsupported source URL. Add \"source_type\": \"forgejo\" to the entry if {} \
                 is a Forgejo instance.",
                bounded_excerpt(&source_url)
            )
        })?;
    let credentials =
        crate::commands::credentials_for(source_type, entry.username, entry.access_token);

    Ok(App {
        id: String::new(),
        name,
        source_type,
        source_url,
        current_version: None,
        latest_version: None,
        install_path: None,
        last_checked: None,
        last_check_attempt: None,
        downloaded: None,
        username: credentials.username().map(str::to_string),
        access_token: credentials.token().map(str::to_string),
    })
}

/// A Forgejo program that arrived with a username but no application key —
/// the shape every private-instance entry has after a round trip through a
/// file, since the key is never written. Worth telling the user about, or
/// their next update check just fails with a credentials error.
pub fn is_missing_application_key(app: &App) -> bool {
    app.source_type == SourceType::Forgejo && app.username.is_some() && app.access_token.is_none()
}

/// The name the save dialog proposes: dated, so successive exports sit side
/// by side instead of prompting to replace each other.
pub fn suggested_file_name(today: chrono::NaiveDate) -> String {
    format!(
        "Obtainintosh Programs {}.{FILE_EXTENSION}",
        today.format("%Y-%m-%d")
    )
}

/// Gives a chosen save path the list's extension when it has none. The macOS
/// save panel appends it itself; the GTK one saves exactly what was typed. An
/// extension the user typed deliberately (`list.json`) is left alone.
pub fn with_default_extension(path: PathBuf) -> PathBuf {
    // `extension()` is `Some("")` for a name that ends in a dot, which is
    // no extension either.
    if path
        .extension()
        .is_some_and(|extension| !extension.is_empty())
    {
        return path;
    }
    let Some(name) = path.file_name() else {
        return path;
    };
    let mut name = name.to_os_string();
    // Drop that trailing dot, so `list.` becomes `list.obtainintosh`
    // rather than `list..obtainintosh`.
    if let Some(text) = name.to_str() {
        let trimmed = text.trim_end_matches('.');
        if trimmed.is_empty() {
            return path;
        }
        name = trimmed.into();
    }
    name.push(".");
    name.push(FILE_EXTENSION);
    path.with_file_name(name)
}

/// The file name shown in messages: the final component, or the whole path
/// when there is none to speak of.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, source_type: SourceType, url: &str) -> App {
        App {
            id: format!("id-{name}"),
            name: name.to_string(),
            source_type,
            source_url: url.to_string(),
            current_version: Some("1.0".to_string()),
            latest_version: Some("1.1".to_string()),
            install_path: Some("/Applications/App.app".to_string()),
            last_checked: Some("2026-09-05T00:00:00Z".to_string()),
            last_check_attempt: None,
            downloaded: Some(crate::models::DownloadedRelease {
                version: "1.1".to_string(),
                path: "/tmp/App.dmg".to_string(),
            }),
            username: None,
            access_token: None,
        }
    }

    fn entry(name: &str, url: &str) -> ExportedApp {
        ExportedApp {
            name: name.to_string(),
            source_type: None,
            source_url: url.to_string(),
            username: None,
            access_token: None,
        }
    }

    #[test]
    fn export_writes_identity_only_and_never_the_application_key() {
        let mut private = app(
            "Private",
            SourceType::Forgejo,
            "https://git.example.internal/owner/private",
        );
        private.username = Some("alice".to_string());
        private.access_token = Some("secret-key".to_string());
        let public = app(
            "Public",
            SourceType::GitHub,
            "https://github.com/owner/public",
        );

        let json = render(&[private, public]).unwrap();

        assert!(!json.contains("secret-key"), "{json}");
        assert!(!json.contains("access_token"), "{json}");
        let document: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(document["format"], FORMAT);
        assert_eq!(document["format_version"], FORMAT_VERSION);
        assert_eq!(document["exported_by"], crate::sources::USER_AGENT);
        let apps = document["apps"].as_array().unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(
            apps[0],
            serde_json::json!({
                "name": "Private",
                "source_type": "forgejo",
                "source_url": "https://git.example.internal/owner/private",
                "username": "alice",
            })
        );
        // Derived state stays out: nothing but the dialog's fields.
        assert_eq!(
            apps[1],
            serde_json::json!({
                "name": "Public",
                "source_type": "github",
                "source_url": "https://github.com/owner/public",
            })
        );
        assert!(json.ends_with('\n'));
    }

    #[test]
    fn a_rendered_list_parses_back_to_the_same_entries() {
        let mut private = app(
            "Private",
            SourceType::Forgejo,
            "https://git.example.internal/owner/private",
        );
        private.username = Some("alice".to_string());
        private.access_token = Some("secret-key".to_string());
        let public = app(
            "Public",
            SourceType::GitHub,
            "https://github.com/owner/public",
        );

        let entries = parse(&render(&[private, public]).unwrap()).unwrap();

        assert_eq!(
            entries,
            vec![
                ExportedApp {
                    name: "Private".to_string(),
                    source_type: Some(SourceType::Forgejo),
                    source_url: "https://git.example.internal/owner/private".to_string(),
                    username: Some("alice".to_string()),
                    access_token: None,
                },
                ExportedApp {
                    name: "Public".to_string(),
                    source_type: Some(SourceType::GitHub),
                    source_url: "https://github.com/owner/public".to_string(),
                    username: None,
                    access_token: None,
                },
            ]
        );
    }

    #[test]
    fn parse_refuses_what_is_not_a_program_list() {
        let not_json = parse("{ not json").unwrap_err().to_string();
        assert!(
            not_json.starts_with("The file is not valid JSON:"),
            "{not_json}"
        );

        // A raw apps.json, or any JSON without the marker, is turned away in
        // words rather than with a missing-field error.
        for document in [
            r#"{"apps": [], "settings": {}}"#,
            r#"{"format": "something-else", "format_version": 1, "apps": []}"#,
            "[]",
            "42",
        ] {
            assert_eq!(
                parse(document).unwrap_err().to_string(),
                "The file is not an Obtainintosh program list",
                "{document}"
            );
        }

        // A missing version and a mistyped one are both version problems —
        // the marker next to them is fine and must not be blamed.
        for document in [
            r#"{"format": "obtainintosh-app-list", "apps": []}"#,
            r#"{"format": "obtainintosh-app-list", "format_version": "1", "apps": []}"#,
            r#"{"format": "obtainintosh-app-list", "format_version": 1.5, "apps": []}"#,
        ] {
            assert_eq!(
                parse(document).unwrap_err().to_string(),
                "The file does not say which program list format it uses",
                "{document}"
            );
        }

        let newer =
            parse(r#"{"format": "obtainintosh-app-list", "format_version": 2, "apps": []}"#)
                .unwrap_err()
                .to_string();
        assert!(newer.contains("newer version of Obtainintosh"), "{newer}");
        assert!(newer.contains("format 2"), "{newer}");

        let no_apps = parse(r#"{"format": "obtainintosh-app-list", "format_version": 1}"#)
            .unwrap_err()
            .to_string();
        assert!(no_apps.contains("missing field `apps`"), "{no_apps}");
    }

    #[test]
    fn parse_reports_a_malformed_entry_by_position_in_the_text() {
        let document = "{\n  \"format\": \"obtainintosh-app-list\",\n  \"format_version\": 1,\n  \
                        \"apps\": [\n    {\"name\": \"A\", \"source_type\": \"gihub\", \
                        \"source_url\": \"https://github.com/o/a\"}\n  ]\n}";

        let error = parse(document).unwrap_err().to_string();

        assert!(error.contains("unknown variant `gihub`"), "{error}");
        assert!(error.contains("line 5"), "{error}");
    }

    #[test]
    fn parse_skips_a_leading_byte_order_mark() {
        let document = r#"{"format": "obtainintosh-app-list", "format_version": 1, "apps": []}"#;
        // Without the strip, serde_json rejects the mark as "expected value".
        assert!(serde_json::from_str::<serde_json::Value>(&format!("\u{feff}{document}")).is_err());

        assert!(parse(&format!("\u{feff}{document}")).unwrap().is_empty());
    }

    #[test]
    fn parse_accepts_minimal_hand_written_entries_and_a_supplied_key() {
        let document = r#"{
            "format": "obtainintosh-app-list",
            "format_version": 1,
            "apps": [
                {"name": "A", "source_url": "https://github.com/o/a"},
                {"name": "B", "source_type": "forgejo", "source_url": "https://git.example.internal/o/b",
                 "username": "bob", "access_token": "key-b", "unknown_field": true}
            ]
        }"#;

        let entries = parse(document).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source_type, None);
        assert_eq!(entries[1].access_token.as_deref(), Some("key-b"));
    }

    #[test]
    fn plan_import_applies_the_add_dialog_rules() {
        let mut forgejo_with_key = entry("Private", "https://git.example.internal/o/private");
        forgejo_with_key.source_type = Some(SourceType::Forgejo);
        forgejo_with_key.username = Some("alice".to_string());
        forgejo_with_key.access_token = Some("key".to_string());
        let mut github_with_stray_key = entry("Public", "https://github.com/o/public");
        github_with_stray_key.username = Some("alice".to_string());
        github_with_stray_key.access_token = Some("key".to_string());
        let mut forgejo_with_padded_credentials =
            entry("Padded key", "https://codeberg.org/o/padded-key");
        forgejo_with_padded_credentials.username = Some("  bob  ".to_string());
        forgejo_with_padded_credentials.access_token = Some("   ".to_string());

        let plan = plan_import(vec![
            entry("  Padded  ", "  https://github.com/o/padded  "),
            entry("", "https://github.com/o/nameless"),
            entry("No URL", "   "),
            entry("Unknown host", "https://git.example.internal/o/unknown"),
            entry("Codeberg", "https://codeberg.org/o/detected"),
            forgejo_with_key,
            github_with_stray_key,
            forgejo_with_padded_credentials,
        ]);

        let names: Vec<&str> = plan.apps.iter().map(|app| app.name.as_str()).collect();
        assert_eq!(
            names,
            ["Padded", "Codeberg", "Private", "Public", "Padded key"]
        );
        assert!(plan.apps.iter().all(|app| app.id.is_empty()));

        // Credentials are cleaned the way the dialog's are: trimmed, and
        // dropped when nothing is left — so a padded username does not fail
        // the first check with an invisible mismatch.
        let padded_key = &plan.apps[4];
        assert_eq!(padded_key.username.as_deref(), Some("bob"));
        assert_eq!(padded_key.access_token, None);

        let padded = &plan.apps[0];
        assert_eq!(padded.source_url, "https://github.com/o/padded");
        assert_eq!(padded.source_type, SourceType::GitHub);
        assert_eq!(plan.apps[1].source_type, SourceType::Forgejo);

        let private = &plan.apps[2];
        assert_eq!(private.username.as_deref(), Some("alice"));
        assert_eq!(private.access_token.as_deref(), Some("key"));
        // Credentials against a forge that never sends them are dropped, as
        // the dialog drops them.
        let public = &plan.apps[3];
        assert_eq!(public.username, None);
        assert_eq!(public.access_token, None);

        assert_eq!(
            plan.rejected,
            vec![
                RejectedEntry {
                    label: "entry 2".to_string(),
                    reason: "The program name is empty".to_string(),
                },
                RejectedEntry {
                    label: "No URL".to_string(),
                    reason: "The repository URL is empty".to_string(),
                },
                RejectedEntry {
                    label: "Unknown host".to_string(),
                    reason: "Unsupported source URL. Add \"source_type\": \"forgejo\" to the \
                             entry if https://git.example.internal/o/unknown is a Forgejo \
                             instance."
                        .to_string(),
                },
            ]
        );
    }

    #[test]
    fn rejected_entries_quote_the_file_back_within_bounds() {
        let long_name = format!("Name {}", "x".repeat(500));
        let long_url = format!("https://git.example.internal/{}/repo", "o".repeat(500));

        let plan = plan_import(vec![
            entry(&long_name, "   "),
            entry("Unknown  host\n\twith   spaces", &long_url),
        ]);

        assert_eq!(plan.rejected.len(), 2);
        let name_label = &plan.rejected[0].label;
        assert_eq!(name_label.chars().count(), 60, "{name_label}");
        assert!(name_label.starts_with("Name xxx"), "{name_label}");
        assert!(name_label.ends_with('…'), "{name_label}");

        // Whitespace is collapsed so a multi-line name stays one line.
        assert_eq!(plan.rejected[1].label, "Unknown host with spaces");
        let reason = &plan.rejected[1].reason;
        assert!(reason.chars().count() < 200, "{reason}");
        assert!(
            reason.contains("https://git.example.internal/ooo"),
            "{reason}"
        );
        assert!(reason.contains("… is a Forgejo instance."), "{reason}");
    }

    #[test]
    fn a_private_forgejo_entry_is_flagged_as_missing_its_key_after_a_round_trip() {
        let mut private = app(
            "Private",
            SourceType::Forgejo,
            "https://git.example.internal/o/private",
        );
        private.username = Some("alice".to_string());
        private.access_token = Some("key".to_string());
        assert!(!is_missing_application_key(&private));

        let plan = plan_import(parse(&render(&[private]).unwrap()).unwrap());

        assert_eq!(plan.apps.len(), 1);
        assert!(is_missing_application_key(&plan.apps[0]));

        // A public Forgejo repository has no username, so nothing is missing.
        let public = app(
            "Public",
            SourceType::Forgejo,
            "https://codeberg.org/o/public",
        );
        assert!(!is_missing_application_key(&public));
    }

    #[test]
    fn suggested_name_is_dated_and_carries_the_extension() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 5).unwrap();
        assert_eq!(
            suggested_file_name(today),
            "Obtainintosh Programs 2026-09-05.obtainintosh"
        );
    }

    #[test]
    fn default_extension_is_added_only_when_there_is_none() {
        assert_eq!(
            with_default_extension(PathBuf::from("/tmp/list")),
            PathBuf::from("/tmp/list.obtainintosh")
        );
        assert_eq!(
            with_default_extension(PathBuf::from("/tmp/list.obtainintosh")),
            PathBuf::from("/tmp/list.obtainintosh")
        );
        assert_eq!(
            with_default_extension(PathBuf::from("/tmp/list.json")),
            PathBuf::from("/tmp/list.json")
        );
        // A trailing dot is no extension, and is not kept either.
        assert_eq!(
            with_default_extension(PathBuf::from("/tmp/list.")),
            PathBuf::from("/tmp/list.obtainintosh")
        );
        assert_eq!(
            with_default_extension(PathBuf::from("/tmp/list...")),
            PathBuf::from("/tmp/list.obtainintosh")
        );
        assert_eq!(
            with_default_extension(PathBuf::from("/")),
            PathBuf::from("/")
        );
    }

    #[test]
    fn display_name_is_the_final_component() {
        assert_eq!(
            display_name(Path::new("/tmp/Obtainintosh Programs.obtainintosh")),
            "Obtainintosh Programs.obtainintosh"
        );
        assert_eq!(display_name(Path::new("/")), "/");
    }
}
