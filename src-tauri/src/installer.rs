use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

struct IndexedApplicationDirectory {
    path: PathBuf,
    apps_by_name: HashMap<String, Vec<PathBuf>>,
}

/// One pass over the application directories, reusable for every app.
///
/// Checking N tracked programs used to mean N directory scans, because each
/// `detect_installed_app` call re-read `/Applications` from scratch. Building
/// the listing once and querying it turns that into a single pass.
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
            // within each application directory, so which bundle wins does not
            // change just because the search got faster.
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
    let app_dir = enclosing_bundle(&exe)?;
    let app_path = app_dir.to_str()?.to_string();
    let version = get_app_version(&app_path)?;
    Some((app_path, version))
}

/// Nearest ancestor that is a macOS .app bundle directory. Walks up instead of
/// assuming the executable sits exactly at `<Name>.app/Contents/MacOS/<bin>`,
/// so helper binaries nested deeper inside the bundle still resolve. Split out
/// from `detect_running_bundle` so the walk is testable with controlled paths
/// rather than depending on where the test binary happens to live.
fn enclosing_bundle(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("app"))
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
    use super::{enclosing_bundle, index_application_directory};
    use std::path::{Path, PathBuf};

    #[test]
    fn indexed_matches_preserve_directory_and_exact_match_precedence() {
        let directory = index_application_directory(
            PathBuf::from("/Applications"),
            [
                PathBuf::from("/Applications/Thing.app"),
                PathBuf::from("/Applications/Notes.txt"),
                PathBuf::from("/Applications/Other.app"),
            ],
        );
        let index = super::InstalledAppIndex {
            directories: vec![directory],
        };

        let candidates = index.candidate_paths("Thing");

        // The exact-cased path is tried first, exactly as the unindexed
        // search did, so which bundle wins cannot change.
        assert_eq!(candidates[0], PathBuf::from("/Applications/Thing.app"));
        assert!(candidates.contains(&PathBuf::from("/Applications/Thing.app")));
        // Non-.app entries are never indexed.
        assert!(!candidates
            .iter()
            .any(|path| path.to_string_lossy().ends_with("Notes.txt")));
    }

    #[test]
    fn indexed_matches_are_case_insensitive_but_name_specific() {
        let directory = index_application_directory(
            PathBuf::from("/Applications"),
            [
                PathBuf::from("/Applications/ThInG.app"),
                PathBuf::from("/Applications/Thingamajig.app"),
            ],
        );
        let index = super::InstalledAppIndex {
            directories: vec![directory],
        };

        let candidates = index.candidate_paths("thing");

        assert!(candidates.contains(&PathBuf::from("/Applications/ThInG.app")));
        // A longer name that merely starts the same is not a match.
        assert!(!candidates.contains(&PathBuf::from("/Applications/Thingamajig.app")));
    }

    #[test]
    fn finds_enclosing_app_bundle() {
        assert_eq!(
            enclosing_bundle(Path::new(
                "/Applications/Obtainintosh.app/Contents/MacOS/obtainintosh"
            )),
            Some(Path::new("/Applications/Obtainintosh.app"))
        );
        // ancestors() starts at the path itself, so the bundle root resolves
        // to itself.
        assert_eq!(
            enclosing_bundle(Path::new("/Applications/Obtainintosh.app")),
            Some(Path::new("/Applications/Obtainintosh.app"))
        );
        // Helper binaries nested deeper inside the bundle still resolve.
        assert_eq!(
            enclosing_bundle(Path::new(
                "/Applications/Obtainintosh.app/Contents/Frameworks/helper"
            )),
            Some(Path::new("/Applications/Obtainintosh.app"))
        );
        // With nested bundles the innermost wins — that is the bundle the
        // binary actually belongs to.
        assert_eq!(
            enclosing_bundle(Path::new(
                "/x/Outer.app/Contents/Frameworks/Inner.app/Contents/MacOS/inner"
            )),
            Some(Path::new("/x/Outer.app/Contents/Frameworks/Inner.app"))
        );
    }

    /// `check_for_updates` relies on the dev-build path: outside a .app bundle
    /// the probe yields None and the compiled-in version is used instead.
    #[test]
    fn no_bundle_outside_an_app_directory() {
        assert_eq!(
            enclosing_bundle(Path::new("/home/user/project/target/debug/obtainintosh")),
            None
        );
    }
}
