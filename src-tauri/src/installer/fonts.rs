//! Detects installed fonts and reads their versions.
//!
//! Font installers — Font Book, GNOME Fonts, a copy into `~/.fonts` — keep
//! the file's name, so an installed font is found by the name of the file the
//! entry downloads. Any installable format counts: an entry that downloads
//! `C64Keyboard-Regular.ttf` is installed when `C64Keyboard-Regular.otf` is.

use crate::font_file::{self, Step, VersionReader};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How deep font directories are searched: enough for the per-package
/// folders of `/usr/share/fonts/truetype/<package>`.
const MAX_DEPTH: usize = 4;

/// One pass over the font directories, reusable for every tracked font.
pub(crate) struct InstalledFontIndex {
    /// Lowercased file name without its extension → font files carrying it,
    /// in scan order.
    fonts_by_name: HashMap<String, Vec<PathBuf>>,
}

impl InstalledFontIndex {
    pub(crate) fn scan() -> Self {
        Self::of(font_directories())
    }

    fn of(directories: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut fonts_by_name = HashMap::new();
        for directory in directories {
            collect_fonts(&directory, 0, &mut fonts_by_name);
        }
        Self { fonts_by_name }
    }

    /// Where the font a file name stands for is installed, and its version.
    /// Of several copies the newest wins: an update installed beside the old
    /// copy is the one the user just fetched.
    pub(crate) fn detect(&self, font_file_name: &str) -> Option<(String, String)> {
        let copies = self.fonts_by_name.get(&font_key(font_file_name)?)?;
        copies
            .iter()
            .filter_map(|path| Some((path, read_version(path).ok()?)))
            .reduce(|newest, copy| {
                if version_key(&copy.1) > version_key(&newest.1) {
                    copy
                } else {
                    newest
                }
            })
            .map(|(path, version)| (path.to_string_lossy().into_owned(), version))
    }
}

/// The version of the font file at `path`, or None when it is gone or
/// cannot be read.
pub(crate) fn installed_font_version(path: &Path) -> Option<String> {
    read_version(path).ok()
}

/// The user's font directory first, then the system-wide ones.
fn font_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = dirs::font_dir().into_iter().collect();
    if cfg!(target_os = "macos") {
        directories.push(PathBuf::from("/Library/Fonts"));
    } else {
        directories.extend(dirs::home_dir().map(|home| home.join(".fonts")));
        directories.extend(["/usr/local/share/fonts", "/usr/share/fonts"].map(PathBuf::from));
    }
    directories
}

/// Indexes the font files under `directory`. Symlinked directories are not
/// followed, so a link loop cannot trap the scan.
fn collect_fonts(
    directory: &Path,
    depth: usize,
    fonts_by_name: &mut HashMap<String, Vec<PathBuf>>,
) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<(PathBuf, bool)> = entries
        .flatten()
        .map(|entry| {
            let is_directory = entry.file_type().is_ok_and(|kind| kind.is_dir());
            (entry.path(), is_directory)
        })
        .collect();
    // read_dir order is arbitrary; sorting keeps which copy wins a tie
    // stable across scans.
    entries.sort();

    for (path, is_directory) in entries {
        if is_directory {
            if depth < MAX_DEPTH {
                collect_fonts(&path, depth + 1, fonts_by_name);
            }
            continue;
        }

        let Some(key) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(font_key)
        else {
            continue;
        };
        fonts_by_name.entry(key).or_default().push(path);
    }
}

/// `C64Keyboard-Regular.TTF` → `c64keyboard-regular`; None for a file that
/// is no installable font.
fn font_key(file_name: &str) -> Option<String> {
    if !font_file::is_font_file(file_name) {
        return None;
    }
    let stem = &file_name[..file_name.rfind('.')?];
    Some(stem.to_lowercase())
}

/// Numeric component-wise ordering, so 1.10 is newer than 1.9.
fn version_key(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn read_version(path: &Path) -> anyhow::Result<String> {
    let mut file = File::open(path)?;
    let mut reader = VersionReader::default();
    let mut step = reader.start();
    loop {
        let range = match step {
            Step::Version(version) => return Ok(version),
            Step::Read(range) => range,
        };
        file.seek(SeekFrom::Start(range.start))?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(range.end - range.start)
            .read_to_end(&mut bytes)?;
        step = reader.advance(&bytes)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_file::tests::font_bytes;

    const WINDOWS_PLATFORM: u16 = 3;

    /// A scratch directory removed when the test ends.
    struct ScratchDirectory(PathBuf);

    impl ScratchDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("obtainintosh-fonts-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn font(&self, relative_path: &str, version: &str) -> PathBuf {
            let path = self.0.join(relative_path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let declared = format!("Version {version}");
            std::fs::write(&path, font_bytes(&[(WINDOWS_PLATFORM, &declared)])).unwrap();
            path
        }
    }

    impl Drop for ScratchDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn finds_an_installed_font_by_file_name_in_any_format() {
        let fonts = ScratchDirectory::new("formats");
        let installed = fonts.font("c64keyboard-regular.OTF", "1.107");
        fonts.font("Other-Regular.ttf", "9.0");

        let index = InstalledFontIndex::of([fonts.0.clone()]);

        assert_eq!(
            index.detect("C64Keyboard-Regular.ttf"),
            Some((
                installed.to_string_lossy().into_owned(),
                "1.107".to_string()
            ))
        );
        assert_eq!(index.detect("Missing-Regular.ttf"), None);
        assert_eq!(index.detect("C64Keyboard-Regular.zip"), None);
    }

    #[test]
    fn the_newest_copy_wins() {
        let user = ScratchDirectory::new("user");
        let system = ScratchDirectory::new("system");
        user.font("Inter.ttf", "3.9");
        let newer = system.font("truetype/inter/Inter.ttf", "3.10");

        let index = InstalledFontIndex::of([user.0.clone(), system.0.clone()]);

        assert_eq!(
            index.detect("Inter.ttf"),
            Some((newer.to_string_lossy().into_owned(), "3.10".to_string()))
        );
    }

    #[test]
    fn unreadable_copies_are_skipped() {
        let fonts = ScratchDirectory::new("unreadable");
        std::fs::write(fonts.0.join("Broken.ttf"), b"not a font").unwrap();

        let index = InstalledFontIndex::of([fonts.0.clone()]);

        assert_eq!(index.detect("Broken.ttf"), None);
    }

    #[test]
    fn reads_the_version_of_a_font_at_a_known_path() {
        let fonts = ScratchDirectory::new("path");
        let path = fonts.font("Abacus.otf", "2.0");

        assert_eq!(installed_font_version(&path).as_deref(), Some("2.0"));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(installed_font_version(&path), None);
    }
}
