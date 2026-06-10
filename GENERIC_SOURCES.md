# Generic Web Page Sources

## Feasibility

Allowing users to add an arbitrary web page URL is feasible, but it should be treated as a best-effort source type rather than a guaranteed resolver for every site.

Obtainintosh already has most of the backend shape needed for this. Apps store a `source_type` and `source_url`, release resolution happens in source adapters, and the download flow only needs a resolved `Release` containing a version, download URL, file name, and optional metadata. A generic web page source can fit into that model as another source type, for example `webpage`.

## What Can Work Reliably

A generic web page adapter can handle many static vendor download pages by:

- Fetching the page HTML from the user-provided URL.
- Extracting links from `<a href>` attributes.
- Resolving relative links against the page URL.
- Filtering for macOS download artifacts such as `.dmg`, `.pkg`, `.zip`, `.tar.gz`, and `.app.tar.gz`.
- Preferring Apple Silicon links containing terms like `arm64`, `aarch64`, `apple-silicon`, `silicon`, `macos-arm64`, or `darwin-arm64`.
- Falling back to universal macOS builds when no explicit Apple Silicon build exists.
- Extracting versions from filenames, URLs, link text, or nearby page text using semver-like patterns.
- Choosing the highest versioned compatible candidate as the latest release.

This would support pages that expose direct download links in their HTML and use reasonably descriptive filenames or link text.

## Limits And Risks

This cannot be fully reliable for every random web page. Common failure cases include:

- Pages rendered entirely by JavaScript, where the download links are not present in the initial HTML.
- Generic download buttons such as `/download/mac`, where the real artifact URL is generated server-side or after redirects.
- Pages with no visible version number.
- Pages containing multiple products, beta channels, nightly builds, or legacy versions.
- Vendor-specific naming that does not include useful platform terms such as `mac`, `macos`, `darwin`, `arm64`, or `universal`.
- Pages where the newest version is listed in text but the actual download link does not contain the same version.

The feature should therefore report clear errors when it cannot resolve an unambiguous Apple Silicon macOS download link.

## Suggested Implementation

1. Add a new Rust source type, for example `SourceType::WebPage`.
2. Update `detect_source_type` so unknown `http://` and `https://` URLs are accepted as web page sources after GitHub and GitLab checks.
3. Add a `WebPageAdapter` in `src-tauri/src/sources.rs` with a `get_latest_release(url)` method.
4. Fetch the page with `reqwest` using the existing app user agent pattern.
5. Parse HTML and collect candidate links.
6. Resolve relative links against the page URL.
7. Score candidates by platform, architecture, file type, and version.
8. Return the highest-scoring latest candidate as a `Release`.
9. Wire `SourceType::WebPage` into `check_for_updates` and `download_and_install`.
10. Update frontend types so `SourceType` includes `webpage`.
11. Update add/edit dialog copy from `GitHub URL` to `Source URL`.
12. Add tests for version extraction, Apple Silicon preference, universal fallback, relative URL resolution, and ambiguous page handling.

## Candidate Scoring

A practical MVP should rank candidates roughly like this:

- Prefer macOS-specific links over generic archives.
- Prefer Apple Silicon over Intel.
- Prefer universal builds over Intel-only builds.
- Reject obvious Windows/Linux builds.
- Prefer installer-friendly artifacts in this order: `.dmg`, `.pkg`, `.app.tar.gz`, `.tar.gz`, `.zip`.
- Prefer stable releases over beta, alpha, rc, nightly, canary, or dev builds unless no stable candidate exists.
- Prefer the highest parsed version among otherwise comparable candidates.

## User Experience

The UI should avoid promising that every arbitrary URL will work. Good wording would be:

- Label: `Source URL`
- Placeholder: `https://github.com/owner/repo or https://example.com/downloads`
- Error: `Could not find an Apple Silicon macOS download link on this page.`

## Recommendation

Build this as an experimental generic source resolver. It should work automatically for simple static download pages, but fail explicitly when a page is too dynamic, ambiguous, or does not expose a direct macOS Apple Silicon artifact.
