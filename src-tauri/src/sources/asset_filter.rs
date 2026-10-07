//! Which program of a release an entry tracks, for repositories that publish
//! several programs side by side:
//!
//! ```text
//!   seance-macos-universal.zip      ─┐
//!   seance_1.9.0-1_amd64.deb         ├─ program "seance"
//!   seance-linux-x64.AppImage       ─┘
//!   seance-sync-linux-x64.tar.gz    ── program "seance-sync"
//!   planchette-macos-universal.zip  ── program "planchette"
//! ```
//!
//! A filter narrows the assets the platform picker ranks to one program, so
//! the picker's platform and architecture preferences still apply within it.
//! It is either a program name, compared with each asset's program name, or a
//! wildcard pattern over the whole file name for naming schemes that program
//! names cannot tell apart (`*-gtk4-*`).

use super::UNSUPPORTED_CPU_MARKERS;

/// Characters that make a filter a file name pattern rather than a program
/// name. Neither can appear in a release asset's file name.
const WILDCARDS: [char; 2] = ['*', '?'];

/// Packaging suffixes removed before a file name is split into words, so that
/// `tar` and `gz` cannot continue a program name. Composite suffixes come
/// before the shorter ones they end with: the first match wins.
const PACKAGE_SUFFIXES: &[&str] = &[
    ".app.tar.gz",
    ".tar.gz",
    ".tar.xz",
    ".tar.bz2",
    ".tar.zst",
    ".appimage",
    ".flatpak",
    ".dmg",
    ".pkg",
    ".zip",
    ".deb",
    ".rpm",
    ".tgz",
    ".snap",
    ".apk",
    ".ipa",
    ".exe",
    ".msi",
    ".7z",
    ".tar",
    ".gz",
    ".xz",
    ".bz2",
    ".zst",
];

/// Operating system names. Every marker either platform picker recognises is
/// here, so a program name always ends before the platform part of a file
/// name, also when a version is fused on (`macos14`, `win11`).
const OS_WORDS: &[&str] = &[
    "mac", "macos", "macosx", "osx", "darwin", "apple", "linux", "linux32", "linux64", "ubuntu",
    "debian", "fedora", "rhel", "centos", "alpine", "windows", "win", "win32", "win64", "android",
    "ios", "ipados", "tvos", "watchos", "freebsd", "openbsd", "netbsd", "solaris", "illumos",
];

/// Words that describe a build of the program rather than the program: the
/// architectures the pickers recognise (`x86_64` splits into `x86` and `64`),
/// Apple Silicon spellings, libc and toolchain names from target triples, and
/// packaging variants of the same program. Words such as `cli`, `sync`, or
/// `nightly` are deliberately absent: they tell programs or channels apart.
const BUILD_WORDS: &[&str] = &[
    "universal",
    "universal2",
    "arm64",
    "arm64e",
    "aarch64",
    "arm",
    "armv6l",
    "armv7l",
    "armv8",
    "x86",
    "x64",
    "amd64",
    "intel",
    "silicon",
    "applesilicon",
    "gnu",
    "musl",
    "msvc",
    "mingw",
    "static",
    "portable",
    "installer",
    "setup",
    "standalone",
    "bundle",
    "bin",
];

/// Shortest commit hash treated as one; `git describe` abbreviates to 7.
const MIN_COMMIT_HASH_LEN: usize = 7;

/// The program an entry tracks within its repository's releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetFilter {
    /// Assets whose program name is this one, e.g. `seance`.
    Program(String),
    /// Assets whose whole file name, lowercased, matches this lowercased
    /// pattern, where `*` stands for any run of characters and `?` for one.
    Pattern(String),
}

impl AssetFilter {
    /// Reads a filter as the user typed it; `None` when blank. A file name
    /// counts as its program and a download link as its file name, so
    /// `Seance`, `seance-macos-universal.zip`, and a link to that file all
    /// name the program `seance`. Input that could select nothing in
    /// particular is refused rather than silently tracking every program.
    pub fn parse(input: &str) -> Result<Option<Self>, String> {
        let input = input.trim();
        if input.is_empty() {
            return Ok(None);
        }

        let file_name = if input.contains("://") {
            link_file_name(input)?
        } else {
            input
        };
        if file_name.contains(WILDCARDS) {
            if !file_name.chars().any(char::is_alphanumeric) {
                return Err(format!(
                    "The pattern \"{input}\" matches every file. Leave the program blank \
                     to let Obtainintosh choose."
                ));
            }
            return Ok(Some(Self::Pattern(file_name.to_lowercase())));
        }

        match program_name(file_name) {
            Some(name) => Ok(Some(Self::Program(name))),
            None => Err(format!(
                "\"{input}\" is not a program name. Enter the name the program's release \
                 files start with, such as seance for seance-macos-universal.zip."
            )),
        }
    }

    /// The canonical form of a filter as the user typed it, `None` when
    /// blank; see `parse`.
    pub fn canonicalize(input: &str) -> Result<Option<String>, String> {
        Ok(Self::parse(input)?.map(Self::into_canonical))
    }

    /// The form stored with an entry and compared to tell entries apart.
    /// Parsing it yields this filter again.
    pub fn canonical(&self) -> &str {
        match self {
            Self::Program(name) | Self::Pattern(name) => name,
        }
    }

    fn into_canonical(self) -> String {
        match self {
            Self::Program(name) | Self::Pattern(name) => name,
        }
    }

    pub fn matches(&self, file_name: &str) -> bool {
        match self {
            Self::Program(name) => program_name(file_name).as_deref() == Some(name.as_str()),
            Self::Pattern(pattern) => wildcard_match(pattern, &file_name.to_lowercase()),
        }
    }
}

/// The program a release asset belongs to: the leading words of its file
/// name, up to the first word that describes the build instead.
///
/// ```text
///   seance_1.9.0-1_amd64.deb          → seance
///   seance-sync-macos-arm64.tar.gz    → seance-sync
///   Mac-Mouse-Fix-3.0.dmg             → mac-mouse-fix   (first word always counts)
///   1.0-linux.tar.gz                  → None            (only a version)
/// ```
pub fn program_name(file_name: &str) -> Option<String> {
    let words = words(file_name);
    let (first, rest) = words.split_first()?;
    if is_bare_version(first) {
        return None;
    }

    let mut name = vec![first.as_str()];
    name.extend(
        rest.iter()
            .map(String::as_str)
            .take_while(|word| !describes_build(word)),
    );
    Some(name.join("-"))
}

/// The release file a download link points at:
/// `https://host/o/r/releases/download/v1/seance.zip?x=1` → `seance.zip`. A
/// link to anything else, such as the repository itself, names no program.
fn link_file_name(link: &str) -> Result<&str, String> {
    let path = link.split(['?', '#']).next().unwrap_or(link);
    let file_name = path
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or(path);
    let lower = file_name.to_lowercase();
    if PACKAGE_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
    {
        return Ok(file_name);
    }
    Err(format!(
        "\"{link}\" is not a link to a release file. Enter the program's name instead, \
         such as seance for seance-macos-universal.zip."
    ))
}

/// The lowercase words of a file name without its packaging suffix. Unicode
/// letters count as word characters, so `séance` stays one word.
fn words(file_name: &str) -> Vec<String> {
    let lower = file_name.trim().to_lowercase();
    let base = PACKAGE_SUFFIXES
        .iter()
        .find_map(|suffix| lower.strip_suffix(suffix))
        .unwrap_or(&lower);
    base.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn describes_build(word: &str) -> bool {
    is_version(word)
        || is_commit_hash(word)
        || is_versioned_os(word)
        || is_apple_chip(word)
        || OS_WORDS.contains(&word)
        || BUILD_WORDS.contains(&word)
        || UNSUPPORTED_CPU_MARKERS.contains(&word)
}

/// `1`, `2024`, `v2`, `v2beta`: any word that starts with a number.
fn is_version(word: &str) -> bool {
    word.strip_prefix('v')
        .unwrap_or(word)
        .starts_with(|c: char| c.is_ascii_digit())
}

/// `1` or `v2`, but not `7zip`: a first word that is nothing but a version
/// leaves the file name without a program.
fn is_bare_version(word: &str) -> bool {
    let digits = word.strip_prefix('v').unwrap_or(word);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// A commit hash, bare (`3f9a2c1`) or as `git describe` writes it
/// (`g3f9a2c1`). It changes with every build, so letting it into the program
/// name would make the name unmatchable by the next release. A hex word
/// without digits (`facade`) is a word, not a hash.
fn is_commit_hash(word: &str) -> bool {
    let hash = word.strip_prefix('g').unwrap_or(word);
    hash.len() >= MIN_COMMIT_HASH_LEN
        && hash.chars().all(|c| c.is_ascii_hexdigit())
        && hash.chars().any(|c| c.is_ascii_digit())
}

/// An Apple Silicon chip generation, `m1` onwards, so that builds named for
/// one (`tool-m4.dmg`) need no list update when the next chip ships. The
/// price: such a word inside a program's own name (`bmw-m3-tools`) ends the
/// name early, which a wildcard pattern works around.
fn is_apple_chip(word: &str) -> bool {
    const MAX_GENERATION_DIGITS: usize = 2;
    word.strip_prefix('m').is_some_and(|generation| {
        (1..=MAX_GENERATION_DIGITS).contains(&generation.len())
            && generation.chars().all(|c| c.is_ascii_digit())
    })
}

/// An operating system name with its version fused on: `macos14`, `win11`,
/// `ubuntu2204`.
fn is_versioned_os(word: &str) -> bool {
    let Some(split) = word.find(|c: char| c.is_ascii_digit()) else {
        return false;
    };
    let (os, version) = word.split_at(split);
    OS_WORDS.contains(&os) && version.chars().all(|c| c.is_ascii_digit())
}

/// Whole-string match where `*` stands for any run of characters and `?` for
/// exactly one. On a mismatch it backtracks to the most recent `*` only,
/// which suffices: a later `*` can absorb anything an earlier one could.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    // The last `*` seen, and the text position it currently absorbs up to.
    let mut star: Option<(usize, usize)> = None;

    while t < text.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(&c) if c == '?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => {
                let Some((star_p, star_t)) = star else {
                    return false;
                };
                star = Some((star_p, star_t + 1));
                p = star_p + 1;
                t = star_t + 1;
            }
        }
    }

    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(input: &str) -> Option<String> {
        program_name(input)
    }

    #[test]
    fn hauntware_assets_group_into_their_programs() {
        let cases = [
            ("planchette-macos-universal.zip", "planchette"),
            ("planchette_1.9.0-1_amd64.deb", "planchette"),
            ("planchette-linux-x64.AppImage", "planchette"),
            ("planchette-linux-x64.flatpak", "planchette"),
            ("poltergeist-ios-unsigned.ipa", "poltergeist"),
            ("seance-linux-x64.tar.gz", "seance"),
            ("seance_1.9.0-1_amd64.deb", "seance"),
            ("seance-sync-linux-x64.tar.gz", "seance-sync"),
            ("seance-sync-macos-arm64.tar.gz", "seance-sync"),
        ];
        for (file_name, expected) in cases {
            assert_eq!(program(file_name).as_deref(), Some(expected), "{file_name}");
        }
    }

    #[test]
    fn common_release_tool_naming_yields_the_bare_program() {
        let cases = [
            // Tauri bundler
            ("MyApp_1.6.0_x64.dmg", "myapp"),
            ("MyApp_1.6.0_aarch64.dmg", "myapp"),
            ("MyApp.app.tar.gz", "myapp"),
            ("my-app_1.6.0_amd64.AppImage", "my-app"),
            // electron-builder
            ("MyApp-1.2.3-arm64-mac.zip", "myapp"),
            ("MyApp-1.2.3-universal.dmg", "myapp"),
            // goreleaser
            ("tool_Darwin_arm64.tar.gz", "tool"),
            ("tool_2.1.0_linux_amd64.tar.gz", "tool"),
            // cargo-dist
            ("tool-x86_64-unknown-linux-gnu.tar.xz", "tool"),
            ("tool-aarch64-apple-darwin.tar.xz", "tool"),
            // Apple Silicon spellings and fused OS versions
            ("Tool-AppleSilicon.dmg", "tool"),
            ("tool-m1.dmg", "tool"),
            ("Tool-M5-Max.dmg", "tool"),
            ("Tool-Apple-Silicon.dmg", "tool"),
            ("tool-macos14-arm64.zip", "tool"),
            ("tool-win11-x64.zip", "tool"),
            ("tool-ubuntu2204.deb", "tool"),
            // Packaging variants of the same program
            ("Tool-Setup-1.0.pkg", "tool"),
            ("tool-portable-linux.zip", "tool"),
            // Bare archive suffixes
            ("tool.tar", "tool"),
            ("tool.tar.zst", "tool"),
            ("tool.bz2", "tool"),
        ];
        for (file_name, expected) in cases {
            assert_eq!(program(file_name).as_deref(), Some(expected), "{file_name}");
        }
    }

    #[test]
    fn program_words_survive_and_the_first_word_always_counts() {
        let cases = [
            ("Mac-Mouse-Fix-3.0.dmg", "mac-mouse-fix"),
            ("7zip-24.08-linux-x64.tar.xz", "7zip"),
            ("0ad-0.27-linux.AppImage", "0ad"),
            ("v2ray-linux-64.zip", "v2ray"),
            ("tool-cli-linux-x64.tar.gz", "tool-cli"),
            ("tool-nightly-linux.zip", "tool-nightly"),
            ("Séance-macos.zip", "séance"),
            ("My App 1.0.dmg", "my-app"),
        ];
        for (file_name, expected) in cases {
            assert_eq!(program(file_name).as_deref(), Some(expected), "{file_name}");
        }
    }

    #[test]
    fn commit_hashes_end_the_program_name() {
        assert_eq!(program("tool-3f9a2c1-linux.zip").as_deref(), Some("tool"));
        assert_eq!(program("tool-g3f9a2c1-linux.zip").as_deref(), Some("tool"));
        // Hex letters without a digit form a word, not a hash.
        assert_eq!(
            program("tool-facade-linux.zip").as_deref(),
            Some("tool-facade")
        );
    }

    #[test]
    fn names_without_a_program_have_none() {
        assert_eq!(program("1.0-linux.tar.gz"), None);
        assert_eq!(program("v2-macos.zip"), None);
        assert_eq!(program("---.zip"), None);
        assert_eq!(program(""), None);
    }

    #[test]
    fn parse_reads_names_file_names_and_links_as_the_same_program() {
        let expected = Some(AssetFilter::Program("seance".to_string()));
        for input in [
            "seance",
            "  Seance ",
            "seance-macos-universal.zip",
            "seance_1.9.0-1_amd64.deb",
            "https://github.com/L-K-M/Hauntware/releases/download/v1.9.0/seance-linux-x64.AppImage?raw=1",
        ] {
            assert_eq!(AssetFilter::parse(input).unwrap(), expected, "{input}");
        }

        assert_eq!(
            AssetFilter::parse("Seance Sync").unwrap(),
            Some(AssetFilter::Program("seance-sync".to_string()))
        );
    }

    #[test]
    fn parse_treats_wildcards_as_a_lowercased_pattern() {
        assert_eq!(
            AssetFilter::parse(" *-GTK4-*.zip ").unwrap(),
            Some(AssetFilter::Pattern("*-gtk4-*.zip".to_string()))
        );
        assert_eq!(
            AssetFilter::parse("seance-?.zip").unwrap(),
            Some(AssetFilter::Pattern("seance-?.zip".to_string()))
        );
    }

    #[test]
    fn parse_refuses_links_to_anything_but_a_release_file() {
        for link in [
            "https://github.com/L-K-M/Hauntware",
            "https://github.com/L-K-M/Hauntware/",
            "https://github.com/L-K-M/Hauntware/releases/tag/v1.9.0",
        ] {
            let error = AssetFilter::parse(link).unwrap_err();
            assert!(error.contains("not a link to a release file"), "{error}");
        }
    }

    #[test]
    fn parse_leaves_blank_input_unfiltered() {
        assert_eq!(AssetFilter::parse("").unwrap(), None);
        assert_eq!(AssetFilter::parse("   ").unwrap(), None);
    }

    #[test]
    fn parse_refuses_filters_that_select_nothing_in_particular() {
        for input in ["*", "*.*", "?*", "1.9.0", "v2", "---"] {
            assert!(AssetFilter::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn the_canonical_form_parses_back_to_the_same_filter() {
        for input in ["Seance Sync", "Mac-Mouse-Fix-3.0.dmg", "*-GTK4-*", "séance"] {
            let filter = AssetFilter::parse(input).unwrap().unwrap();
            assert_eq!(
                AssetFilter::parse(filter.canonical()).unwrap(),
                Some(filter.clone()),
                "{input}"
            );
            assert_eq!(
                AssetFilter::canonicalize(input).unwrap().as_deref(),
                Some(filter.canonical()),
                "{input}"
            );
        }
        assert_eq!(AssetFilter::canonicalize(" ").unwrap(), None);
    }

    #[test]
    fn a_program_filter_matches_its_own_assets_only() {
        let seance = AssetFilter::parse("seance").unwrap().unwrap();
        assert!(seance.matches("seance-macos-universal.zip"));
        assert!(seance.matches("Seance_1.9.0-1_amd64.deb"));
        assert!(!seance.matches("seance-sync-linux-x64.tar.gz"));
        assert!(!seance.matches("planchette-macos-universal.zip"));

        let sync = AssetFilter::parse("seance-sync").unwrap().unwrap();
        assert!(sync.matches("seance-sync-linux-x64.tar.gz"));
        assert!(!sync.matches("seance-linux-x64.tar.gz"));
    }

    #[test]
    fn a_pattern_filter_matches_whole_file_names_case_insensitively() {
        let gtk4 = AssetFilter::parse("*-gtk4-*").unwrap().unwrap();
        assert!(gtk4.matches("Tool-1.0-Linux-GTK4-x64.AppImage"));
        assert!(!gtk4.matches("tool-1.0-linux-gtk3-x64.AppImage"));

        let exact = AssetFilter::parse("seance-?.zip").unwrap().unwrap();
        assert!(exact.matches("seance-a.zip"));
        assert!(!exact.matches("seance-ab.zip"));
        assert!(!exact.matches("xseance-a.zip"));
    }

    #[test]
    fn wildcard_match_backtracks_across_stars() {
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("a*b*c", "aXbYbZc"));
        assert!(wildcard_match("*.tar.gz", "x.tar.gz.tar.gz"));
        assert!(wildcard_match("a?c*", "abcdef"));
        assert!(!wildcard_match("a*b", "aXbYc"));
        assert!(!wildcard_match("abc", "abcd"));
        assert!(!wildcard_match("", "a"));
    }
}
