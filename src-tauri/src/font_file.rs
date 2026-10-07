//! Reads the version a font file declares. Fonts carry no bundle metadata:
//! the version lives in the `name` table, which the table directory at the
//! start of the file points at.
//!
//! ```text
//!   ┌───────────────────┐
//!   │ table directory   │── "name" at offset 9184, 412 bytes ──┐
//!   ├───────────────────┤                                      │
//!   │ glyf, cmap, …     │                                      │
//!   │ name              │◀─────────────────────────────────────┘
//!   └───────────────────┘   nameID 5: "Version 1.107; ttfautohint"
//! ```
//!
//! The reader does no I/O. It names the byte range it needs next and is fed
//! those bytes, so a font in a repository is read with a few small range
//! requests instead of a download, and an installed one with a few seeks.
//! A font collection (`.ttc`) adds one step: its header points at the table
//! directory of the first font. A version string without a number
//! ("Version ") adds another: the revision in the `head` table stands in.

use anyhow::{bail, Context, Result};
use std::ops::Range;

/// Installable desktop font formats, most preferred first. Web fonts
/// (`.woff`, `.woff2`) are left out: desktop systems do not install them.
pub const FONT_EXTENSIONS: [&str; 4] = [".ttf", ".otf", ".ttc", ".otc"];

/// The first read: enough for a collection header, or for a table
/// directory of up to 255 tables (12 + 16 × 255 bytes).
const HEADER_LEN: u64 = 4096;

/// More than any real `name` or `head` table needs, so a corrupt directory
/// cannot make the reader fetch gigabytes.
const MAX_TABLE_LEN: u64 = 1024 * 1024;

const NAME_TABLE_TAG: &[u8; 4] = b"name";
const HEAD_TABLE_TAG: &[u8; 4] = b"head";
const COLLECTION_TAG: &[u8; 4] = b"ttcf";
/// The sfnt versions of TrueType (`0x00010000`, Apple's `true`) and of
/// CFF-based OpenType (`OTTO`) fonts.
const SFNT_VERSIONS: [&[u8; 4]; 3] = [b"\x00\x01\x00\x00", b"OTTO", b"true"];

const VERSION_NAME_ID: u16 = 5;
const UNICODE_PLATFORM: u16 = 0;
const MACINTOSH_PLATFORM: u16 = 1;
const WINDOWS_PLATFORM: u16 = 3;
/// Platforms whose `name` strings are UTF-16BE, most preferred first.
/// Macintosh strings are single-byte and only a fallback.
const UTF16_PLATFORMS: [u16; 2] = [WINDOWS_PLATFORM, UNICODE_PLATFORM];

/// Where the `head` table keeps `fontRevision`, a 16.16 fixed-point number.
const FONT_REVISION_OFFSET: usize = 4;
const FIXED_POINT_ONE: f64 = 65536.0;

const NO_VERSION: &str = "The font declares no version";

pub fn is_font_file(file_name: &str) -> bool {
    let name = file_name.to_ascii_lowercase();
    FONT_EXTENSIONS
        .iter()
        .any(|extension| name.ends_with(extension))
}

/// What the reader needs next.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Feed these bytes of the file to `VersionReader::advance`. A read that
    /// reaches past the end of the file returns what there is.
    Read(Range<u64>),
    /// The version, without its "Version" prefix: `1.107`.
    Version(String),
}

/// Steps through a font file to its declared version:
///
/// ```ignore
/// let mut reader = VersionReader::default();
/// let mut step = reader.start();
/// while let Step::Read(range) = step {
///     step = reader.advance(&read(range)?)?;
/// }
/// ```
#[derive(Debug, Default)]
pub struct VersionReader {
    state: State,
}

#[derive(Debug, Default)]
enum State {
    #[default]
    Header,
    Directory,
    /// Reading the `name` table, with the `head` table as the fallback.
    NameTable {
        head: Option<Range<u64>>,
    },
    HeadTable,
}

impl VersionReader {
    pub fn start(&self) -> Step {
        Step::Read(0..HEADER_LEN)
    }

    pub fn advance(&mut self, bytes: &[u8]) -> Result<Step> {
        match std::mem::take(&mut self.state) {
            State::Header if bytes.starts_with(COLLECTION_TAG) => {
                // A collection's header lists the offset of each font's table
                // directory; the first font speaks for the file.
                let offset = u64::from(read_u32(bytes, 12).context("Truncated font collection")?);
                self.state = State::Directory;
                Ok(Step::Read(offset..offset + HEADER_LEN))
            }
            State::Header | State::Directory => {
                let VersionTables { name, head } = version_tables(bytes)?;
                let Some(name) = name else {
                    return self.read_head(head);
                };
                self.state = State::NameTable { head };
                Ok(Step::Read(name))
            }
            State::NameTable { head } => match declared_version(bytes) {
                Some(version) => Ok(Step::Version(version)),
                None => self.read_head(head),
            },
            State::HeadTable => Ok(Step::Version(font_revision(bytes).context(NO_VERSION)?)),
        }
    }

    fn read_head(&mut self, head: Option<Range<u64>>) -> Result<Step> {
        let head = head.context(NO_VERSION)?;
        self.state = State::HeadTable;
        Ok(Step::Read(head))
    }
}

/// Where the tables that hold a font's version sit.
struct VersionTables {
    name: Option<Range<u64>>,
    head: Option<Range<u64>>,
}

/// Finds the version tables in a table directory. Table offsets count from
/// the start of the file, in a collection too.
fn version_tables(directory: &[u8]) -> Result<VersionTables> {
    let sfnt_version = directory.get(..4).context("Not a font file")?;
    if !SFNT_VERSIONS.iter().any(|version| sfnt_version == *version) {
        bail!("Not a font file");
    }

    let mut tables = VersionTables {
        name: None,
        head: None,
    };
    let table_count = read_u16(directory, 4).context("Truncated font header")?;
    for index in 0..usize::from(table_count) {
        let record = 12 + 16 * index;
        // A directory reaching past the first read ends there; the tables
        // found before that still count.
        let (Some(tag), Some(offset), Some(length)) = (
            directory.get(record..record + 4),
            read_u32(directory, record + 8),
            read_u32(directory, record + 12),
        ) else {
            break;
        };
        let table = match tag {
            tag if tag == NAME_TABLE_TAG => &mut tables.name,
            tag if tag == HEAD_TABLE_TAG => &mut tables.head,
            _ => continue,
        };

        let (offset, length) = (u64::from(offset), u64::from(length));
        if length > MAX_TABLE_LEN {
            bail!("The font's tables are implausibly large");
        }
        // An empty table holds no version.
        if length > 0 {
            *table = Some(offset..offset + length);
        }
    }

    Ok(tables)
}

/// The version string of a `name` table, preferring the Unicode encodings
/// over the legacy Macintosh one. None when no version string has a number.
fn declared_version(table: &[u8]) -> Option<String> {
    let record_count = read_u16(table, 2)?;
    let strings_offset = usize::from(read_u16(table, 4)?);

    let mut utf16 = Vec::new();
    let mut macintosh = Vec::new();
    for index in 0..usize::from(record_count) {
        let record = 6 + 12 * index;
        let field = |position: usize| read_u16(table, record + 2 * position);
        let (Some(platform), Some(name_id), Some(length), Some(offset)) =
            (field(0), field(3), field(4), field(5))
        else {
            break;
        };
        if name_id != VERSION_NAME_ID {
            continue;
        }

        let start = strings_offset + usize::from(offset);
        let Some(bytes) = table.get(start..start + usize::from(length)) else {
            continue;
        };
        if let Some(rank) = UTF16_PLATFORMS.iter().position(|id| *id == platform) {
            utf16.push((rank, decode_utf16_be(bytes)));
        } else if platform == MACINTOSH_PLATFORM {
            macintosh.push(String::from_utf8_lossy(bytes).into_owned());
        }
    }

    utf16.sort_by_key(|(rank, _)| *rank);
    utf16
        .into_iter()
        .map(|(_, text)| text)
        .chain(macintosh)
        .find_map(|text| version_number(&text))
}

/// The `head` table's revision. Font tools derive it from the version, which
/// it stores inexactly: 1.107 as 1.10699…, so three decimals restore it.
fn font_revision(head: &[u8]) -> Option<String> {
    let revision = i32::from_be_bytes(
        head.get(FONT_REVISION_OFFSET..FONT_REVISION_OFFSET + 4)?
            .try_into()
            .ok()?,
    );
    (revision > 0).then(|| format!("{:.3}", f64::from(revision) / FIXED_POINT_ONE))
}

/// The version number of a `name` table version string, which by convention
/// reads "Version 1.107" and may continue with build notes:
/// `Version 2.304; ttfautohint (v1.8.4)` → `2.304`.
fn version_number(text: &str) -> Option<String> {
    let text = text.trim();
    let text = match text.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("version") => &text[7..],
        _ => text,
    };
    let text = text.trim_start();
    let text = text.strip_prefix(['v', 'V']).unwrap_or(text);

    let number: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let number = number.trim_end_matches('.');
    number
        .starts_with(|c: char| c.is_ascii_digit())
        .then(|| number.to_string())
}

fn decode_utf16_be(bytes: &[u8]) -> String {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal font: a table directory with a `head` stand-in and a `name`
    /// table holding the given version strings as (platform, text) records.
    pub(crate) fn font_bytes(versions: &[(u16, &str)]) -> Vec<u8> {
        sfnt_at(0, b"\x00\x01\x00\x00", versions, 0)
    }

    fn sfnt_at(
        base: usize,
        sfnt_version: &[u8; 4],
        versions: &[(u16, &str)],
        revision: i32,
    ) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut records = Vec::new();
        for (platform, text) in versions {
            let encoded: Vec<u8> = if *platform == MACINTOSH_PLATFORM {
                text.as_bytes().to_vec()
            } else {
                text.encode_utf16().flat_map(u16::to_be_bytes).collect()
            };
            for value in [*platform, 1, 0x409, VERSION_NAME_ID] {
                records.extend(value.to_be_bytes());
            }
            records.extend((encoded.len() as u16).to_be_bytes());
            records.extend((strings.len() as u16).to_be_bytes());
            strings.extend(encoded);
        }
        let mut name = Vec::new();
        name.extend(0u16.to_be_bytes());
        name.extend((versions.len() as u16).to_be_bytes());
        name.extend(((6 + records.len()) as u16).to_be_bytes());
        name.extend(records);
        name.extend(strings);

        let mut head = vec![0u8; 54];
        head[FONT_REVISION_OFFSET..FONT_REVISION_OFFSET + 4]
            .copy_from_slice(&revision.to_be_bytes());
        let tables_start = base + 12 + 16 * 2;
        let name_offset = tables_start + head.len();

        let mut font = sfnt_version.to_vec();
        font.extend(2u16.to_be_bytes());
        font.extend([0u8; 6]);
        for (tag, offset, length) in [
            (b"head", tables_start, head.len()),
            (b"name", name_offset, name.len()),
        ] {
            font.extend(tag);
            font.extend([0u8; 4]);
            font.extend((offset as u32).to_be_bytes());
            font.extend((length as u32).to_be_bytes());
        }
        font.extend(head);
        font.extend(name);
        font
    }

    /// Runs the reader over in-memory bytes, recording the reads it asks for.
    fn read_version(font: &[u8]) -> (Result<String>, Vec<Range<u64>>) {
        let mut reader = VersionReader::default();
        let mut step = reader.start();
        let mut reads = Vec::new();
        loop {
            let range = match step {
                Step::Version(version) => return (Ok(version), reads),
                Step::Read(range) => range,
            };
            reads.push(range.clone());
            let end = (range.end as usize).min(font.len());
            let start = (range.start as usize).min(end);
            step = match reader.advance(&font[start..end]) {
                Ok(step) => step,
                Err(error) => return (Err(error), reads),
            };
        }
    }

    #[test]
    fn reads_the_declared_version_in_two_reads() {
        let font = font_bytes(&[(WINDOWS_PLATFORM, "Version 1.107")]);

        let (version, reads) = read_version(&font);

        assert_eq!(version.unwrap(), "1.107");
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[0], 0..HEADER_LEN);
    }

    #[test]
    fn reads_cff_and_apple_truetype_fonts() {
        for sfnt_version in [b"OTTO", b"true"] {
            let font = sfnt_at(0, sfnt_version, &[(WINDOWS_PLATFORM, "Version 2.0")], 0);
            assert_eq!(read_version(&font).0.unwrap(), "2.0");
        }
    }

    #[test]
    fn reads_the_first_font_of_a_collection() {
        let directory_offset = 16;
        let mut collection = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x01".to_vec();
        collection.extend((directory_offset as u32).to_be_bytes());
        collection.extend(sfnt_at(
            directory_offset,
            b"\x00\x01\x00\x00",
            &[(WINDOWS_PLATFORM, "Version 3.1")],
            0,
        ));

        let (version, reads) = read_version(&collection);

        assert_eq!(version.unwrap(), "3.1");
        assert_eq!(reads.len(), 3);
        assert_eq!(reads[1].start, directory_offset as u64);
    }

    #[test]
    fn prefers_unicode_strings_and_falls_back_to_macintosh_ones() {
        let font = font_bytes(&[
            (MACINTOSH_PLATFORM, "Version 1.0"),
            (UNICODE_PLATFORM, "Version 1.1"),
            (WINDOWS_PLATFORM, "Version 1.2"),
        ]);
        assert_eq!(read_version(&font).0.unwrap(), "1.2");

        let font = font_bytes(&[(MACINTOSH_PLATFORM, "Version 1.0")]);
        assert_eq!(read_version(&font).0.unwrap(), "1.0");
    }

    #[test]
    fn version_strings_lose_their_prefix_and_build_notes() {
        for (text, expected) in [
            ("Version 1.107", Some("1.107")),
            ("Version 2.304; ttfautohint (v1.8.4.7-5d5b)", Some("2.304")),
            ("Version 4.001;git-0a5106e0b", Some("4.001")),
            ("version 1.00 December 1, 2020", Some("1.00")),
            ("4.001", Some("4.001")),
            ("v1.2.", Some("1.2")),
            ("Version Beta", None),
            ("", None),
        ] {
            assert_eq!(version_number(text).as_deref(), expected, "{text}");
        }
    }

    #[test]
    fn rejects_files_that_are_not_fonts() {
        for bytes in [
            b"version https://git-lfs.github.com/spec/v1\n".as_slice(),
            b"<!DOCTYPE html>",
            b"wOF2\x00\x01\x00\x00",
            b"",
        ] {
            let (version, _) = read_version(bytes);
            assert_eq!(version.unwrap_err().to_string(), "Not a font file");
        }
    }

    #[test]
    fn a_version_string_without_a_number_falls_back_to_the_revision() {
        // 1.107 as font tools store it: 72548 / 65536 = 1.10699…
        for (revision, expected) in [(72548, "1.107"), (65536, "1.000")] {
            let font = sfnt_at(0, b"OTTO", &[(WINDOWS_PLATFORM, "Version  ")], revision);

            let (version, reads) = read_version(&font);

            assert_eq!(version.unwrap(), expected);
            assert_eq!(reads.len(), 3);
        }
    }

    /// Where the `name` table's length sits in a `sfnt_at` font: the second
    /// record of the directory, twelve bytes in.
    const NAME_LENGTH_AT: usize = 12 + 16 + 12;

    #[test]
    fn an_empty_name_table_falls_back_to_the_revision() {
        let mut font = sfnt_at(0, b"OTTO", &[(WINDOWS_PLATFORM, "Version 1.0")], 65536);
        font[NAME_LENGTH_AT..NAME_LENGTH_AT + 4].copy_from_slice(&0u32.to_be_bytes());

        let (version, reads) = read_version(&font);

        assert_eq!(version.unwrap(), "1.000");
        assert_eq!(reads.len(), 2);
    }

    #[test]
    fn a_directory_longer_than_the_first_read_keeps_the_tables_found() {
        let mut font = font_bytes(&[(WINDOWS_PLATFORM, "Version 1.2")]);
        // More tables than the first read could ever hold.
        font[4..6].copy_from_slice(&300u16.to_be_bytes());

        assert_eq!(read_version(&font).0.unwrap(), "1.2");
    }

    #[test]
    fn a_font_without_a_version_is_an_error() {
        let font = font_bytes(&[(WINDOWS_PLATFORM, "Release candidate")]);
        assert_eq!(
            read_version(&font).0.unwrap_err().to_string(),
            "The font declares no version"
        );
    }

    #[test]
    fn recognises_installable_font_files() {
        assert!(is_font_file("C64Keyboard-Regular.ttf"));
        assert!(is_font_file("fonts/Inter.OTF"));
        assert!(is_font_file("Sarasa.ttc"));
        assert!(!is_font_file("Inter.woff2"));
        assert!(!is_font_file("ttf.zip"));
    }
}
