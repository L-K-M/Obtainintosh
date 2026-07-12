use crate::models::{Release, SourceType};
use anyhow::{Context, Result};
use serde::Deserialize;

pub const USER_AGENT: &str = concat!("Obtainintosh/", env!("CARGO_PKG_VERSION"));

const GITHUB_URL_MESSAGE: &str =
    "Only GitHub repository URLs are supported (expected github.com/<owner>/<repo>)";
const RECENT_RELEASE_LIMIT: usize = 10;

/// Canonical GitHub identity shared by storage dedupe, adapter requests, and
/// self-entry matching so accepted URLs and identity comparisons cannot drift.
pub(crate) fn normalize_repo_url(url: &str) -> Result<String> {
    let (owner, repo) = parse_github_url(url)?;
    Ok(format!(
        "https://github.com/{}/{}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    ))
}

fn parse_github_url(url: &str) -> Result<(String, String)> {
    let input = url.trim();
    let candidate = if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let parsed = reqwest::Url::parse(&candidate).context(GITHUB_URL_MESSAGE)?;

    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        anyhow::bail!(GITHUB_URL_MESSAGE);
    }

    let host = parsed.host_str().unwrap_or_default();
    if host.eq_ignore_ascii_case("gitlab.com") || host.eq_ignore_ascii_case("www.gitlab.com") {
        anyhow::bail!(
            "GitLab repositories are not supported; enter a GitHub repository URL instead"
        );
    }
    if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
        anyhow::bail!(GITHUB_URL_MESSAGE);
    }

    let mut parts = parsed.path_segments().context(GITHUB_URL_MESSAGE)?;
    let owner = parts.next().unwrap_or_default();
    let repo_segment = parts.next().unwrap_or_default();
    let repo = if repo_segment
        .get(repo_segment.len().saturating_sub(4)..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".git"))
    {
        &repo_segment[..repo_segment.len() - 4]
    } else {
        repo_segment
    };

    if !valid_github_owner(owner) || !valid_github_repo(repo) {
        anyhow::bail!(GITHUB_URL_MESSAGE);
    }

    Ok((owner.to_string(), repo.to_string()))
}

fn valid_github_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && owner.starts_with(|c: char| c.is_ascii_alphanumeric())
        && owner.ends_with(|c: char| c.is_ascii_alphanumeric())
        && !owner.contains("--")
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn valid_github_repo(repo: &str) -> bool {
    !repo.is_empty()
        && repo.len() <= 100
        && repo != "."
        && repo != ".."
        && repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Shared HTTP client for API calls: connection reuse plus a request timeout
/// so a hung connection can't leave the UI stuck in "Checking..." forever.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client")
    })
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
    assets: Vec<GitHubAsset>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub struct GitHubAdapter {
    token: Option<String>,
}

#[derive(Clone, Copy)]
enum MacArch {
    AppleSilicon,
    Intel,
}

impl GitHubAdapter {
    pub fn new(token: Option<String>) -> Self {
        Self { token }
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let mut request = http_client()
            .get(url)
            .header("User-Agent", USER_AGENT)
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
        let (owner, repo) = parse_github_url(repo_url)?;

        let api_url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            owner, repo
        );

        let response = self
            .get(&api_url)
            .send()
            .await
            .context("Failed to fetch GitHub release")?;

        let latest: Option<GitHubRelease> = if response.status() == reqwest::StatusCode::NOT_FOUND {
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

        if let Some(release) = latest.as_ref() {
            if Self::find_macos_asset(&release.assets).is_some() {
                return Self::build_release(release);
            }
        }

        let releases = self.get_recent_releases(&owner, &repo).await?;
        let release = Self::select_recent_release(&releases, latest.is_some())?;
        Self::build_release(release)
    }

    fn build_release(release: &GitHubRelease) -> Result<Release> {
        let asset = Self::find_macos_asset(&release.assets)
            .context("Selected GitHub release has no macOS-compatible asset")?;

        Ok(Release {
            version: clean_version_tag(&release.tag_name),
            download_url: asset.browser_download_url.clone(),
            file_name: asset.name.clone(),
            file_size: Some(asset.size),
            checksum: None,
            release_notes: release.body.clone(),
        })
    }

    async fn get_recent_releases(&self, owner: &str, repo: &str) -> Result<Vec<GitHubRelease>> {
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

    fn select_recent_release(
        releases: &[GitHubRelease],
        stable_release_published: bool,
    ) -> Result<&GitHubRelease> {
        let stable = if stable_release_published {
            Self::find_compatible_release(releases, false)
        } else {
            None
        };

        stable
            .or_else(|| Self::find_compatible_release(releases, true))
            .with_context(|| {
                format!(
                    "No macOS-compatible release found in the {} most recent GitHub releases",
                    RECENT_RELEASE_LIMIT
                )
            })
    }

    fn find_compatible_release(
        releases: &[GitHubRelease],
        prerelease: bool,
    ) -> Option<&GitHubRelease> {
        releases.iter().find(|release| {
            !release.draft
                && release.prerelease == prerelease
                && Self::find_macos_asset(&release.assets).is_some()
        })
    }

    fn find_macos_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
        let target_arch = if cfg!(target_arch = "aarch64") {
            MacArch::AppleSilicon
        } else {
            MacArch::Intel
        };

        Self::find_macos_asset_for_arch(assets, target_arch)
    }

    fn find_macos_asset_for_arch(
        assets: &[GitHubAsset],
        target_arch: MacArch,
    ) -> Option<&GitHubAsset> {
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
                    "armv7", "armv6", "armhf", "i386", "i686", "powerpc", "ppc64", "riscv64",
                ]
                .iter()
                .any(|marker| has_name_marker(&name, marker))
                    || (has_name_marker(&name, "x86")
                        && !has_name_marker(&name, "x86_64")
                        && !has_name_marker(&name, "x86-64"));

                if unsupported_cpu
                    || matches!(target_arch, MacArch::Intel) && arm64 && !intel64 && !universal
                {
                    return None;
                }

                let architecture_rank = match target_arch {
                    MacArch::AppleSilicon if universal => 0,
                    MacArch::AppleSilicon if arm64 => 1,
                    MacArch::AppleSilicon if !intel64 => 2,
                    MacArch::AppleSilicon => 3,
                    MacArch::Intel if universal => 0,
                    MacArch::Intel if intel64 => 1,
                    MacArch::Intel => 2,
                };

                Some((asset, (architecture_rank, package_rank)))
            })
            .min_by_key(|(_, rank)| *rank)
            .map(|(asset, _)| asset);

        if let Some(asset) = selected {
            log::debug!("Selected compatible macOS asset: {}", asset.name);
        } else {
            log::debug!("No suitable asset found");
        }
        selected
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

pub fn validate_new_source(url: &str) -> Result<SourceType> {
    normalize_repo_url(url)?;
    Ok(SourceType::GitHub)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_release(json: &str) -> GitHubRelease {
        serde_json::from_str(json).unwrap()
    }

    fn parse_releases(json: &str) -> Vec<GitHubRelease> {
        serde_json::from_str(json).unwrap()
    }

    fn asset(name: &str) -> GitHubAsset {
        GitHubAsset {
            name: name.to_string(),
            browser_download_url: "url".to_string(),
            size: 100,
        }
    }

    #[test]
    fn test_parse_github_url_basic() {
        assert_eq!(
            parse_github_url("https://github.com/owner/repo").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            parse_github_url("https://github.com/owner/repo/").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            parse_github_url("github.com/owner/repo").unwrap(),
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
            "https://www.github.com/OWNER/REPO.GIT/releases/latest?download=1#asset",
        ] {
            assert_eq!(
                super::normalize_repo_url(url).unwrap(),
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
            parse_github_url("https://github.com/owner/repo/releases/latest").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            parse_github_url("https://github.com/owner/repo/tree/main/src").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_url_git_suffix_and_query() {
        assert_eq!(
            parse_github_url("https://github.com/owner/repo.git").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            parse_github_url("https://github.com/owner/repo?tab=readme").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
        assert_eq!(
            parse_github_url("https://www.github.com/owner/repo#readme").unwrap(),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn test_parse_github_url_invalid() {
        assert!(parse_github_url("https://github.com/owner").is_err());
        assert!(parse_github_url("https://github.com/").is_err());
        assert!(parse_github_url("https://example.com/owner/repo").is_err());
        assert!(parse_github_url("not a url").is_err());
    }

    #[test]
    fn rejects_lookalike_github_hosts() {
        for url in [
            "https://github.com.evil.example/owner/repo",
            "https://evil-github.com/owner/repo",
            "https://github.com@evil.example/owner/repo",
            "https://evil.example/github.com/owner/repo",
            "https://github.com.evil.example/github.com/owner/repo",
        ] {
            assert!(parse_github_url(url).is_err(), "accepted {url}");
        }
    }

    #[test]
    fn rejects_gitlab_with_github_only_message() {
        let error = validate_new_source("https://gitlab.com/owner/repo")
            .unwrap_err()
            .to_string();
        assert!(error.contains("GitLab repositories are not supported"));
        assert!(error.contains("GitHub"));
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
    fn latest_without_macos_asset_uses_older_stable_release() {
        let latest = parse_release(include_str!(
            "../test-data/github/latest-missing-mac-asset.json"
        ));
        let releases = parse_releases(include_str!("../test-data/github/older-stable-valid.json"));

        assert!(GitHubAdapter::find_macos_asset(&latest.assets).is_none());
        let selected = GitHubAdapter::select_recent_release(&releases, true).unwrap();
        assert_eq!(selected.tag_name, "v1.9.0");

        let selected = GitHubAdapter::select_recent_release(&releases[..2], true).unwrap();
        assert_eq!(selected.tag_name, "v2.1.0-beta.1");
    }

    #[test]
    fn recent_release_selection_excludes_drafts() {
        let releases = parse_releases(include_str!("../test-data/github/draft-exclusion.json"));

        let selected = GitHubAdapter::select_recent_release(&releases, true).unwrap();
        assert_eq!(selected.tag_name, "v2.9.0");
    }

    #[test]
    fn latest_404_falls_back_to_compatible_prerelease() {
        let releases = parse_releases(include_str!("../test-data/github/prerelease-only.json"));

        let selected = GitHubAdapter::select_recent_release(&releases, false).unwrap();
        assert_eq!(selected.tag_name, "v4.0.0-beta.1");
    }

    #[test]
    fn no_compatible_recent_release_has_clear_error() {
        let releases = parse_releases(include_str!(
            "../test-data/github/no-compatible-release.json"
        ));

        let error = GitHubAdapter::select_recent_release(&releases, true)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "No macOS-compatible release found in the 10 most recent GitHub releases"
        );
    }

    #[test]
    fn test_asset_extension_requires_dot() {
        // "checksums-dmg" must not be mistaken for a .dmg file
        let assets = vec![asset("checksums-dmg"), asset("MyApp-macos.zip")];

        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
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

        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
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
        for target_arch in [MacArch::AppleSilicon, MacArch::Intel] {
            let selected = GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).unwrap();
            assert_eq!(selected.name, "tool-universal.dmg");
        }
    }

    #[test]
    fn test_native_arch_preferred_on_every_ci_host() {
        let assets = vec![
            asset("tool-macos-x86_64.dmg"),
            asset("tool-macos-arm64.dmg"),
        ];
        for (target_arch, expected) in [
            (MacArch::AppleSilicon, "tool-macos-arm64.dmg"),
            (MacArch::Intel, "tool-macos-x86_64.dmg"),
        ] {
            let selected = GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).unwrap();
            assert_eq!(selected.name, expected);
        }
    }

    #[test]
    fn test_dmg_fallback_without_keywords() {
        // dmg is macOS-specific, so it may match without a keyword
        let assets = vec![asset("MyTool-1.2.3.dmg")];
        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
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
        for target_arch in [MacArch::AppleSilicon, MacArch::Intel] {
            assert!(GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).is_none());
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
                MacArch::Intel,
                &["tool-macos-arm64.dmg", "tool-macos-x86_64.zip"],
                "tool-macos-x86_64.zip",
            ),
        ];

        for (target_arch, names, expected) in cases {
            let assets: Vec<_> = names.iter().map(|name| asset(name)).collect();
            let selected = GitHubAdapter::find_macos_asset_for_arch(&assets, *target_arch).unwrap();
            assert_eq!(selected.name, *expected);
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
                MacArch::Intel,
                &["tool-windows-x86_64.pkg", "tool-macos-x86_64.zip"],
                "tool-macos-x86_64.zip",
            ),
        ];

        for (target_arch, names, expected) in cases {
            let assets: Vec<_> = names.iter().map(|name| asset(name)).collect();
            let selected = GitHubAdapter::find_macos_asset_for_arch(&assets, *target_arch).unwrap();
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
            GitHubAdapter::find_macos_asset_for_arch(&assets, MacArch::AppleSilicon)
                .unwrap()
                .name,
            "tool-macos-x86_64.dmg"
        );

        let assets = vec![asset("tool-macos-arm64.dmg")];
        assert!(GitHubAdapter::find_macos_asset_for_arch(&assets, MacArch::Intel).is_none());
    }

    #[test]
    fn test_unsupported_cpu_assets_are_rejected() {
        let cases = [
            (MacArch::AppleSilicon, "tool-macos-armv7.dmg"),
            (MacArch::Intel, "tool-macos-arm64.dmg"),
        ];
        for (target_arch, name) in cases {
            let assets = vec![asset(name)];
            assert!(GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).is_none());
        }
    }
}
