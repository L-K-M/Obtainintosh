use crate::models::{Release, SourceType};
use anyhow::{Context, Result};
use serde::Deserialize;

pub const USER_AGENT: &str = concat!("Obtainintosh/", env!("CARGO_PKG_VERSION"));

/// The platform this build picks release assets for, as named in user-facing
/// messages ("No macOS-compatible asset found").
#[cfg(target_os = "macos")]
const PLATFORM_LABEL: &str = "macOS";
#[cfg(not(target_os = "macos"))]
const PLATFORM_LABEL: &str = "Linux";

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
    #[serde(default)]
    prerelease: bool,
}

/// How far back to look when the newest release carries nothing this machine
/// can install. Matches the page size both forges are asked for.
const RECENT_RELEASE_LIMIT: usize = 10;

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

impl ForgeRelease {
    /// Turns the forge's release into ours, picking the asset this machine can
    /// use.
    fn build_release(&self) -> Result<Release> {
        let asset = find_compatible_asset(&self.assets).with_context(|| {
            format!("Selected release has no {PLATFORM_LABEL}-compatible asset")
        })?;

        Ok(Release {
            version: clean_version_tag(&self.tag_name),
            download_url: asset.browser_download_url.clone(),
            file_name: asset.name.clone(),
            file_size: Some(asset.size),
            checksum: None,
            release_notes: self.body.clone(),
        })
    }

    fn has_compatible_asset(&self) -> bool {
        find_compatible_asset(&self.assets).is_some()
    }
}

/// Whether the list contains even one published release. Drafts do not
/// count: both forges hide them from anyone without write access, so a
/// repository whose releases are all drafts publishes nothing as far as
/// every reader — this app included — can see.
fn has_published_release(releases: &[ForgeRelease]) -> bool {
    releases.iter().any(|release| !release.draft)
}

/// Picks a release this machine can actually install out of the recent list.
///
/// Whether the forge had a nominal latest release at all decides how much
/// freedom there is. Both forges' `releases/latest` returns the newest
/// published, non-prerelease release, so when it answered, the repository does
/// publish a stable channel and that is the channel the user is tracking.
/// Quietly moving them onto a prerelease because the stable build happened to
/// ship assets for other platforms only would change what they are subscribed
/// to without saying so, hence the restriction to stable there.
///
/// When it 404s there is no such expectation to protect, and the reason is not
/// knowable from here — a repository that only ever publishes prereleases, one
/// whose only stable releases are still drafts, or a forge that is simply
/// inconsistent about the flag. So the search widens to the newest published
/// release of any channel that carries an installable asset.
fn select_recent_release(
    releases: &[ForgeRelease],
    stable_release_published: bool,
) -> Result<&ForgeRelease> {
    // A list with nothing published in it gets its own message — but only
    // when the list is the sole evidence. When `releases/latest` answered,
    // a published release exists by definition (perhaps just outside the
    // recent window), so the stable-channel message below stays accurate
    // there.
    //
    // "No compatible release" implies releases exist and merely missed this
    // platform; an unpublished repository is a different problem, and one
    // that is easy to miss from the releases page, because drafts *are*
    // shown there to the repository's maintainers — a release whose workflow
    // failed before publishing looks present to its owner and absent to
    // everyone else.
    if !stable_release_published && !has_published_release(releases) {
        anyhow::bail!(
            "No published releases found among the releases visible to \
             this app. Draft releases are invisible to anyone without write \
             access, so a release stuck in draft — for example by a failed \
             release workflow — shows on the releases page for its \
             maintainers while remaining unpublished"
        );
    }

    if stable_release_published {
        return find_compatible_release(releases, Some(false)).with_context(|| {
            format!(
                "No {}-compatible stable release found in the {} most recent releases",
                PLATFORM_LABEL, RECENT_RELEASE_LIMIT
            )
        });
    }

    find_compatible_release(releases, None).with_context(|| {
        format!(
            "No {}-compatible release found in the {} most recent releases",
            PLATFORM_LABEL, RECENT_RELEASE_LIMIT
        )
    })
}

/// The newest published release carrying an asset this machine can use,
/// optionally restricted to one channel. Drafts are unpublished and never
/// count. The list arrives newest-first from both forges, so the first match
/// is the newest.
fn find_compatible_release(
    releases: &[ForgeRelease],
    prerelease: Option<bool>,
) -> Option<&ForgeRelease> {
    releases.iter().find(|release| {
        !release.draft
            && prerelease.is_none_or(|wanted| release.prerelease == wanted)
            && release.has_compatible_asset()
    })
}

pub struct GitHubAdapter {
    token: Option<String>,
}

/// The CPU the running build targets. Shared by the macOS and Linux asset
/// pickers — both platforms ship for exactly these two architectures.
#[derive(Clone, Copy)]
enum CpuArch {
    Arm64,
    X86_64,
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
compile_error!("release asset selection supports only aarch64 and x86_64 targets");

fn target_cpu_arch() -> CpuArch {
    if cfg!(target_arch = "aarch64") {
        CpuArch::Arm64
    } else {
        CpuArch::X86_64
    }
}

/// CPU families no supported build can run, spelled the ways release assets
/// actually name them (including Debian architecture names like `ppc64el` and
/// `armel` — the boundary check in `has_name_marker` means `ppc64` does not
/// match inside `ppc64el`, so the long forms need their own entries).
/// One shared list for both pickers, so an architecture learned on one
/// platform cannot silently stay accepted on the other. Bare `x86` is absent
/// on purpose: `_` and `-` are marker boundaries, so it would also match
/// inside `x86_64` and `x86-64` and reject every 64-bit Intel asset.
const UNSUPPORTED_CPU_MARKERS: &[&str] = &[
    "armv5",
    "armv6",
    "armv7",
    "armel",
    "armhf",
    "i386",
    "i486",
    "i586",
    "i686",
    "i786",
    "x86_32",
    "x86-32",
    // 32-bit x86 by long-standing convention, like the win32/linux32 pairing.
    "linux32",
    "powerpc",
    "ppc",
    "ppc64",
    "ppc64el",
    "ppc64le",
    "mips",
    "mipsel",
    "mips64",
    "mips64el",
    "s390x",
    "sparc",
    "sparc64",
    "riscv32",
    "riscv64",
    "loongarch64",
    "loong64",
];

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

        let latest: Option<ForgeRelease> = if response.status() == reqwest::StatusCode::NOT_FOUND {
            // `releases/latest` 404s when a repo only has pre-releases (common for
            // nightly/continuous builds), so fall back to the release list.
            None
        } else if !response.status().is_success() {
            anyhow::bail!("GitHub API error: {}", response.status());
        } else {
            Some(
                response
                    .json()
                    .await
                    .context("Failed to parse GitHub release")?,
            )
        };

        // The nominal latest release is only usable if it ships something this
        // machine can install. A project that publishes a source-only point
        // release, or one built for other platforms only, would otherwise make
        // the app report no compatible asset at all, even with a perfectly
        // good build one release back.
        if let Some(release) = latest.as_ref() {
            if release.has_compatible_asset() {
                return release.build_release();
            }
        }

        let releases = self.get_recent_releases(&owner, &repo).await?;
        select_recent_release(&releases, latest.is_some())?.build_release()
    }

    async fn get_recent_releases(&self, owner: &str, repo: &str) -> Result<Vec<ForgeRelease>> {
        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases?per_page={}",
            owner, repo, RECENT_RELEASE_LIMIT
        );

        let response = self
            .get(&api_url)
            .send()
            .await
            .context("Failed to fetch GitHub releases")?;

        if !response.status().is_success() {
            anyhow::bail!("GitHub API error: {}", response.status());
        }

        response
            .json()
            .await
            .context("Failed to parse GitHub releases")
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

        let latest: Option<ForgeRelease> = if response.status() == reqwest::StatusCode::NOT_FOUND {
            // Forgejo's `releases/latest` returns the newest non-draft,
            // non-prerelease release and 404s when there is none — the same
            // shape as GitHub, so fall back to the release list the same way.
            // A repository the credentials cannot see also 404s here, which
            // `get_recent_releases` reports on.
            None
        } else if !response.status().is_success() {
            anyhow::bail!("{}", forgejo_error(response.status()));
        } else {
            Some(
                response
                    .json()
                    .await
                    .context("Failed to parse Forgejo release")?,
            )
        };

        if let Some(release) = latest.as_ref() {
            if release.has_compatible_asset() {
                return release.build_release();
            }
        }

        let releases = self.get_recent_releases(&releases_url).await?;
        select_recent_release(&releases, latest.is_some())?.build_release()
    }

    async fn get_recent_releases(&self, releases_url: &str) -> Result<Vec<ForgeRelease>> {
        let response = self
            .get(&format!("{}?limit={}", releases_url, RECENT_RELEASE_LIMIT))
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

        response
            .json()
            .await
            .context("Failed to parse Forgejo releases")
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

/// `str::get` rather than a `[..7]` slice: indexing panics when byte 7 lands
/// inside a multi-byte character, and this reads URLs that came off the
/// network. A short or non-ASCII-prefixed URL is simply not plain HTTP.
fn is_plaintext_http(url: &str) -> bool {
    url.trim_start()
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
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

/// The release asset the platform this build runs on can install, if any.
///
/// Dispatches with `cfg!` rather than `#[cfg]` so both platform pickers are
/// compiled — and their tests run — everywhere; the unused one is eliminated
/// as dead code in release builds.
fn find_compatible_asset(assets: &[ReleaseAsset]) -> Option<&ReleaseAsset> {
    if cfg!(target_os = "macos") {
        find_macos_asset_for_arch(assets, target_cpu_arch())
    } else {
        find_linux_asset_for_arch(assets, target_cpu_arch())
    }
}

fn find_macos_asset_for_arch(
    assets: &[ReleaseAsset],
    target_arch: CpuArch,
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
            let unsupported_cpu = UNSUPPORTED_CPU_MARKERS
                .iter()
                .any(|marker| has_name_marker(&name, marker));

            if unsupported_cpu
                || matches!(target_arch, CpuArch::X86_64) && arm64 && !intel64 && !universal
            {
                return None;
            }

            // Intel-only builds remain a last-resort option on Apple Silicon because
            // Rosetta 2 can run them. The reverse is not possible on x86_64 Macs.
            let architecture_rank = match target_arch {
                CpuArch::Arm64 if universal => 0,
                CpuArch::Arm64 if arm64 => 1,
                CpuArch::Arm64 if !intel64 => 2,
                CpuArch::Arm64 => 3,
                CpuArch::X86_64 if universal => 0,
                CpuArch::X86_64 if intel64 => 1,
                CpuArch::X86_64 => 2,
            };

            Some((asset, (architecture_rank, package_rank)))
        })
        .min_by(|(asset_a, rank_a), (asset_b, rank_b)| {
            rank_a
                .cmp(rank_b)
                .then_with(|| asset_a.name.cmp(&asset_b.name))
        });

    match selected {
        Some((asset, (3, _))) if matches!(target_arch, CpuArch::Arm64) => {
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

/// The Linux counterpart of `find_macos_asset_for_arch`, sharing its shape:
/// filter by package format and filename markers, then rank by (architecture,
/// package format) with the asset name as the tiebreaker.
///
/// `.deb` ranks first because it is the native package on the Ubuntu systems
/// this build targets, ahead of the distribution-independent `.AppImage`.
/// `.rpm` is deliberately not accepted — it is a Linux format, but not one an
/// Ubuntu system installs. Generic archives need a Linux marker in the
/// filename, exactly like generic archives need a macOS marker on a Mac.
///
/// Unlike macOS there is no universal binary and no Rosetta: an asset marked
/// for the other CPU is never usable, so it is rejected outright, and an
/// unmarked asset ranks below one that names the native architecture.
fn find_linux_asset_for_arch(
    assets: &[ReleaseAsset],
    target_arch: CpuArch,
) -> Option<&ReleaseAsset> {
    log::debug!("Finding Linux asset from {} candidates", assets.len());
    let extensions = [".deb", ".appimage", ".tar.gz", ".zip"];
    // "linux32" is deliberately not a valid marker: it names a 32-bit x86
    // build, which no supported target can run — it is rejected through
    // UNSUPPORTED_CPU_MARKERS instead.
    let linux_markers = ["linux", "linux64", "ubuntu", "debian"];
    let other_os_markers = [
        "windows", "win", "win32", "win64", "mac", "macos", "macosx", "darwin", "osx", "android",
        "freebsd", "openbsd", "netbsd", "solaris", "ios", "tvos",
    ];
    // The first index that is a generic archive rather than a Linux-specific
    // package format, mirroring the macOS picker's `package_rank >= 3` check.
    const FIRST_GENERIC_ARCHIVE_RANK: usize = 2;

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

            let has_linux_marker = linux_markers
                .iter()
                .any(|marker| has_name_marker(&name, marker));
            let generic_archive = package_rank >= FIRST_GENERIC_ARCHIVE_RANK;
            if generic_archive && !has_linux_marker {
                return None;
            }

            let arm64 = ["arm64", "aarch64"]
                .iter()
                .any(|marker| has_name_marker(&name, marker));
            // "intel" is left out on purpose: unlike macOS naming, Linux asset
            // names rarely use it for the CPU (an "intel" variant is more
            // often about GPUs or MKL builds). "linux64" counts: by the same
            // convention that makes linux32 mean 32-bit x86, it means x86_64 —
            // and the boundary check keeps the bare "x64" marker from matching
            // inside it, so it needs its own entry.
            let x86_64 = ["x86_64", "x86-64", "amd64", "x64", "linux64"]
                .iter()
                .any(|marker| has_name_marker(&name, marker));
            let unsupported_cpu = UNSUPPORTED_CPU_MARKERS
                .iter()
                .any(|marker| has_name_marker(&name, marker));

            let foreign_arch = match target_arch {
                CpuArch::Arm64 => x86_64 && !arm64,
                CpuArch::X86_64 => arm64 && !x86_64,
            };
            if unsupported_cpu || foreign_arch {
                return None;
            }

            let native_arch = match target_arch {
                CpuArch::Arm64 => arm64,
                CpuArch::X86_64 => x86_64,
            };
            let architecture_rank = if native_arch { 0 } else { 1 };

            Some((asset, (architecture_rank, package_rank)))
        })
        .min_by(|(asset_a, rank_a), (asset_b, rank_b)| {
            rank_a
                .cmp(rank_b)
                .then_with(|| asset_a.name.cmp(&asset_b.name))
        });

    match selected {
        Some((asset, _)) => {
            log::debug!("Selected compatible Linux asset: {}", asset.name);
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
/// A bracketed IPv6 literal comes back truncated, which does not matter here:
/// the only caller matches host *names* against forge software, and an IP
/// literal never carries one, so such a URL falls through to "not detected"
/// and the user picks the source type by hand.
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

    /// An asset name the picker accepts on the platform these tests run on,
    /// whatever the CPU: universal on macOS, architecture-unmarked on Linux.
    fn compatible_asset_name(stem: &str) -> String {
        if cfg!(target_os = "macos") {
            format!("{stem}-macos-universal.dmg")
        } else {
            format!("{stem}.AppImage")
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
            // Byte 7 falls inside the two-byte "é": slicing here would panic.
            "http:/éxample.internal/owner/repo",
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
        // Forgejo-only fields are ignored. Assets for both platforms, so the
        // pick works wherever this test runs.
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
                },
                {
                    "id": 8,
                    "name": "App.AppImage",
                    "size": 4096,
                    "uuid": "8e10",
                    "browser_download_url": "https://git.example.internal/owner/repo/releases/download/v2.1.0/App.AppImage"
                }
            ]
        }"#;

        let release: ForgeRelease = serde_json::from_str(payload).unwrap();
        let release = release.build_release().unwrap();
        assert_eq!(release.version, "2.1.0");
        assert_eq!(release.file_name, compatible_asset_name("App"));
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
            forgejo_release_json("v1.4.0", false, &compatible_asset_name("App")),
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
        assert_eq!(release.file_name, compatible_asset_name("App"));
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
                // Architecture-neutral assets so the fixture picks the same
                // release whatever CPU the tests run on.
                format!(
                    "[{}, {}]",
                    forgejo_release_json("v2.0.0-rc.1", true, &compatible_asset_name("Draft")),
                    forgejo_release_json("v1.9.0", false, &compatible_asset_name("App"))
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

        let selected = find_macos_asset_for_arch(&assets, target_cpu_arch()).unwrap();
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

        let selected = find_macos_asset_for_arch(&assets, target_cpu_arch()).unwrap();
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
        for target_arch in [CpuArch::Arm64, CpuArch::X86_64] {
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
            (CpuArch::Arm64, "tool-macos-arm64.dmg"),
            (CpuArch::X86_64, "tool-macos-x86_64.dmg"),
        ] {
            let selected = find_macos_asset_for_arch(&assets, target_arch).unwrap();
            assert_eq!(selected.name, expected);
        }
    }

    #[test]
    fn test_asset_selection_wrapper_dispatches_to_build_target() {
        // Assets for both platforms and both CPUs, so the wrapper has a valid
        // pick wherever this test runs.
        let assets = vec![
            asset("tool-macos-x86_64.dmg"),
            asset("tool-macos-arm64.dmg"),
            asset("tool-linux-x86_64.deb"),
            asset("tool-linux-arm64.deb"),
        ];

        let expected = if cfg!(target_os = "macos") {
            find_macos_asset_for_arch(&assets, target_cpu_arch()).unwrap()
        } else {
            find_linux_asset_for_arch(&assets, target_cpu_arch()).unwrap()
        };
        let selected = find_compatible_asset(&assets).unwrap();
        assert_eq!(selected.name, expected.name);
    }

    #[test]
    fn test_dmg_fallback_without_keywords() {
        // dmg is macOS-specific, so it may match without a keyword
        let assets = vec![asset("MyTool-1.2.3.dmg")];
        let selected = find_macos_asset_for_arch(&assets, target_cpu_arch()).unwrap();
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
        for target_arch in [CpuArch::Arm64, CpuArch::X86_64] {
            assert!(find_macos_asset_for_arch(&assets, target_arch).is_none());
        }
    }

    #[test]
    fn test_architecture_compatibility_precedes_package_preference() {
        let cases: &[(CpuArch, &[&str], &str)] = &[
            (
                CpuArch::Arm64,
                &["tool-macos-x86_64.dmg", "tool-macos-arm64.zip"],
                "tool-macos-arm64.zip",
            ),
            (
                CpuArch::X86_64,
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
            let selected = find_macos_asset_for_arch(&assets, CpuArch::X86_64).unwrap();
            assert_eq!(selected.name, "tool-macos-x86_64-alpha.dmg");
        }
    }

    #[test]
    fn test_explicit_non_macos_markers_are_rejected() {
        let cases: &[(CpuArch, &[&str], &str)] = &[
            (
                CpuArch::Arm64,
                &["tool-linux-arm64.dmg", "tool-macos-arm64.zip"],
                "tool-macos-arm64.zip",
            ),
            (
                CpuArch::X86_64,
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
            find_macos_asset_for_arch(&assets, CpuArch::Arm64)
                .unwrap()
                .name,
            "tool-macos-x86_64.dmg"
        );

        let assets = vec![asset("tool-macos-arm64.dmg")];
        assert!(find_macos_asset_for_arch(&assets, CpuArch::X86_64).is_none());
    }

    #[test]
    fn test_unsupported_cpu_assets_are_rejected() {
        for (target_arch, name) in [
            (CpuArch::Arm64, "tool-macos-armv7.dmg"),
            (CpuArch::Arm64, "tool-macos-x86_32.dmg"),
            (CpuArch::Arm64, "tool-macos-i786.dmg"),
            (CpuArch::X86_64, "tool-macos-arm64.dmg"),
            (CpuArch::X86_64, "tool-macos-x86_32.dmg"),
            (CpuArch::X86_64, "tool-macos-i786.dmg"),
        ] {
            let assets = vec![asset(name)];
            assert!(find_macos_asset_for_arch(&assets, target_arch).is_none());
        }
    }

    #[test]
    fn test_linux_prefers_deb_then_appimage_then_marked_archives() {
        let assets = vec![
            asset("tool-linux-x86_64.tar.gz"),
            asset("tool-x86_64.AppImage"),
            asset("tool-linux-amd64.deb"),
            asset("tool-windows-x64.zip"),
            asset("tool-macos-universal.dmg"),
        ];

        let selected = find_linux_asset_for_arch(&assets, CpuArch::X86_64).unwrap();
        assert_eq!(selected.name, "tool-linux-amd64.deb");

        let without_deb = vec![
            asset("tool-linux-x86_64.tar.gz"),
            asset("tool-x86_64.AppImage"),
            asset("tool-windows-x64.zip"),
        ];
        let selected = find_linux_asset_for_arch(&without_deb, CpuArch::X86_64).unwrap();
        assert_eq!(selected.name, "tool-x86_64.AppImage");
    }

    #[test]
    fn test_linux_generic_archives_require_linux_marker() {
        // deb and AppImage are Linux-specific, so they match without a keyword;
        // a bare archive could be anything, source code included.
        let assets = vec![asset("tool-x86_64.tar.gz"), asset("tool-x86_64.zip")];
        assert!(find_linux_asset_for_arch(&assets, CpuArch::X86_64).is_none());

        let assets = vec![asset("tool-linux-x86_64.tar.gz")];
        assert_eq!(
            find_linux_asset_for_arch(&assets, CpuArch::X86_64)
                .unwrap()
                .name,
            "tool-linux-x86_64.tar.gz"
        );

        let assets = vec![asset("tool.deb"), asset("tool.AppImage")];
        assert_eq!(
            find_linux_asset_for_arch(&assets, CpuArch::X86_64)
                .unwrap()
                .name,
            "tool.deb"
        );
    }

    #[test]
    fn test_linux_native_arch_preferred_and_foreign_arch_rejected() {
        let assets = vec![asset("tool-linux-amd64.deb"), asset("tool-linux-arm64.deb")];
        for (target_arch, expected) in [
            (CpuArch::Arm64, "tool-linux-arm64.deb"),
            (CpuArch::X86_64, "tool-linux-amd64.deb"),
        ] {
            let selected = find_linux_asset_for_arch(&assets, target_arch).unwrap();
            assert_eq!(selected.name, expected);
        }

        // There is no Rosetta on Linux: an asset for the other CPU is never
        // usable, on either target.
        let assets = vec![asset("tool-linux-arm64.deb")];
        assert!(find_linux_asset_for_arch(&assets, CpuArch::X86_64).is_none());
        let assets = vec![asset("tool-linux-x86_64.AppImage")];
        assert!(find_linux_asset_for_arch(&assets, CpuArch::Arm64).is_none());
    }

    #[test]
    fn test_linux_unmarked_arch_ranks_below_native() {
        let assets = vec![asset("tool-linux.deb"), asset("tool-linux-amd64.deb")];
        let selected = find_linux_asset_for_arch(&assets, CpuArch::X86_64).unwrap();
        assert_eq!(selected.name, "tool-linux-amd64.deb");

        // An unmarked asset is still acceptable when nothing names the CPU.
        let assets = vec![asset("tool-linux.deb")];
        assert!(find_linux_asset_for_arch(&assets, CpuArch::Arm64).is_some());
    }

    #[test]
    fn test_linux_architecture_compatibility_precedes_package_preference() {
        // A native-arch archive beats a foreign-format .deb-less pick: the
        // (architecture, package) rank order matches the macOS picker.
        let assets = vec![
            asset("tool-linux-arm64.AppImage"),
            asset("tool-linux-x86_64.tar.gz"),
        ];
        let selected = find_linux_asset_for_arch(&assets, CpuArch::X86_64).unwrap();
        assert_eq!(selected.name, "tool-linux-x86_64.tar.gz");
    }

    #[test]
    fn test_linux_rejects_other_platforms_and_unsupported_cpus() {
        for name in [
            "tool-macos-universal.dmg",
            "tool-macos-arm64.zip",
            "tool-windows-x64.zip",
            "tool-linux-x86_64.rpm",
            "tool-linux-armv7.deb",
            "tool-linux-i386.deb",
            "tool-linux-riscv64.AppImage",
            // 32-bit x86 by convention, so never runnable on a supported
            // target — and it must not pass as a mere "Linux" marker either.
            "tool-linux32.tar.gz",
            "tool-linux32.AppImage",
            // Debian architecture names: the marker boundary check means the
            // short forms don't match inside them, so each needs its own entry
            // in UNSUPPORTED_CPU_MARKERS.
            "tool_1.0_ppc64el.deb",
            "tool_1.0_armel.deb",
            "tool_1.0_s390x.deb",
            "tool_1.0_mips64el.deb",
            "tool_1.0_loong64.deb",
            "tool_1.0_sparc64.deb",
        ] {
            let assets = vec![asset(name)];
            for target_arch in [CpuArch::Arm64, CpuArch::X86_64] {
                assert!(
                    find_linux_asset_for_arch(&assets, target_arch).is_none(),
                    "should reject {name}"
                );
            }
        }
    }

    /// Obtainintosh tracks itself, so its own release has to be readable by
    /// every build it ships. These are the exact file names the release
    /// workflow produces (Tauri's `<Product>_<version>_<arch>` naming for the
    /// Linux bundles, tauri-action's for the macOS ones) — if a bundle target
    /// or naming convention changes, the app stops seeing its own updates,
    /// which is precisely the failure nobody notices until a release is out.
    #[test]
    fn test_obtainintoshs_own_release_assets_are_readable_on_every_shipped_build() {
        let assets = vec![
            asset("Obtainintosh_1.6.0_aarch64.dmg"),
            asset("Obtainintosh_1.6.0_x64.dmg"),
            asset("Obtainintosh_1.6.0_amd64.deb"),
            asset("Obtainintosh_1.6.0_amd64.AppImage"),
        ];

        // The Linux builds are x86_64-only, and the .deb is preferred there.
        assert_eq!(
            find_linux_asset_for_arch(&assets, CpuArch::X86_64)
                .unwrap()
                .name,
            "Obtainintosh_1.6.0_amd64.deb"
        );
        // Each Mac gets its own architecture's disk image, and never a Linux
        // package — the extension gate keeps .deb/.AppImage out entirely.
        assert_eq!(
            find_macos_asset_for_arch(&assets, CpuArch::Arm64)
                .unwrap()
                .name,
            "Obtainintosh_1.6.0_aarch64.dmg"
        );
        assert_eq!(
            find_macos_asset_for_arch(&assets, CpuArch::X86_64)
                .unwrap()
                .name,
            "Obtainintosh_1.6.0_x64.dmg"
        );
    }

    #[test]
    fn test_linux32_is_never_chosen_over_a_64_bit_build() {
        // The legacy naming pair: both carry a Linux marker and neither spells
        // out a CPU the short markers match, so without linux32/linux64 being
        // classified the name tiebreak would hand over the 32-bit build.
        let assets = vec![asset("tool-linux32.zip"), asset("tool-linux64.zip")];

        let selected = find_linux_asset_for_arch(&assets, CpuArch::X86_64).unwrap();
        assert_eq!(selected.name, "tool-linux64.zip");

        // linux64 names x86_64, so it is not for an arm64 machine at all.
        assert!(find_linux_asset_for_arch(&assets, CpuArch::Arm64).is_none());
    }

    #[test]
    fn test_linux64_counts_as_the_native_architecture() {
        // Ranked as native (0), so it beats an architecture-unmarked build
        // even though that one's package format would otherwise tie.
        let assets = vec![asset("tool-linux.tar.gz"), asset("tool-linux64.tar.gz")];
        let selected = find_linux_asset_for_arch(&assets, CpuArch::X86_64).unwrap();
        assert_eq!(selected.name, "tool-linux64.tar.gz");
    }

    #[test]
    fn test_unsupported_cpu_markers_are_shared_by_both_pickers() {
        // The list exists once so an architecture learned on one platform
        // cannot stay accepted on the other.
        for marker in ["ppc64el", "s390x", "armel", "loong64"] {
            let macos_asset = vec![asset(&format!("tool-macos-{marker}.dmg"))];
            let linux_asset = vec![asset(&format!("tool-linux-{marker}.deb"))];
            for target_arch in [CpuArch::Arm64, CpuArch::X86_64] {
                assert!(
                    find_macos_asset_for_arch(&macos_asset, target_arch).is_none(),
                    "macOS picker should reject {marker}"
                );
                assert!(
                    find_linux_asset_for_arch(&linux_asset, target_arch).is_none(),
                    "Linux picker should reject {marker}"
                );
            }
        }
    }

    #[test]
    fn test_x86_64_assets_survive_the_unsupported_cpu_list() {
        // A bare "x86" marker would also match inside "x86_64"/"x86-64" and
        // reject every 64-bit Intel asset, so it is deliberately absent.
        for name in ["tool-linux-x86_64.deb", "tool-linux-x86-64.AppImage"] {
            let assets = vec![asset(name)];
            assert!(
                find_linux_asset_for_arch(&assets, CpuArch::X86_64).is_some(),
                "should accept {name}"
            );
        }
        let assets = vec![asset("tool-macos-x86_64.dmg")];
        assert!(find_macos_asset_for_arch(&assets, CpuArch::X86_64).is_some());
    }

    fn parse_release(json: &str) -> ForgeRelease {
        serde_json::from_str(json).unwrap()
    }

    fn parse_releases(json: &str) -> Vec<ForgeRelease> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn latest_without_compatible_asset_falls_back_to_an_older_stable_release() {
        let latest = parse_release(include_str!(
            "../test-data/github/latest-missing-compatible-asset.json"
        ));
        assert!(
            !latest.has_compatible_asset(),
            "fixture should lack a compatible asset"
        );

        let releases = parse_releases(include_str!("../test-data/github/older-stable-valid.json"));
        let selected = select_recent_release(&releases, true).unwrap();

        // The newer prerelease is skipped: the repository publishes stable
        // releases, so the user is tracking the stable channel.
        assert_eq!(selected.tag_name, "v1.9.0");
        assert_eq!(selected.build_release().unwrap().version, "1.9.0");
    }

    #[test]
    fn a_repository_with_only_prereleases_selects_a_compatible_prerelease() {
        let releases = parse_releases(include_str!("../test-data/github/prerelease-only.json"));

        // stable_release_published = false: `releases/latest` 404d.
        let selected = select_recent_release(&releases, false).unwrap();

        assert_eq!(selected.tag_name, "v4.0.0-beta.1");
    }

    #[test]
    fn drafts_are_never_selected() {
        let releases = parse_releases(include_str!("../test-data/github/draft-exclusion.json"));

        let selected = select_recent_release(&releases, true).unwrap();

        assert_eq!(selected.tag_name, "v2.9.0", "a draft was selected");
    }

    #[test]
    fn no_compatible_stable_release_does_not_silently_switch_channel() {
        let releases = parse_releases(include_str!(
            "../test-data/github/no-compatible-stable-release.json"
        ));

        // A compatible prerelease exists, but the repository publishes stable
        // releases, so switching the user onto it would change the channel
        // they track without saying so.
        let error = select_recent_release(&releases, true)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains(&format!(
                "No {PLATFORM_LABEL}-compatible stable release found in the 10 most recent releases"
            )),
            "{error}"
        );
        // The prerelease is there and installable — it is withheld on purpose,
        // not missed.
        assert!(find_compatible_release(&releases, Some(true)).is_some());
    }

    #[test]
    fn without_a_stable_release_any_published_channel_is_acceptable() {
        // `releases/latest` 404ing does not prove the repository is
        // prerelease-only: its stable releases may all still be drafts, or the
        // forge may be inconsistent about the flag. With no stable expectation
        // to protect, the newest installable published release wins whatever
        // its channel.
        let releases = parse_releases(include_str!(
            "../test-data/github/no-compatible-stable-release.json"
        ));

        let selected = select_recent_release(&releases, false).unwrap();

        assert_eq!(selected.tag_name, "v5.1.0-beta.1");
    }

    #[test]
    fn an_empty_release_list_names_the_absence_of_published_releases() {
        // `releases/latest` 404s and the list comes back empty: the repository
        // has published nothing at all, which is a different problem than
        // ten releases that all miss this platform.
        let error = select_recent_release(&[], false).unwrap_err().to_string();

        assert!(error.contains("No published releases"), "{error}");
        assert!(error.contains("draft"), "{error}");
        // It must not claim releases exist but miss the platform.
        assert!(!error.contains("compatible"), "{error}");
    }

    #[test]
    fn a_list_of_only_drafts_names_the_absence_of_published_releases() {
        // Reachable only through a token with write access, since drafts are
        // hidden from everyone else: every entry is a draft, each carrying an
        // asset this machine could install, and none of it is published.
        let releases: Vec<ForgeRelease> = ["v2.0.0", "v1.9.0"]
            .iter()
            .map(|tag| {
                serde_json::from_str(&forgejo_release_json(
                    tag,
                    true,
                    &compatible_asset_name("App"),
                ))
                .unwrap()
            })
            .collect();
        assert!(releases.iter().all(|release| release.draft));

        let error = select_recent_release(&releases, false)
            .unwrap_err()
            .to_string();

        assert!(error.contains("No published releases"), "{error}");
    }

    #[test]
    fn an_all_draft_window_keeps_the_stable_channel_message_when_latest_answered() {
        // With a write-access token the recent window can be all drafts even
        // though `releases/latest` answered — a published release exists by
        // definition, so the failure must stay about the stable channel, not
        // claim the repository has nothing published at all.
        let releases: Vec<ForgeRelease> = ["v2.0.0"]
            .iter()
            .map(|tag| {
                serde_json::from_str(&forgejo_release_json(
                    tag,
                    true,
                    &compatible_asset_name("App"),
                ))
                .unwrap()
            })
            .collect();
        assert!(releases.iter().all(|release| release.draft));

        let error = select_recent_release(&releases, true)
            .unwrap_err()
            .to_string();

        assert!(error.contains("stable"), "{error}");
        assert!(!error.contains("No published releases"), "{error}");
    }
}
