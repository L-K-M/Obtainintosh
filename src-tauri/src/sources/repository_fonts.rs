//! Fonts committed to a repository that publishes no releases, such as
//! github.com/szabadkai/c64-keyboard-font:
//!
//! ```text
//!   fonts/C64Keyboard-Regular.ttf  ─┐ font "c64keyboard-regular",
//!   fonts/C64Keyboard-Regular.otf  ─┘ version 1.107 (declared by the font)
//!   C64-Keyboard.zip                  not a font file, ignored
//! ```
//!
//! The files take the place of a release's assets, so program filters and
//! the platform picker work on them unchanged. With no release tag to go by,
//! the version is the one the chosen font declares, read with a few range
//! requests rather than a download.

use super::{find_compatible_asset, list_programs, program_name, AssetFilter, ReleaseAsset};
use crate::font_file::{self, Step, VersionReader};
use crate::models::{Release, ReleasePrograms};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::ops::Range;

/// A forge's recursive listing of a repository's files. GitHub and Forgejo
/// name these fields alike; only Forgejo pages the listing and counts it.
#[derive(Debug, Deserialize)]
pub(super) struct Tree {
    pub(super) tree: Vec<TreeEntry>,
    #[serde(default)]
    pub(super) total_count: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    size: u64,
}

const FILE_ENTRY: &str = "blob";

pub(super) struct RepositoryFonts {
    /// Shallowest path first, so that of two copies of a font the picker
    /// keeps `fonts/A.ttf` over `docs/site/fonts/A.ttf`.
    files: Vec<ReleaseAsset>,
}

impl RepositoryFonts {
    /// The font files among a listing's entries, or None when there are
    /// none. `raw_url` gives the URL that serves a path's contents.
    pub(super) fn from_tree(
        entries: impl IntoIterator<Item = TreeEntry>,
        raw_url: impl Fn(&str) -> Result<String>,
    ) -> Result<Option<Self>> {
        let mut entries: Vec<TreeEntry> = entries
            .into_iter()
            .filter(|entry| entry.kind == FILE_ENTRY && font_file::is_font_file(&entry.path))
            .collect();
        entries.sort_by(|a, b| {
            let depth = |entry: &TreeEntry| entry.path.matches('/').count();
            depth(a).cmp(&depth(b)).then_with(|| a.path.cmp(&b.path))
        });

        let files = entries
            .into_iter()
            .map(|entry| {
                let name = entry.path.rsplit('/').next().unwrap_or(&entry.path);
                Ok(ReleaseAsset {
                    name: name.to_string(),
                    browser_download_url: raw_url(&entry.path)?,
                    size: entry.size,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((!files.is_empty()).then_some(Self { files }))
    }

    /// The font to download, with the version it declares. `request` starts
    /// an authenticated GET of a file URL.
    pub(super) async fn release(
        &self,
        filter: Option<&AssetFilter>,
        request: impl Fn(&str) -> reqwest::RequestBuilder,
    ) -> Result<Release> {
        let font = self.pick(filter)?;
        Ok(Release {
            version: read_version(&font.browser_download_url, &request).await?,
            download_url: font.browser_download_url.clone(),
            file_name: font.name.clone(),
            file_size: Some(font.size),
            checksum: None,
            release_notes: None,
        })
    }

    /// The fonts on offer, and the version of the one an entry without a
    /// filter gets.
    pub(super) async fn programs(
        &self,
        request: impl Fn(&str) -> reqwest::RequestBuilder,
    ) -> Result<ReleasePrograms> {
        let font = self.pick(None)?;
        Ok(ReleasePrograms {
            version: read_version(&font.browser_download_url, &request).await?,
            programs: self.names(),
            default_program: program_name(&font.name),
        })
    }

    fn pick(&self, filter: Option<&AssetFilter>) -> Result<&ReleaseAsset> {
        if let Some(font) = find_compatible_asset(&self.files, filter) {
            return Ok(font);
        }

        match filter {
            Some(filter) => bail!(
                "The repository publishes no releases, and none of its font files is \
                 \"{}\". Its fonts: {}",
                filter.canonical(),
                list_programs(&self.names())
            ),
            None => bail!(
                "The repository publishes no releases, and its font files are all \
                 marked for other systems"
            ),
        }
    }

    /// The program names of the fonts this platform can use, sorted.
    fn names(&self) -> Vec<String> {
        let names: BTreeSet<String> = self
            .files
            .iter()
            .filter_map(|file| program_name(&file.name))
            .collect();
        names
            .into_iter()
            .filter(|name| {
                let filter = AssetFilter::Program(name.clone());
                find_compatible_asset(&self.files, Some(&filter)).is_some()
            })
            .collect()
    }
}

/// The version a remote font declares, fetched a range at a time.
async fn read_version(
    url: &str,
    request: &impl Fn(&str) -> reqwest::RequestBuilder,
) -> Result<String> {
    let mut reader = VersionReader::default();
    let mut step = reader.start();
    loop {
        let range = match step {
            Step::Version(version) => return Ok(version),
            Step::Read(range) => range,
        };
        let bytes = read_range(request(url), range).await?;
        step = reader
            .advance(&bytes)
            .with_context(|| format!("Could not read the version of {url}"))?;
    }
}

/// Bytes `range` of a file, or as many of them as the file has.
async fn read_range(request: reqwest::RequestBuilder, range: Range<u64>) -> Result<Vec<u8>> {
    let wanted = usize::try_from(range.end - range.start)?;
    let mut response = request
        .header(
            reqwest::header::RANGE,
            format!("bytes={}-{}", range.start, range.end - 1),
        )
        .send()
        .await
        .context("Failed to fetch the font file")?;

    let mut skip = match response.status() {
        reqwest::StatusCode::PARTIAL_CONTENT => 0,
        // A server that ignores ranges sends the whole file: skip to the
        // range, and stop reading once it is in.
        reqwest::StatusCode::OK => range.start,
        // The range starts past the end of the file.
        reqwest::StatusCode::RANGE_NOT_SATISFIABLE => return Ok(Vec::new()),
        status => bail!("Failed to fetch the font file: HTTP {status}"),
    };

    let mut bytes = Vec::new();
    while bytes.len() < wanted {
        let Some(chunk) = response
            .chunk()
            .await
            .context("Failed to read the font file")?
        else {
            break;
        };
        let skipped = usize::try_from(skip).unwrap_or(usize::MAX).min(chunk.len());
        skip -= skipped as u64;
        bytes.extend_from_slice(&chunk[skipped..]);
    }
    bytes.truncate(wanted);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: &str) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            kind: kind.to_string(),
            size: 2048,
        }
    }

    fn fonts(paths: &[&str]) -> Option<RepositoryFonts> {
        RepositoryFonts::from_tree(paths.iter().map(|path| entry(path, FILE_ENTRY)), |path| {
            Ok(format!("https://raw.invalid/{path}"))
        })
        .unwrap()
    }

    #[test]
    fn keeps_only_font_files() {
        let fonts = fonts(&[
            "C64-Keyboard.zip",
            "fonts/C64Keyboard-Regular.woff2",
            "fonts/C64Keyboard-Regular.otf",
            "fonts/C64Keyboard-Regular.ttf",
            "specimen.png",
        ])
        .unwrap();

        let names: Vec<&str> = fonts.files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(
            names,
            ["C64Keyboard-Regular.otf", "C64Keyboard-Regular.ttf"]
        );
        assert_eq!(
            fonts.files[1].browser_download_url,
            "https://raw.invalid/fonts/C64Keyboard-Regular.ttf"
        );
        assert_eq!(fonts.names(), ["c64keyboard-regular"]);
        // TrueType ranks first among the formats of one font.
        assert_eq!(fonts.pick(None).unwrap().name, "C64Keyboard-Regular.ttf");
    }

    #[test]
    fn a_repository_without_fonts_offers_nothing() {
        assert!(fonts(&["README.md", "src/main.rs"]).is_none());
        // A directory named like a font is not one.
        let directory =
            RepositoryFonts::from_tree([entry("Inter.ttf", "tree")], |path| Ok(path.to_string()))
                .unwrap();
        assert!(directory.is_none());
    }

    #[test]
    fn the_shallowest_copy_of_a_font_wins() {
        let fonts = fonts(&["docs/site/fonts/Inter.ttf", "fonts/Inter.ttf"]).unwrap();

        assert_eq!(
            fonts.pick(None).unwrap().browser_download_url,
            "https://raw.invalid/fonts/Inter.ttf"
        );
    }

    #[test]
    fn a_filter_chooses_among_several_fonts() {
        let fonts = fonts(&[
            "fonts/Inter-Bold.ttf",
            "fonts/Inter-Regular.ttf",
            "fonts/InterDisplay-Regular.otf",
        ])
        .unwrap();
        let program = |name: &str| AssetFilter::Program(name.to_string());

        assert_eq!(
            fonts.names(),
            ["inter-bold", "inter-regular", "interdisplay-regular"]
        );
        assert_eq!(
            fonts.pick(Some(&program("inter-regular"))).unwrap().name,
            "Inter-Regular.ttf"
        );

        let error = fonts
            .pick(Some(&program("inter-light")))
            .unwrap_err()
            .to_string();
        assert!(
            error.ends_with(
                "\"inter-light\". Its fonts: inter-bold, inter-regular, interdisplay-regular"
            ),
            "{error}"
        );
    }

    #[test]
    fn parses_both_forges_tree_listings() {
        let github = r#"{"sha": "abc", "truncated": false, "tree": [
            {"path": "fonts", "mode": "040000", "type": "tree", "sha": "d"},
            {"path": "fonts/A.ttf", "mode": "100644", "type": "blob", "sha": "e", "size": 23572}
        ]}"#;
        let forgejo = r#"{"sha": "abc", "truncated": true, "page": 1, "total_count": 7,
            "tree": [{"path": "A.otf", "mode": "100644", "type": "blob", "size": 4116, "sha": "f"}]}"#;

        let github: Tree = serde_json::from_str(github).unwrap();
        let forgejo: Tree = serde_json::from_str(forgejo).unwrap();

        assert_eq!(github.tree.len(), 2);
        assert_eq!(github.tree[1].size, 23572);
        assert_eq!(github.total_count, None);
        assert_eq!(forgejo.total_count, Some(7));
        assert_eq!(forgejo.tree[0].kind, FILE_ENTRY);
    }
}
