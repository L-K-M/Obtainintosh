//! Detects installed programs and reads their versions.
//!
//! Platform-specific by nature: macOS looks for `.app` bundles in the
//! application directories, Linux asks the dpkg database and looks for
//! AppImages. Both sides expose the same surface — `InstalledAppIndex` for
//! batch checks, `detect_installed_app` for a single program, and
//! `detect_running_bundle` for the build that is currently running — so the
//! commands layer stays platform-free.

pub(crate) use platform::InstalledAppIndex;
pub use platform::{detect_installed_app, detect_running_bundle};

#[cfg(target_os = "macos")]
mod platform {
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
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// One pass over everything Linux can cheaply enumerate, reusable for
    /// every app — the same trade the macOS directory index makes.
    ///
    /// Two places are indexed: the dpkg database (programs installed from the
    /// `.deb` assets this build downloads) and the conventional AppImage
    /// directories. Sandboxed formats like Flatpak and Snap manage their own
    /// updates, so they are deliberately not looked at.
    pub(crate) struct InstalledAppIndex {
        /// Installed dpkg packages: package name → upstream version.
        deb_packages: HashMap<String, String>,
        /// AppImages found on disk: normalized program name → files carrying
        /// that name, in scan order.
        appimages: HashMap<String, Vec<AppImageEntry>>,
    }

    struct AppImageEntry {
        path: PathBuf,
        version: String,
    }

    impl InstalledAppIndex {
        pub(crate) fn scan() -> Self {
            Self {
                deb_packages: installed_deb_packages(),
                appimages: scan_appimage_directories(),
            }
        }

        pub(crate) fn detect(&self, app_name: &str) -> Option<(String, String)> {
            log::debug!("Searching app index for: {}", app_name);

            // A dpkg package is the system-managed install, so it wins over a
            // loose AppImage carrying the same name.
            if let Some(package) = deb_package_name(app_name) {
                if let Some(version) = self.deb_packages.get(&package) {
                    log::debug!("Found dpkg package {} version {}", package, version);
                    // dpkg tracks packages, not bundle paths; record which
                    // package the version came from instead of a location.
                    return Some((format!("deb:{package}"), version.clone()));
                }
            }

            let normalized = normalize_program_name(app_name);
            if let Some(entry) = self
                .appimages
                .get(&normalized)
                .and_then(|entries| newest_appimage(entries))
            {
                log::debug!("Found AppImage: {}", entry.path.display());
                return Some((
                    entry.path.to_string_lossy().to_string(),
                    entry.version.clone(),
                ));
            }

            log::debug!("No indexed match found for {}", app_name);
            None
        }
    }

    /// The entry with the highest version among files carrying the same name.
    /// The documented update flow downloads the new AppImage next to the old
    /// one and removes nothing, so several versions of a program routinely sit
    /// side by side — reporting anything but the newest would tell the user an
    /// update they already fetched is still pending. Versions compare
    /// numerically component-wise (1.10 beats 1.9, which file-name order gets
    /// wrong); the scan-sorted path settles exact ties.
    fn newest_appimage(entries: &[AppImageEntry]) -> Option<&AppImageEntry> {
        entries.iter().reduce(|best, candidate| {
            if version_sort_key(&candidate.version) > version_sort_key(&best.version) {
                candidate
            } else {
                best
            }
        })
    }

    /// Numeric component-wise ordering key for versions parsed out of AppImage
    /// file names. `looks_like_version` only lets digits-and-dots through, so
    /// every component parses; a malformed component (empty from "1..2", or
    /// absurdly long) just counts as 0.
    fn version_sort_key(version: &str) -> Vec<u64> {
        version
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    }

    /// Detect if a program is installed — as a dpkg package or an AppImage in
    /// the usual places — and get its version.
    pub fn detect_installed_app(app_name: &str) -> Option<(String, String)> {
        InstalledAppIndex::scan().detect(app_name)
    }

    /// The Linux counterpart of the macOS running-bundle probe. An AppImage
    /// knows where it runs from — its runtime exports the file's path as
    /// `APPIMAGE` — and the version is baked into that file name. A
    /// dpkg-installed build is found by the ordinary package lookup instead,
    /// and a dev build has neither, so this returns None and the caller falls
    /// back to the compiled-in version.
    pub fn detect_running_bundle() -> Option<(String, String)> {
        let appimage = std::env::var("APPIMAGE").ok()?;
        let file_name = Path::new(&appimage).file_name()?.to_str()?;
        let (_, version) = parse_appimage_file_name(file_name)?;
        Some((appimage, version))
    }

    /// Every installed dpkg package with its upstream version, in one
    /// subprocess for the whole index. A non-Debian system (no dpkg) simply
    /// contributes nothing.
    fn installed_deb_packages() -> HashMap<String, String> {
        let output = Command::new("dpkg-query")
            .args([
                "--show",
                "--showformat=${Package}\t${db:Status-Status}\t${Version}\n",
            ])
            .output();
        let output = match output {
            Ok(output) if output.status.success() => output,
            _ => return HashMap::new(),
        };

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let package = fields.next()?;
                let status = fields.next()?;
                let version = fields.next()?;
                // A removed-but-not-purged package still has a database entry;
                // only a genuinely installed one counts.
                (status == "installed" && !version.is_empty())
                    .then(|| (package.to_string(), clean_deb_version(version)))
            })
            .collect()
    }

    /// Directories where AppImages conventionally live: `~/Applications` (the
    /// AppImageLauncher convention) and `~/.local/bin`.
    fn appimage_directories() -> Vec<PathBuf> {
        let Some(home) = dirs::home_dir() else {
            return Vec::new();
        };
        vec![home.join("Applications"), home.join(".local").join("bin")]
    }

    fn scan_appimage_directories() -> HashMap<String, Vec<AppImageEntry>> {
        let mut appimages: HashMap<String, Vec<AppImageEntry>> = HashMap::new();
        for directory in appimage_directories() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
            // read_dir order is arbitrary; sorting keeps which file wins for a
            // name stable across scans.
            paths.sort();
            for path in paths {
                let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let Some((name, version)) = parse_appimage_file_name(file_name) else {
                    continue;
                };
                appimages
                    .entry(name)
                    .or_default()
                    .push(AppImageEntry { path, version });
            }
        }
        appimages
    }

    /// The dpkg package a program is expected to be installed as. Debian
    /// package names are lowercase-with-dashes — Tauri's own bundler derives
    /// them from the product name the same way — so "My App" is looked up as
    /// "my-app". A name that cannot be a package name yields None instead of
    /// a lookup that can never match.
    fn deb_package_name(app_name: &str) -> Option<String> {
        let package = normalize_program_name(app_name);
        let valid = package.len() >= 2
            && package.starts_with(|c: char| c.is_ascii_alphanumeric())
            && package.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '+')
            });
        valid.then_some(package)
    }

    fn normalize_program_name(name: &str) -> String {
        name.trim().to_lowercase().replace([' ', '_'], "-")
    }

    /// The upstream part of a dpkg version: `2:1.5.0-1ubuntu2` → `1.5.0`.
    /// Comparable against release tags, which never carry an epoch or a
    /// Debian revision. Debian policy only allows a dash inside the upstream
    /// version when a revision follows, so splitting at the last dash is safe.
    fn clean_deb_version(version: &str) -> String {
        let without_epoch = match version.split_once(':') {
            Some((epoch, rest))
                if !epoch.is_empty() && epoch.chars().all(|c| c.is_ascii_digit()) =>
            {
                rest
            }
            _ => version,
        };
        without_epoch
            .rsplit_once('-')
            .map(|(upstream, _)| upstream)
            .unwrap_or(without_epoch)
            .to_string()
    }

    /// Splits an AppImage file name into the normalized program name and the
    /// version baked into it. Tauri names its AppImages
    /// `name_1.2.3_amd64.AppImage`; other builders use dashes. A file without
    /// a recognizable version segment is skipped — a bare `Tool.AppImage`
    /// carries nothing to compare against a release.
    ///
    /// The extension is matched case-insensitively, like the asset picker's
    /// own `.appimage` check: an asset the picker downloads must not then be
    /// undetectable on disk because the project spells it `.APPIMAGE`.
    fn parse_appimage_file_name(file_name: &str) -> Option<(String, String)> {
        let extension_start = file_name.len().checked_sub(".AppImage".len())?;
        let stem = file_name
            .get(extension_start..)
            .filter(|extension| extension.eq_ignore_ascii_case(".appimage"))
            .and_then(|_| file_name.get(..extension_start))?;

        let segments: Vec<&str> = stem
            .split(['-', '_', ' '])
            .filter(|segment| !segment.is_empty())
            .collect();
        // The name is everything before the first version-shaped segment, so
        // that segment must not be the first — a file named only by a version
        // identifies no program.
        let version_index = segments
            .iter()
            .skip(1)
            .position(|segment| looks_like_version(segment))?
            + 1;

        let name = normalize_program_name(&segments[..version_index].join("-"));
        let version = segments[version_index]
            .strip_prefix(['v', 'V'])
            .unwrap_or(segments[version_index])
            .to_string();
        Some((name, version))
    }

    /// A digits-and-dots segment with at least one dot, optionally
    /// `v`-prefixed: `1.5.0`, `v2.1`. Deliberately strict so an architecture
    /// fragment is not mistaken for a version — `x86_64` splits at the
    /// underscore into `x86` and a version-looking bare `64`, which the
    /// required dot rules out.
    fn looks_like_version(segment: &str) -> bool {
        let digits = segment.strip_prefix(['v', 'V']).unwrap_or(segment);
        digits.starts_with(|c: char| c.is_ascii_digit())
            && digits.contains('.')
            && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
    }

    #[cfg(test)]
    mod tests {
        use super::{
            clean_deb_version, deb_package_name, parse_appimage_file_name, AppImageEntry,
            InstalledAppIndex,
        };
        use std::collections::HashMap;
        use std::path::PathBuf;

        #[test]
        fn parses_tauri_and_dash_style_appimage_names() {
            for (file_name, expected) in [
                (
                    "Obtainintosh_1.5.0_amd64.AppImage",
                    ("obtainintosh", "1.5.0"),
                ),
                (
                    "Obtainintosh-1.5.0-x86_64.AppImage",
                    ("obtainintosh", "1.5.0"),
                ),
                ("My App 2.3.AppImage", ("my-app", "2.3")),
                ("tool-v2.1.appimage", ("tool", "2.1")),
                // The extension's case is the project's business, not ours:
                // the asset picker matches it case-insensitively, so anything
                // it downloads has to be detectable here.
                ("tool-2.1.APPIMAGE", ("tool", "2.1")),
                ("tool-2.1.AppImagE", ("tool", "2.1")),
                // The name may itself contain separators; the first
                // version-shaped segment ends it.
                ("some_long_name-0.9.1.AppImage", ("some-long-name", "0.9.1")),
            ] {
                let (name, version) = parse_appimage_file_name(file_name).expect(file_name);
                assert_eq!((name.as_str(), version.as_str()), expected, "{file_name}");
            }
        }

        #[test]
        fn rejects_appimage_names_without_a_version() {
            for file_name in [
                // No version to compare against a release.
                "Tool.AppImage",
                "Tool-x86_64.AppImage",
                // A version with no name in front identifies no program.
                "1.5.0.AppImage",
                // Not an AppImage at all.
                "Tool-1.5.0.tar.gz",
                "Tool-1.5.0",
            ] {
                assert!(
                    parse_appimage_file_name(file_name).is_none(),
                    "should reject {file_name}"
                );
            }
        }

        #[test]
        fn deb_versions_lose_epoch_and_revision() {
            assert_eq!(clean_deb_version("1.5.0"), "1.5.0");
            assert_eq!(clean_deb_version("1.5.0-1"), "1.5.0");
            assert_eq!(clean_deb_version("2:1.5.0-1ubuntu2"), "1.5.0");
            // An upstream version may contain dashes when a revision follows;
            // only the revision is dropped.
            assert_eq!(clean_deb_version("1.0-beta-2"), "1.0-beta");
            assert_eq!(clean_deb_version("2:1.5.0"), "1.5.0");
        }

        #[test]
        fn program_names_map_to_debian_package_names() {
            assert_eq!(
                deb_package_name("Obtainintosh").as_deref(),
                Some("obtainintosh")
            );
            assert_eq!(deb_package_name("My App").as_deref(), Some("my-app"));
            assert_eq!(deb_package_name("gtk+2.0").as_deref(), Some("gtk+2.0"));
            // Names dpkg could never know are not looked up at all.
            assert_eq!(deb_package_name("x"), None);
            assert_eq!(deb_package_name("App!"), None);
            assert_eq!(deb_package_name("+plus-first"), None);
        }

        #[test]
        fn index_prefers_dpkg_over_appimage_and_matches_normalized_names() {
            let index = InstalledAppIndex {
                deb_packages: HashMap::from([("my-app".to_string(), "2.0.0".to_string())]),
                appimages: HashMap::from([(
                    "my-app".to_string(),
                    vec![AppImageEntry {
                        path: PathBuf::from("/home/user/Applications/My_App_1.9.0.AppImage"),
                        version: "1.9.0".to_string(),
                    }],
                )]),
            };

            // The system-managed package wins over the loose file.
            assert_eq!(
                index.detect("My App"),
                Some(("deb:my-app".to_string(), "2.0.0".to_string()))
            );

            let appimage_only = InstalledAppIndex {
                deb_packages: HashMap::new(),
                appimages: index.appimages,
            };
            assert_eq!(
                appimage_only.detect("My App"),
                Some((
                    "/home/user/Applications/My_App_1.9.0.AppImage".to_string(),
                    "1.9.0".to_string()
                ))
            );
            assert_eq!(appimage_only.detect("Other"), None);
        }

        /// Several versions of an AppImage routinely sit side by side: the
        /// documented update flow drops the new file next to the old one and
        /// removes nothing. Reporting the older one would leave the program
        /// stuck showing an update the user already downloaded.
        #[test]
        fn the_newest_appimage_wins_when_several_versions_sit_side_by_side() {
            let entries = |versions: &[&str]| {
                versions
                    .iter()
                    .map(|version| AppImageEntry {
                        path: PathBuf::from(format!(
                            "/home/user/Applications/Tool-{version}.AppImage"
                        )),
                        version: version.to_string(),
                    })
                    .collect::<Vec<_>>()
            };

            for versions in [
                // An ordinary bump, in both scan orders.
                ["1.5.0", "1.6.0"],
                ["1.6.0", "1.5.0"],
                // Numeric, not lexicographic: "1.10.0" < "1.9.0" as strings.
                ["1.9.0", "1.10.0"],
                ["1.10.0", "1.9.0"],
            ] {
                let index = InstalledAppIndex {
                    deb_packages: HashMap::new(),
                    appimages: HashMap::from([("tool".to_string(), entries(&versions))]),
                };
                let expected = if versions.contains(&"1.10.0") {
                    "1.10.0"
                } else {
                    "1.6.0"
                };
                assert_eq!(
                    index.detect("Tool"),
                    Some((
                        format!("/home/user/Applications/Tool-{expected}.AppImage"),
                        expected.to_string()
                    )),
                    "scan order {versions:?}"
                );
            }
        }
    }
}
