use crate::models::{Release, SourceType};
use anyhow::{Context, Result};
use serde::Deserialize;

pub const USER_AGENT: &str = concat!("Obtainintosh/", env!("CARGO_PKG_VERSION"));

/// Canonical form for "is this the same repository?" comparisons: lowercased
/// first (so `.GIT` trims like `.git`), query string and fragment dropped
/// (like `parse_github_url` does), then stripped of a trailing slash and
/// `.git`, with `http://` folded into `https://` and a `www.` host prefix
/// dropped. Shared by `Storage::add_app`'s dedupe and `updates::is_self_app`
/// so the two checks can't drift apart.
pub(crate) fn normalize_repo_url(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    let base = lower.split(['?', '#']).next().unwrap_or("");
    let trimmed = base
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    match rest {
        Some(rest) => format!("https://{}", rest.strip_prefix("www.").unwrap_or(rest)),
        None => trimmed.to_string(),
    }
}

/// Shared HTTP client for API calls: connection reuse, a request timeout so a
/// hung connection can't leave the UI stuck in "Checking..." forever, and the
/// default User-Agent GitHub requires — set here once so every caller gets it.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .expect("failed to build HTTP client")
    })
}

/// A release as both supported forges report it. Forgejo's API is
/// Gitea-compatible and names these fields exactly like GitHub's, so one type
/// deserializes both and the asset picker below can stay shared. Fields the
/// two disagree on are simply ignored by serde.
#[derive(Debug, Deserialize)]
struct ForgeRelease {
    tag_name: String,
    body: Option<String>,
    assets: Vec<ReleaseAsset>,
    #[serde(default)]
    draft: bool,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

impl ForgeRelease {
    /// Turns the forge's release into ours, picking the asset this Mac can use.
    fn into_release(self) -> Result<Release> {
        let asset = find_macos_asset(&self.assets).context("No macOS-compatible asset found")?;

        Ok(Release {
            version: clean_version_tag(&self.tag_name),
            download_url: asset.browser_download_url.clone(),
            file_name: asset.name.clone(),
            file_size: Some(asset.size),
            checksum: None,
            release_notes: self.body,
        })
    }
}

pub struct GitHubAdapter {
    token: Option<String>,
}

#[derive(Clone, Copy)]
enum MacArch {
    AppleSilicon,
    X86_64,
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!("macOS asset selection supports only aarch64 and x86_64 targets");

impl GitHubAdapter {
    pub fn new(token: Option<String>) -> Self {
        Self { token }
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let mut request = http_client()
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");

        if let Some(token) = &self.token {
            if !token.trim().is_empty() {
                request = request.header("Authorization", format!("Bearer {}", token));
            }
        }

        request
    }

    pub async fn get_latest_release(&self, repo_url: &str) -> Result<Release> {
        let (owner, repo) = Self::parse_github_url(repo_url)?;

        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            owner, repo
        );

        let response = self
            .get(&api_url)
            .send()
            .await
            .context("Failed to fetch GitHub release")?;

        let release: ForgeRelease = if response.status() == reqwest::StatusCode::NOT_FOUND {
            // `releases/latest` 404s when a repo only has pre-releases (common for
            // nightly/continuous builds), so fall back to the release list.
            self.get_newest_prerelease(&owner, &repo).await?
        } else if !response.status().is_success() {
            anyhow::bail!("GitHub API error: {}", response.status());
        } else {
            response
                .json()
                .await
                .context("Failed to parse GitHub release")?
        };

        release.into_release()
    }

    async fn get_newest_prerelease(&self, owner: &str, repo: &str) -> Result<ForgeRelease> {
        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases?per_page=10",
            owner, repo
        );

        let response = self
            .get(&api_url)
            .send()
            .await
            .context("Failed to fetch GitHub releases")?;

        if !response.status().is_success() {
            anyhow::bail!("GitHub API error: {}", response.status());
        }

        let releases: Vec<ForgeRelease> = response
            .json()
            .await
            .context("Failed to parse GitHub releases")?;

        releases
            .into_iter()
            .find(|r| !r.draft)
            .context("No releases found for this repository")
    }

    fn parse_github_url(url: &str) -> Result<(String, String)> {
        let url = url.trim();
        let without_scheme = url.split("://").last().unwrap_or(url);
        let mut parts = without_scheme.split('/').filter(|s| !s.is_empty());

        let host = parts.next().unwrap_or_default();
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com")
        {
            anyhow::bail!("Invalid GitHub URL: expected github.com/<owner>/<repo>");
        }

        // Ignore anything past owner/repo (e.g. /releases, /tree/main) as well as
        // query strings, fragments, and a trailing .git.
        let strip = |s: &str| s.split(['?', '#']).next().unwrap_or("").to_string();
        let owner = strip(
            parts
                .next()
                .context("GitHub URL is missing the repository owner")?,
        );
        let repo = strip(
            parts
                .next()
                .context("GitHub URL is missing the repository name")?,
        );
        let repo = repo.trim_end_matches(".git").to_string();

        if owner.is_empty() || repo.is_empty() {
            anyhow::bail!("Invalid GitHub URL: expected github.com/<owner>/<repo>");
        }

        Ok((owner, repo))
    }
}

/// Forgejo is self-hosted, so unlike GitHub there is no fixed host to check
/// against and no anonymous access to assume: a private instance rejects
/// unauthenticated API reads outright. Credentials therefore travel with the
/// tracked app rather than living in global settings.
pub struct ForgejoAdapter {
    credentials: ForgeCredentials,
}

impl ForgejoAdapter {
    pub fn new(credentials: ForgeCredentials) -> Self {
        Self { credentials }
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.credentials.authorize(
            http_client().get(url).header("Accept", "application/json"),
            url,
        )
    }

    pub async fn get_latest_release(&self, repo_url: &str) -> Result<Release> {
        let (base_url, owner, repo) = parse_forgejo_url(repo_url)?;
        let releases_url = format!("{}/api/v1/repos/{}/{}/releases", base_url, owner, repo);

        let response = self
            .get(&format!("{}/latest", releases_url))
            .send()
            .await
            .context("Failed to fetch Forgejo release")?;

        let release: ForgeRelease = if response.status() == reqwest::StatusCode::NOT_FOUND {
            // Forgejo's `releases/latest` returns the newest non-draft,
            // non-prerelease release and 404s when there is none — the same
            // shape as GitHub, so fall back to the release list the same way.
            // A repository the credentials cannot see also 404s here, which
            // `get_newest_prerelease` reports on.
            self.get_newest_prerelease(&releases_url).await?
        } else if !response.status().is_success() {
            anyhow::bail!("{}", forgejo_error(response.status()));
        } else {
            response
                .json()
                .await
                .context("Failed to parse Forgejo release")?
        };

        release.into_release()
    }

    async fn get_newest_prerelease(&self, releases_url: &str) -> Result<ForgeRelease> {
        let response = self
            .get(&format!("{}?limit=10", releases_url))
            .send()
            .await
            .context("Failed to fetch Forgejo releases")?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!(
                "Repository not found on this Forgejo instance. A private repository \
                 also looks like this when the username or application key is missing \
                 or wrong."
            );
        }
        if !response.status().is_success() {
            anyhow::bail!("{}", forgejo_error(response.status()));
        }

        let releases: Vec<ForgeRelease> = response
            .json()
            .await
            .context("Failed to parse Forgejo releases")?;

        releases
            .into_iter()
            .find(|r| !r.draft)
            .context("No releases found for this repository")
    }

    /// The credentials to reuse for the asset download that follows a release
    /// lookup, pinned to the instance the app is tracked from. Assets on a
    /// private repository are served by the instance's web routes, which need
    /// the same authentication the API call did.
    pub fn download_auth(&self, repo_url: &str) -> Result<DownloadAuth> {
        let (base_url, _, _) = parse_forgejo_url(repo_url)?;
        Ok(DownloadAuth {
            credentials: self.credentials.clone(),
            origin: base_url,
        })
    }
}

/// Credentials for a self-hosted forge instance: the account name plus the
/// application key generated under Settings → Applications.
///
/// `Debug` is hand-written so the key is redacted — `DownloadAuth` and anything
/// else holding these inherits that. It is kept rather than dropped because
/// `assert_eq!` needs it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ForgeCredentials {
    username: Option<String>,
    token: Option<String>,
}

impl std::fmt::Debug for ForgeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForgeCredentials")
            .field("username", &self.username)
            .field(
                "token",
                &self.token.as_ref().map(|_| crate::models::REDACTED),
            )
            .finish()
    }
}

impl ForgeCredentials {
    /// Blank and whitespace-only entries are dropped, so a half-filled form
    /// behaves like no credentials at all instead of sending an empty key.
    pub fn new(username: Option<String>, token: Option<String>) -> Self {
        fn clean(value: Option<String>) -> Option<String> {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        }

        Self {
            username: clean(username),
            token: clean(token),
        }
    }

    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Adds the `Authorization` header Forgejo expects, if there is anything to
    /// send. `url` is the request's own URL, used only to warn about the
    /// transport.
    ///
    /// With a username, HTTP Basic carrying the application key as the
    /// password: Forgejo reads an access token out of the Basic password, and
    /// Basic is accepted on the plain web routes that serve release assets —
    /// not just on `/api/v1` — which is what a private repository's download
    /// needs. Without one, the API's own `Authorization: token <key>` scheme.
    fn authorize(&self, request: reqwest::RequestBuilder, url: &str) -> reqwest::RequestBuilder {
        // A username on its own authenticates nothing; send it to nobody.
        let Some(token) = &self.token else {
            return request;
        };

        // Both schemes carry the key in a header a passive observer can read.
        // `http://` instances are supported on purpose — a Forgejo box on the
        // LAN often has no certificate — so this warns rather than refuses.
        if is_plaintext_http(url) {
            log::warn!(
                "Sending Forgejo credentials over plain HTTP to {}; anyone on the network \
                 path can read the application key",
                url
            );
        }

        match &self.username {
            Some(username) => request.basic_auth(username, Some(token)),
            None => request.header("Authorization", format!("token {}", token)),
        }
    }
}

fn is_plaintext_http(url: &str) -> bool {
    url.trim_start().len() >= 7 && url.trim_start()[..7].eq_ignore_ascii_case("http://")
}

/// Credentials plus the instance origin they belong to. Asset URLs come from
/// the forge's own API response, but they are still data from the network:
/// scoping the credentials to the origin the user typed keeps a redirected or
/// off-site asset URL from collecting them.
#[derive(Debug, Clone)]
pub struct DownloadAuth {
    credentials: ForgeCredentials,
    origin: String,
}

impl DownloadAuth {
    pub fn authorize(
        &self,
        request: reqwest::RequestBuilder,
        url: &str,
    ) -> reqwest::RequestBuilder {
        if same_origin(url, &self.origin) {
            self.credentials.authorize(request, url)
        } else {
            log::warn!("Not sending instance credentials to a different origin: {url}");
            request
        }
    }
}

/// Splits a Forgejo repository URL into the instance base URL and the
/// `owner`/`repo` pair. The host is whatever the user self-hosts, so only the
/// `<owner>/<repo>` shape of the path is fixed; anything past the repository
/// (`/releases`, `/src/branch/main`) is ignored, as in `parse_github_url`. A
/// URL without a scheme is assumed to be https. An instance served under a URL
/// path prefix is not supported — the repository must be the first two path
/// segments.
fn parse_forgejo_url(url: &str) -> Result<(String, String, String)> {
    let url = url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("https".to_string(), url),
    };
    if scheme != "https" && scheme != "http" {
        anyhow::bail!("Unsupported URL scheme for a Forgejo instance: {}", scheme);
    }

    let strip = |s: &str| s.split(['?', '#']).next().unwrap_or("").to_string();
    let mut parts = rest.split('/').filter(|s| !s.is_empty());

    // Hosts are case-insensitive; owner and repo keep the case the user typed.
    let authority = strip(parts.next().unwrap_or_default()).to_ascii_lowercase();
    // Drop any `user:password@` prefix rather than folding it into the host.
    let host = match authority.rsplit_once('@') {
        Some((_, host)) => host.to_string(),
        None => authority,
    };

    let owner = strip(
        parts
            .next()
            .context("Forgejo URL is missing the repository owner")?,
    );
    let repo = strip(
        parts
            .next()
            .context("Forgejo URL is missing the repository name")?,
    );
    let repo = repo.trim_end_matches(".git").to_string();

    if host.is_empty() || !host.contains(|c: char| c.is_ascii_alphanumeric()) {
        anyhow::bail!("Invalid Forgejo URL: expected <host>/<owner>/<repo>");
    }
    if owner.is_empty() || repo.is_empty() {
        anyhow::bail!("Invalid Forgejo URL: expected <host>/<owner>/<repo>");
    }

    Ok((format!("{}://{}", scheme, host), owner, repo))
}

/// Turns the status codes a Forgejo instance answers with into advice, since
/// the usual cause is a credential problem rather than a broken repository.
fn forgejo_error(status: reqwest::StatusCode) -> String {
    match status {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => format!(
            "Forgejo rejected the credentials (HTTP {}). Check the username and \
             application key, and that the key has read access to the repository.",
            status.as_u16()
        ),
        _ => format!("Forgejo API error: {}", status),
    }
}

/// Whether two URLs address the same origin (scheme, host, port), treating a
/// missing scheme as https so a stored `codeberg.org/owner/repo` still matches
/// its own asset URLs.
fn same_origin(a: &str, b: &str) -> bool {
    fn origin(url: &str) -> Option<(String, String, Option<u16>)> {
        let url = url.trim();
        let absolute = if url.contains("://") {
            url.to_string()
        } else {
            format!("https://{}", url)
        };
        let parsed = reqwest::Url::parse(&absolute).ok()?;
        Some((
            parsed.scheme().to_ascii_lowercase(),
            parsed.host_str()?.to_ascii_lowercase(),
            parsed.port_or_known_default(),
        ))
    }

    match (origin(a), origin(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn find_macos_asset(assets: &[ReleaseAsset]) -> Option<&ReleaseAsset> {
    let target_arch = if cfg!(target_arch = "aarch64") {
        MacArch::AppleSilicon
    } else {
        MacArch::X86_64
    };

    find_macos_asset_for_arch(assets, target_arch)
}

fn find_macos_asset_for_arch(
    assets: &[ReleaseAsset],
    target_arch: MacArch,
) -> Option<&ReleaseAsset> {
    log::debug!("Finding macOS asset from {} candidates", assets.len());
    let extensions = [".dmg", ".pkg", ".app.tar.gz", ".tar.gz", ".zip"];
    let macos_markers = ["mac", "macos", "macosx", "darwin", "osx"];
    let other_os_markers = [
        "windows", "win", "win32", "win64", "linux", "linux32", "linux64", "ubuntu", "debian",
        "android", "freebsd", "openbsd", "netbsd", "solaris", "ios", "tvos",
    ];

    let selected = assets
        .iter()
        .filter_map(|asset| {
            let name = asset.name.to_ascii_lowercase();
            let package_rank = extensions.iter().position(|ext| name.ends_with(ext))?;

            if other_os_markers
                .iter()
                .any(|marker| has_name_marker(&name, marker))
            {
                return None;
            }

            let has_macos_marker = macos_markers
                .iter()
                .any(|marker| has_name_marker(&name, marker));
            let generic_archive = package_rank >= 3;
            if generic_archive && !has_macos_marker {
                return None;
            }

            // They are separate markers because the boundary check intentionally
            // does not match "universal" within "universal2".
            let universal = ["universal", "universal2"]
                .iter()
                .any(|marker| has_name_marker(&name, marker));
            let arm64 = ["arm64", "aarch64"]
                .iter()
                .any(|marker| has_name_marker(&name, marker));
            let intel64 = ["x86_64", "x86-64", "amd64", "x64", "intel"]
                .iter()
                .any(|marker| has_name_marker(&name, marker));
            let unsupported_cpu = [
                "armv7", "armv6", "armhf", "i386", "i486", "i586", "i686", "i786", "x86_32",
                "x86-32", "powerpc", "ppc64", "riscv64",
            ]
            .iter()
            .any(|marker| has_name_marker(&name, marker));

            if unsupported_cpu
                || matches!(target_arch, MacArch::X86_64) && arm64 && !intel64 && !universal
            {
                return None;
            }

            // Intel-only builds remain a last-resort option on Apple Silicon because
            // Rosetta 2 can run them. The reverse is not possible on x86_64 Macs.
            let architecture_rank = match target_arch {
                MacArch::AppleSilicon if universal => 0,
                MacArch::AppleSilicon if arm64 => 1,
                MacArch::AppleSilicon if !intel64 => 2,
                MacArch::AppleSilicon => 3,
                MacArch::X86_64 if universal => 0,
                MacArch::X86_64 if intel64 => 1,
                MacArch::X86_64 => 2,
            };

            Some((asset, (architecture_rank, package_rank)))
        })
        .min_by(|(asset_a, rank_a), (asset_b, rank_b)| {
            rank_a
                .cmp(rank_b)
                .then_with(|| asset_a.name.cmp(&asset_b.name))
        });

    match selected {
        Some((asset, (3, _))) if matches!(target_arch, MacArch::AppleSilicon) => {
            log::warn!(
                "Selected Intel-only macOS asset for Apple Silicon; Rosetta 2 is required: {}",
                asset.name
            );
            Some(asset)
        }
        Some((asset, _)) => {
            log::debug!("Selected compatible macOS asset: {}", asset.name);
            Some(asset)
        }
        None => {
            log::debug!("No suitable asset found");
            None
        }
    }
}

fn has_name_marker(name: &str, marker: &str) -> bool {
    name.match_indices(marker).any(|(start, matched)| {
        let before = name[..start].chars().next_back();
        let after = name[start + matched.len()..].chars().next();
        before.is_none_or(|c| !c.is_ascii_alphanumeric())
            && after.is_none_or(|c| !c.is_ascii_alphanumeric())
    })
}

/// Strip a single leading "v" from a version tag, but only when it actually
/// prefixes a number ("v1.2.3" -> "1.2.3", "version-2" stays untouched).
fn clean_version_tag(tag: &str) -> String {
    let tag = tag.trim();
    if let Some(rest) = tag.strip_prefix(['v', 'V']) {
        if rest.starts_with(|c: char| c.is_ascii_digit()) {
            return rest.to_string();
        }
    }
    tag.to_string()
}

/// Best-effort guess at which forge a URL belongs to, used when the user has
/// not picked one. Forgejo has no canonical host — an instance can be at any
/// domain — so only the public flagship and hosts that name the software are
/// recognised here; everything else has to be selected explicitly in the Add
/// Program dialog.
pub fn detect_source_type(url: &str) -> Option<SourceType> {
    if url.contains("github.com") {
        Some(SourceType::GitHub)
    } else if url.contains("gitlab.com") || url.contains("gitlab") {
        Some(SourceType::GitLab)
    } else if url_host(url).is_some_and(|host| looks_like_forgejo_host(&host)) {
        Some(SourceType::Forgejo)
    } else {
        None
    }
}

/// The lowercased host of a URL, with any userinfo prefix and port removed.
fn url_host(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Codeberg is the public Forgejo instance, and self-hosters overwhelmingly
/// name the host after the software (`forgejo.example.org`, `git.gitea.io`).
/// Gitea counts because Forgejo's API is a superset of the one this adapter
/// uses.
fn looks_like_forgejo_host(host: &str) -> bool {
    host == "codeberg.org"
        || host.ends_with(".codeberg.org")
        || host
            .split('.')
            .any(|label| label.contains("forgejo") || label.contains("gitea"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            browser_download_url: "url".to_string(),
            size: 100,
        }
    }

    #[test]
    fn test_parse_github_url_basic() {
        assert_eq!(
            GitHubAdapter::parse_github_url("https://github.com/owner/repo").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            GitHubAdapter::parse_github_url("https://github.com/owner/repo/").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            GitHubAdapter::parse_github_url("github.com/owner/repo").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn test_normalize_repo_url() {
        for url in [
            "https://github.com/Owner/Repo",
            "https://GitHub.com/owner/repo/",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo.git/",
            "https://github.com/owner/repo.GIT",
            "http://github.com/owner/repo",
            "https://www.github.com/owner/repo",
            "http://WWW.github.com/owner/repo.Git/",
            "https://github.com/owner/repo?tab=readme",
            "https://github.com/owner/repo/#readme",
        ] {
            assert_eq!(
                super::normalize_repo_url(url),
                "https://github.com/owner/repo",
                "normalizing {}",
                url
            );
        }
    }

    #[test]
    fn test_parse_github_url_extra_segments() {
        // URLs deeper than the repo root should still resolve to owner/repo
        assert_eq!(
            GitHubAdapter::parse_github_url("https://github.com/owner/repo/releases/latest")
                .unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            GitHubAdapter::parse_github_url("https://github.com/owner/repo/tree/main/src").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_url_git_suffix_and_query() {
        assert_eq!(
            GitHubAdapter::parse_github_url("https://github.com/owner/repo.git").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            GitHubAdapter::parse_github_url("https://github.com/owner/repo?tab=readme").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            GitHubAdapter::parse_github_url("https://www.github.com/owner/repo#readme").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_url_invalid() {
        assert!(GitHubAdapter::parse_github_url("https://github.com/owner").is_err());
        assert!(GitHubAdapter::parse_github_url("https://github.com/").is_err());
        assert!(GitHubAdapter::parse_github_url("https://example.com/owner/repo").is_err());
        assert!(GitHubAdapter::parse_github_url("not a url").is_err());
    }

    #[test]
    fn test_parse_forgejo_url_accepts_any_host() {
        for (url, expected) in [
            (
                "https://codeberg.org/owner/repo",
                ("https://codeberg.org", "owner", "repo"),
            ),
            (
                "https://git.example.internal/owner/repo/",
                ("https://git.example.internal", "owner", "repo"),
            ),
            // Scheme-less input is assumed to be https, like the GitHub parser.
            (
                "git.example.internal/owner/repo",
                ("https://git.example.internal", "owner", "repo"),
            ),
            // A plain-http instance on a non-standard port stays intact.
            (
                "http://192.168.1.10:3000/owner/repo",
                ("http://192.168.1.10:3000", "owner", "repo"),
            ),
            // Case is normalized on the host only: Forgejo paths are the
            // repository's own spelling.
            (
                "https://Git.Example.INTERNAL/Owner/Repo",
                ("https://git.example.internal", "Owner", "Repo"),
            ),
            (
                "https://git.example.internal/owner/repo.git",
                ("https://git.example.internal", "owner", "repo"),
            ),
            (
                "https://git.example.internal/owner/repo?tab=releases",
                ("https://git.example.internal", "owner", "repo"),
            ),
            (
                "https://git.example.internal/owner/repo#readme",
                ("https://git.example.internal", "owner", "repo"),
            ),
            // Anything past the repository is ignored.
            (
                "https://git.example.internal/owner/repo/releases/tag/v1.0.0",
                ("https://git.example.internal", "owner", "repo"),
            ),
            (
                "https://git.example.internal/owner/repo/src/branch/main",
                ("https://git.example.internal", "owner", "repo"),
            ),
        ] {
            let (base_url, owner, repo) = parse_forgejo_url(url).expect(url);
            assert_eq!(
                (base_url.as_str(), owner.as_str(), repo.as_str()),
                expected,
                "parsing {}",
                url
            );
        }
    }

    #[test]
    fn test_parse_forgejo_url_drops_userinfo() {
        // Credentials belong in the username/application key fields, not in the
        // URL — and they must never end up inside the instance origin.
        let (base_url, owner, repo) =
            parse_forgejo_url("https://user:secret@git.example.internal/owner/repo").unwrap();
        assert_eq!(base_url, "https://git.example.internal");
        assert_eq!((owner.as_str(), repo.as_str()), ("owner", "repo"));
    }

    #[test]
    fn test_parse_forgejo_url_invalid() {
        for url in [
            "https://git.example.internal/owner",
            "https://git.example.internal/",
            "https://git.example.internal",
            "not a url",
            "ftp://git.example.internal/owner/repo",
            "javascript://x/owner/repo",
        ] {
            assert!(
                parse_forgejo_url(url).is_err(),
                "expected error for {}",
                url
            );
        }
    }

    #[test]
    fn test_detect_source_type() {
        for (url, expected) in [
            ("https://github.com/owner/repo", Some(SourceType::GitHub)),
            ("https://gitlab.com/owner/repo", Some(SourceType::GitLab)),
            ("https://codeberg.org/owner/repo", Some(SourceType::Forgejo)),
            ("https://Codeberg.org/owner/repo", Some(SourceType::Forgejo)),
            (
                "https://forgejo.example.org/owner/repo",
                Some(SourceType::Forgejo),
            ),
            (
                "https://gitea.example.org/owner/repo",
                Some(SourceType::Forgejo),
            ),
            // A private instance on an unremarkable host can't be guessed; the
            // user picks Forgejo explicitly in the dialog instead.
            ("https://git.example.internal/owner/repo", None),
            ("https://example.com/downloads", None),
        ] {
            assert_eq!(detect_source_type(url), expected, "detecting {}", url);
        }
    }

    #[test]
    fn test_credentials_drop_blank_entries() {
        let blank = ForgeCredentials::new(Some("   ".to_string()), Some(String::new()));
        assert_eq!(blank, ForgeCredentials::default());
        assert!(blank.username().is_none());
        assert!(blank.token().is_none());

        let trimmed = ForgeCredentials::new(Some(" alice ".to_string()), Some(" key ".to_string()));
        assert_eq!(trimmed.username(), Some("alice"));
        assert_eq!(trimmed.token(), Some("key"));
    }

    #[test]
    fn test_plaintext_http_is_recognised() {
        // Drives the warning logged before an application key goes out over a
        // connection anyone on the path can read.
        for url in [
            "http://192.168.1.10:3000/owner/repo",
            "HTTP://git.example.internal/owner/repo",
            "  http://git.example.internal/owner/repo",
        ] {
            assert!(is_plaintext_http(url), "expected plain HTTP: {url}");
        }
        for url in [
            "https://git.example.internal/owner/repo",
            "HTTPS://git.example.internal/owner/repo",
            // Not a scheme — a host that merely starts with the same letters.
            "https://http.example.internal/owner/repo",
            "http",
            "",
        ] {
            assert!(!is_plaintext_http(url), "expected not plain HTTP: {url}");
        }
    }

    #[test]
    fn test_debug_output_redacts_the_application_key() {
        // A stray `{:?}` must never put the key in the log — including through
        // a type that merely holds the credentials.
        let credentials =
            ForgeCredentials::new(Some("alice".to_string()), Some("key123".to_string()));
        let auth = DownloadAuth {
            credentials: credentials.clone(),
            origin: "https://git.example.internal".to_string(),
        };

        for rendered in [format!("{:?}", credentials), format!("{:?}", auth)] {
            assert!(!rendered.contains("key123"), "leaked the key: {rendered}");
            assert!(rendered.contains(crate::models::REDACTED), "{rendered}");
            // The username is not a secret and stays readable.
            assert!(rendered.contains("alice"), "{rendered}");
        }
    }

    /// The `Authorization` header the credentials produce, read back off a
    /// built request so the assertion covers what actually goes over the wire.
    fn authorization_header(credentials: &ForgeCredentials) -> Option<String> {
        let request = credentials
            .authorize(
                http_client().get("https://git.example.internal/"),
                "https://git.example.internal/",
            )
            .build()
            .unwrap();
        request
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .map(|value| value.to_str().unwrap().to_string())
    }

    #[test]
    fn test_username_and_key_authenticate_with_basic() {
        // Forgejo reads an application key out of the Basic password, and Basic
        // also works on the web routes that serve release assets.
        let credentials =
            ForgeCredentials::new(Some("alice".to_string()), Some("key123".to_string()));
        let expected = format!(
            "Basic {}",
            base64_standard(format!("{}:{}", "alice", "key123").as_bytes())
        );
        assert_eq!(authorization_header(&credentials), Some(expected));
    }

    #[test]
    fn test_key_without_username_uses_the_token_scheme() {
        let credentials = ForgeCredentials::new(None, Some("key123".to_string()));
        assert_eq!(
            authorization_header(&credentials),
            Some("token key123".to_string())
        );
    }

    #[test]
    fn test_no_key_sends_no_authorization() {
        assert_eq!(authorization_header(&ForgeCredentials::default()), None);
        // A username on its own authenticates nothing.
        assert_eq!(
            authorization_header(&ForgeCredentials::new(Some("alice".to_string()), None)),
            None
        );
    }

    /// Minimal standard-alphabet base64, so the Basic auth test asserts against
    /// an independently computed value rather than reqwest's own encoder.
    fn base64_standard(input: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in input.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let bits = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[(bits >> (18 - 6 * i)) as usize & 0x3f] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    #[test]
    fn test_download_auth_is_scoped_to_the_instance() {
        let adapter = ForgejoAdapter::new(ForgeCredentials::new(
            Some("alice".to_string()),
            Some("key123".to_string()),
        ));
        let auth = adapter
            .download_auth("https://git.example.internal/owner/repo")
            .unwrap();

        let header = |url: &str| {
            auth.authorize(http_client().get(url), url)
                .build()
                .unwrap()
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .map(|value| value.to_str().unwrap().to_string())
        };

        // Assets served by the instance itself get the credentials...
        assert!(
            header("https://git.example.internal/owner/repo/releases/download/v1/App.dmg")
                .is_some()
        );
        // ...anywhere else does not, however the asset URL got there.
        assert_eq!(header("https://cdn.example.com/App.dmg"), None);
        assert_eq!(
            header("http://git.example.internal/owner/repo/releases/download/v1/App.dmg"),
            None
        );
    }

    #[test]
    fn test_same_origin() {
        assert!(same_origin(
            "https://git.example.internal/a/b",
            "https://git.example.internal"
        ));
        // A stored scheme-less URL is read as https, matching its asset URLs.
        assert!(same_origin(
            "https://codeberg.org/owner/repo",
            "codeberg.org"
        ));
        // The default port is implied, so it may be written either way.
        assert!(same_origin(
            "https://git.example.internal:443/a/b",
            "https://git.example.internal"
        ));
        assert!(!same_origin(
            "https://git.example.internal:3000/a/b",
            "https://git.example.internal"
        ));
        assert!(!same_origin(
            "https://evil.example.com/a/b",
            "https://git.example.internal"
        ));
        // A host suffix is not the same origin.
        assert!(!same_origin(
            "https://git.example.internal.evil.com/a/b",
            "https://git.example.internal"
        ));
        assert!(!same_origin("", "https://git.example.internal"));
    }

    #[test]
    fn test_forgejo_error_explains_credential_failures() {
        for status in [
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::FORBIDDEN,
        ] {
            assert!(forgejo_error(status).contains("application key"));
        }
        assert!(
            forgejo_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR).contains("Forgejo API error")
        );
    }

    #[test]
    fn test_forge_release_shape_is_shared_by_both_forges() {
        // Field-for-field, a Forgejo release payload deserializes like a GitHub
        // one — the reason a single adapter-agnostic type is enough. Extra
        // Forgejo-only fields are ignored.
        let payload = r#"{
            "tag_name": "v2.1.0",
            "body": "notes",
            "draft": false,
            "prerelease": false,
            "html_url": "https://git.example.internal/owner/repo/releases/tag/v2.1.0",
            "assets": [
                {
                    "id": 7,
                    "name": "App-macos-universal.dmg",
                    "size": 4096,
                    "uuid": "8e0f",
                    "browser_download_url": "https://git.example.internal/owner/repo/releases/download/v2.1.0/App-macos-universal.dmg"
                }
            ]
        }"#;

        let release: ForgeRelease = serde_json::from_str(payload).unwrap();
        let release = release.into_release().unwrap();
        assert_eq!(release.version, "2.1.0");
        assert_eq!(release.file_name, "App-macos-universal.dmg");
        assert_eq!(release.file_size, Some(4096));
        assert_eq!(release.release_notes.as_deref(), Some("notes"));
        assert!(release
            .download_url
            .starts_with("https://git.example.internal/"));
    }

    struct RecordedRequest {
        target: String,
        authorization: Option<String>,
    }

    /// A stand-in for a Forgejo instance: answers with canned responses in
    /// order and records what was asked for, so the adapter's URL building,
    /// fallback and authentication can be exercised without a network.
    async fn mock_forge(
        responses: Vec<(u16, String)>,
    ) -> (
        String,
        std::sync::Arc<std::sync::Mutex<Vec<RecordedRequest>>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = std::sync::Arc::clone(&requests);

        tokio::spawn(async move {
            for (status, body) in responses {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };

                let mut buffer = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                    }
                    if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }

                let text = String::from_utf8_lossy(&buffer).to_string();
                let mut lines = text.lines();
                let target = lines
                    .next()
                    .and_then(|line| line.split(' ').nth(1))
                    .unwrap_or_default()
                    .to_string();
                let authorization = lines
                    .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, value)| value.trim().to_string());
                recorder.lock().unwrap().push(RecordedRequest {
                    target,
                    authorization,
                });

                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    _ => "Error",
                };
                // Closing each connection keeps the accept loop above in step
                // with the requests instead of racing keep-alive reuse.
                let response = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    reason,
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });

        (address, requests)
    }

    fn forgejo_release_json(tag: &str, draft: bool, asset: &str) -> String {
        format!(
            r#"{{
                "tag_name": "{tag}",
                "body": "notes for {tag}",
                "draft": {draft},
                "prerelease": false,
                "assets": [
                    {{
                        "name": "{asset}",
                        "size": 1024,
                        "browser_download_url": "https://forge.invalid/download/{asset}"
                    }}
                ]
            }}"#
        )
    }

    #[tokio::test]
    async fn test_forgejo_reads_the_latest_release_with_credentials() {
        let (address, requests) = mock_forge(vec![(
            200,
            forgejo_release_json("v1.4.0", false, "App-macos-universal.dmg"),
        )])
        .await;

        let adapter = ForgejoAdapter::new(ForgeCredentials::new(
            Some("alice".to_string()),
            Some("key123".to_string()),
        ));
        let release = adapter
            .get_latest_release(&format!("{}/owner/repo", address))
            .await
            .unwrap();

        assert_eq!(release.version, "1.4.0");
        assert_eq!(release.file_name, "App-macos-universal.dmg");
        assert_eq!(release.file_size, Some(1024));

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].target,
            "/api/v1/repos/owner/repo/releases/latest"
        );
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some(format!("Basic {}", base64_standard(b"alice:key123")).as_str())
        );
    }

    #[tokio::test]
    async fn test_forgejo_falls_back_to_the_release_list() {
        // Forgejo's `releases/latest` 404s for a repository that only publishes
        // prereleases, exactly like GitHub's.
        let (address, requests) = mock_forge(vec![
            (404, "{}".to_string()),
            (
                200,
                // Universal assets so the fixture picks the same release on
                // either Mac architecture.
                format!(
                    "[{}, {}]",
                    forgejo_release_json("v2.0.0-rc.1", true, "Draft-macos-universal.dmg"),
                    forgejo_release_json("v1.9.0", false, "App-macos-universal.dmg")
                ),
            ),
        ])
        .await;

        let adapter = ForgejoAdapter::new(ForgeCredentials::default());
        let release = adapter
            .get_latest_release(&format!("{}/owner/repo", address))
            .await
            .unwrap();

        // The draft is skipped, so the newest published release wins.
        assert_eq!(release.version, "1.9.0");

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[1].target,
            "/api/v1/repos/owner/repo/releases?limit=10"
        );
        // Nothing to authenticate with, so nothing is sent.
        assert!(requests[1].authorization.is_none());
    }

    #[tokio::test]
    async fn test_forgejo_missing_repository_points_at_the_credentials() {
        // A private repository the credentials cannot see 404s just like a
        // repository that does not exist, so the message has to cover both.
        let (address, _) = mock_forge(vec![(404, "{}".to_string()), (404, "{}".to_string())]).await;

        let adapter = ForgejoAdapter::new(ForgeCredentials::default());
        let error = adapter
            .get_latest_release(&format!("{}/owner/repo", address))
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("application key"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn test_forgejo_rejected_credentials_are_reported_plainly() {
        let (address, _) = mock_forge(vec![(401, "{}".to_string())]).await;

        let adapter = ForgejoAdapter::new(ForgeCredentials::new(
            Some("alice".to_string()),
            Some("wrong".to_string()),
        ));
        let error = adapter
            .get_latest_release(&format!("{}/owner/repo", address))
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("401"), "unexpected error: {error}");
        assert!(
            error.contains("username and application key"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_clean_version_tag() {
        assert_eq!(clean_version_tag("v1.2.3"), "1.2.3");
        assert_eq!(clean_version_tag("1.2.3"), "1.2.3");
        assert_eq!(clean_version_tag("V2.0"), "2.0");
        assert_eq!(clean_version_tag("version-2"), "version-2");
        assert_eq!(clean_version_tag("vapor"), "vapor");
    }

    #[test]
    fn test_asset_extension_requires_dot() {
        // "checksums-dmg" must not be mistaken for a .dmg file
        let assets = vec![asset("checksums-dmg"), asset("MyApp-macos.zip")];

        let selected = find_macos_asset(&assets).unwrap();
        assert_eq!(selected.name, "MyApp-macos.zip");
    }

    #[test]
    fn test_ferrite_asset_selection() {
        // Reproduction case from user: Ferrite v0.2.3
        // Should favor macos tar.gz over windows zip
        let assets = vec![
            asset("ferrite-linux-x64.tar.gz"),
            asset("ferrite-macos-arm64.tar.gz"),
            asset("ferrite-macos-x64.tar.gz"),
            asset("ferrite-windows-x64.zip"),
        ];

        let selected = find_macos_asset(&assets).unwrap();
        // Definitely a macOS tar.gz, never the windows zip
        assert!(selected.name.contains("macos"));
        assert!(selected.name.ends_with("tar.gz"));
    }

    #[test]
    fn test_universal_preferred() {
        let assets = vec![
            asset("tool-x86_64.dmg"),
            asset("tool-universal.dmg"),
            asset("tool-arm64.dmg"),
        ];
        for target_arch in [MacArch::AppleSilicon, MacArch::X86_64] {
            let selected = find_macos_asset_for_arch(&assets, target_arch).unwrap();
            assert_eq!(selected.name, "tool-universal.dmg");
        }
    }

    #[test]
    fn test_native_arch_preferred_for_each_target() {
        let assets = vec![
            asset("tool-macos-x86_64.dmg"),
            asset("tool-macos-arm64.dmg"),
        ];
        for (target_arch, expected) in [
            (MacArch::AppleSilicon, "tool-macos-arm64.dmg"),
            (MacArch::X86_64, "tool-macos-x86_64.dmg"),
        ] {
            let selected = find_macos_asset_for_arch(&assets, target_arch).unwrap();
            assert_eq!(selected.name, expected);
        }
    }

    #[test]
    fn test_asset_selection_wrapper_dispatches_to_build_target() {
        let assets = vec![
            asset("tool-macos-x86_64.dmg"),
            asset("tool-macos-arm64.dmg"),
        ];
        let target_arch = if cfg!(target_arch = "aarch64") {
            MacArch::AppleSilicon
        } else {
            MacArch::X86_64
        };

        let selected = find_macos_asset(&assets).unwrap();
        let expected = find_macos_asset_for_arch(&assets, target_arch).unwrap();
        assert_eq!(selected.name, expected.name);
    }

    #[test]
    fn test_dmg_fallback_without_keywords() {
        // dmg is macOS-specific, so it may match without a keyword
        let assets = vec![asset("MyTool-1.2.3.dmg")];
        let selected = find_macos_asset(&assets).unwrap();
        assert_eq!(selected.name, "MyTool-1.2.3.dmg");
    }

    #[test]
    fn test_generic_archives_require_macos_marker() {
        let assets = vec![
            asset("tool-windows-arm64.zip"),
            asset("tool-linux-x86_64.tar.gz"),
            asset("tool-arm64.zip"),
            asset("tool-x86_64.tar.gz"),
        ];
        for target_arch in [MacArch::AppleSilicon, MacArch::X86_64] {
            assert!(find_macos_asset_for_arch(&assets, target_arch).is_none());
        }
    }

    #[test]
    fn test_architecture_compatibility_precedes_package_preference() {
        let cases: &[(MacArch, &[&str], &str)] = &[
            (
                MacArch::AppleSilicon,
                &["tool-macos-x86_64.dmg", "tool-macos-arm64.zip"],
                "tool-macos-arm64.zip",
            ),
            (
                MacArch::X86_64,
                &["tool-macos-arm64.dmg", "tool-macos-x86_64.zip"],
                "tool-macos-x86_64.zip",
            ),
        ];

        for (target_arch, names, expected) in cases {
            let assets: Vec<_> = names.iter().map(|name| asset(name)).collect();
            let selected = find_macos_asset_for_arch(&assets, *target_arch).unwrap();
            assert_eq!(selected.name, *expected);
        }
    }

    #[test]
    fn test_equal_scores_use_asset_name_as_tiebreaker() {
        for names in [
            ["tool-macos-x86_64-zeta.dmg", "tool-macos-x86_64-alpha.dmg"],
            ["tool-macos-x86_64-alpha.dmg", "tool-macos-x86_64-zeta.dmg"],
        ] {
            let assets: Vec<_> = names.iter().map(|name| asset(name)).collect();
            let selected = find_macos_asset_for_arch(&assets, MacArch::X86_64).unwrap();
            assert_eq!(selected.name, "tool-macos-x86_64-alpha.dmg");
        }
    }

    #[test]
    fn test_explicit_non_macos_markers_are_rejected() {
        let cases: &[(MacArch, &[&str], &str)] = &[
            (
                MacArch::AppleSilicon,
                &["tool-linux-arm64.dmg", "tool-macos-arm64.zip"],
                "tool-macos-arm64.zip",
            ),
            (
                MacArch::X86_64,
                &["tool-windows-x86_64.pkg", "tool-macos-x86_64.zip"],
                "tool-macos-x86_64.zip",
            ),
        ];

        for (target_arch, names, expected) in cases {
            let assets: Vec<_> = names.iter().map(|name| asset(name)).collect();
            let selected = find_macos_asset_for_arch(&assets, *target_arch).unwrap();
            assert_eq!(selected.name, *expected);
        }
    }

    #[test]
    fn test_intel_fallback_is_apple_silicon_only() {
        let assets = vec![
            asset("tool-windows-x86_64.pkg"),
            asset("tool-macos-x86_64.dmg"),
        ];
        assert_eq!(
            find_macos_asset_for_arch(&assets, MacArch::AppleSilicon)
                .unwrap()
                .name,
            "tool-macos-x86_64.dmg"
        );

        let assets = vec![asset("tool-macos-arm64.dmg")];
        assert!(find_macos_asset_for_arch(&assets, MacArch::X86_64).is_none());
    }

    #[test]
    fn test_unsupported_cpu_assets_are_rejected() {
        for (target_arch, name) in [
            (MacArch::AppleSilicon, "tool-macos-armv7.dmg"),
            (MacArch::AppleSilicon, "tool-macos-x86_32.dmg"),
            (MacArch::AppleSilicon, "tool-macos-i786.dmg"),
            (MacArch::X86_64, "tool-macos-arm64.dmg"),
            (MacArch::X86_64, "tool-macos-x86_32.dmg"),
            (MacArch::X86_64, "tool-macos-i786.dmg"),
        ] {
            let assets = vec![asset(name)];
            assert!(find_macos_asset_for_arch(&assets, target_arch).is_none());
        }
    }
}
