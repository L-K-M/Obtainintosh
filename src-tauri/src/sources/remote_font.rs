//! Reads the version a font on a forge declares, with range requests: the
//! table directory, then the table holding the version. A font of a few
//! megabytes costs a few kilobytes to check.

use crate::font_file::{Step, VersionReader};
use anyhow::{bail, Context, Result};
use std::ops::Range;

/// The version a remote font declares, fetched a range at a time.
pub(super) async fn read_version(
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
    if range.is_empty() {
        return Ok(Vec::new());
    }

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
