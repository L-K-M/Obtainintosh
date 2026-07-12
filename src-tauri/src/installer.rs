use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

struct IndexedApplicationDirectory {
    path: PathBuf,
    apps_by_name: HashMap<String, Vec<PathBuf>>,
}

pub(crate) struct InstalledAppIndex {
    directories: Vec<IndexedApplicationDirectory>,
}

impl InstalledAppIndex {
    pub(crate) fn scan() -> Self {
        let directories = application_directories()
            .into_iter()
            .map(|path| {
                let entries = std::fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.ok().map(|entry| entry.path()));
                index_application_directory(path, entries)
            })
            .collect();

        Self { directories }
    }

    pub(crate) fn detect(&self, app_name: &str) -> Option<(String, String)> {
        log::debug!("Searching app index for: {}", app_name);

        for path in self.candidate_paths(app_name) {
            log::debug!("Trying indexed path: {}", path.display());
            let path_string = path.to_string_lossy().to_string();
            if let Some(version) = get_app_version(&path_string) {
                log::debug!("Found indexed app version: {}", version);
                return Some((path_string, version));
            }
        }

        log::debug!("No indexed match found for {}", app_name);
        None
    }

    fn candidate_paths(&self, app_name: &str) -> Vec<PathBuf> {
        let normalized_name = app_name.to_lowercase();
        let mut paths = Vec::new();

        for directory in &self.directories {
            // Preserve the existing exact-before-case-insensitive lookup order
            // within each application directory.
            paths.push(directory.path.join(format!("{app_name}.app")));
            if let Some(matches) = directory.apps_by_name.get(&normalized_name) {
                paths.extend(matches.iter().cloned());
            }
        }

        paths
    }
}

/// Detect if an app is installed in /Applications or ~/Applications and get its version
pub fn detect_installed_app(app_name: &str) -> Option<(String, String)> {
    InstalledAppIndex::scan().detect(app_name)
}

fn application_directories() -> Vec<PathBuf> {
    let mut directories = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        directories.push(home.join("Applications"));
    }
    directories
}

fn index_application_directory(
    path: PathBuf,
    entries: impl IntoIterator<Item = PathBuf>,
) -> IndexedApplicationDirectory {
    let mut apps_by_name: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for entry in entries {
        if entry.extension().and_then(|extension| extension.to_str()) != Some("app") {
            continue;
        }
        if let Some(name) = entry.file_stem().and_then(|name| name.to_str()) {
            apps_by_name
                .entry(name.to_lowercase())
                .or_default()
                .push(entry);
        }
    }

    IndexedApplicationDirectory { path, apps_by_name }
}

/// Locate the .app bundle the current process runs from and read its version,
/// like `detect_installed_app`. Reading from disk (rather than the compiled-in
/// version) stays truthful when the bundle is replaced by an update while the
/// old binary keeps running. Dev builds run outside a bundle and return None.
pub fn detect_running_bundle() -> Option<(String, String)> {
    let exe = std::env::current_exe().ok()?;
    // Resolve symlinks so a linked binary still maps back to its real bundle.
    let exe = exe.canonicalize().unwrap_or(exe);
    // Walk up instead of assuming the executable sits exactly at
    // <Name>.app/Contents/MacOS/<bin>, so helper binaries nested deeper
    // inside the bundle still resolve.
    let app_dir = exe
        .ancestors()
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("app"))?;
    let app_path = app_dir.to_str()?.to_string();
    let version = get_app_version(&app_path)?;
    Some((app_path, version))
}

/// Get version from an installed .app bundle
fn get_app_version(app_path: &str) -> Option<String> {
    let plist_path = format!("{}/Contents/Info.plist", app_path);

    if !Path::new(&plist_path).exists() {
        return None;
    }

    // Use PlistBuddy to read version
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString", &plist_path])
        .output()
        .ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }

    // Fallback to CFBundleVersion
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleVersion", &plist_path])
        .output()
        .ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !version.is_empty() {
            return Some(version);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_matches_preserve_directory_and_exact_match_precedence() {
        let system = PathBuf::from("/Applications");
        let user = PathBuf::from("/Users/test/Applications");
        let index = InstalledAppIndex {
            directories: vec![
                index_application_directory(
                    system.clone(),
                    [
                        system.join("example.app"),
                        system.join("Example.app"),
                        system.join("EXAMPLE.app"),
                        system.join("Example.APP"),
                        system.join("Example.txt"),
                    ],
                ),
                index_application_directory(user.clone(), [user.join("example.app")]),
            ],
        };

        assert_eq!(
            index.candidate_paths("Example"),
            vec![
                system.join("Example.app"),
                system.join("example.app"),
                system.join("Example.app"),
                system.join("EXAMPLE.app"),
                user.join("Example.app"),
                user.join("example.app"),
            ]
        );
    }

    #[test]
    fn indexed_matches_are_case_insensitive_but_name_specific() {
        let directory = PathBuf::from("/Applications");
        let index = InstalledAppIndex {
            directories: vec![index_application_directory(
                directory.clone(),
                [
                    directory.join("Preview.app"),
                    directory.join("Previewer.app"),
                ],
            )],
        };

        assert_eq!(
            index.candidate_paths("preview"),
            vec![directory.join("preview.app"), directory.join("Preview.app")]
        );
    }
}
