use crate::models::{Release, SourceType};
use anyhow::{Context, Result};
use serde::Deserialize;

pub const USER_AGENT: &str = concat!("Obtainintosh/", env!("CARGO_PKG_VERSION"));

/// Canonical form for "is this the same repository?" comparisons: lowercased,
/// `http://` folded into `https://`, ignoring a trailing slash and a trailing
/// `.git`. Shared by `Storage::add_app`'s dedupe and `updates::is_self_app` so
/// the two checks can't drift apart.
pub(crate) fn normalize_repo_url(url: &str) -> String {
    let url = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .trim_end_matches('/')
        .to_ascii_lowercase();
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url,
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

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
    assets: Vec<GitHubAsset>,
    #[serde(default)]
    draft: bool,
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

        let release: GitHubRelease = if response.status() == reqwest::StatusCode::NOT_FOUND {
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

        // Find macOS-compatible asset
        let asset =
            Self::find_macos_asset(&release.assets).context("No macOS-compatible asset found")?;

        Ok(Release {
            version: clean_version_tag(&release.tag_name),
            download_url: asset.browser_download_url.clone(),
            file_name: asset.name.clone(),
            file_size: Some(asset.size),
            checksum: None,
            release_notes: release.body,
        })
    }

    async fn get_newest_prerelease(&self, owner: &str, repo: &str) -> Result<GitHubRelease> {
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

        let releases: Vec<GitHubRelease> = response
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

    fn find_macos_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
        log::debug!("Finding macOS asset from {} candidates", assets.len());
        // Priority order: dmg, pkg, app.tar.gz, tar.gz, zip
        // Note: Generic extensions (zip, tar.gz) require a keyword match to avoid picking up Windows/Linux files
        let extensions = ["dmg", "pkg", "app.tar.gz", "tar.gz", "zip"];
        let macos_keywords = [
            "mac",
            "macos",
            "darwin",
            "osx",
            "universal",
            "arm64",
            "aarch64",
            "x86_64",
        ];

        // Architecture preference: universal first, then the native architecture,
        // then anything else that still looks like a macOS build.
        #[cfg(target_arch = "aarch64")]
        let arch_tiers: [&[&str]; 2] = [&["universal"], &["arm64", "aarch64"]];
        #[cfg(not(target_arch = "aarch64"))]
        let arch_tiers: [&[&str]; 2] = [&["universal"], &["x86_64", "intel", "x64"]];

        for ext in &extensions {
            // Match ".{ext}" with the dot so e.g. "checksums-dmg" doesn't count as a dmg
            let suffix = format!(".{}", ext);

            // Candidates with macOS keywords, best architecture first
            let candidates: Vec<&GitHubAsset> = assets
                .iter()
                .filter(|a| {
                    let name_lower = a.name.to_lowercase();
                    name_lower.ends_with(&suffix)
                        && macos_keywords.iter().any(|kw| name_lower.contains(kw))
                })
                .collect();

            for tier in &arch_tiers {
                if let Some(asset) = candidates.iter().copied().find(|a| {
                    let name_lower = a.name.to_lowercase();
                    tier.iter().any(|kw| name_lower.contains(kw))
                }) {
                    log::debug!("Selected asset (arch preference): {}", asset.name);
                    return Some(asset);
                }
            }

            if let Some(asset) = candidates.first().copied() {
                log::debug!("Selected asset (keyword match): {}", asset.name);
                return Some(asset);
            }

            // Fallback: match extension only, BUT NOT for generic extensions like zip/tar.gz
            // We don't want to accidentally pick up a windows zip just because it's the only zip
            let is_generic = ["zip", "tar.gz"].contains(ext);
            if !is_generic {
                if let Some(asset) = assets
                    .iter()
                    .find(|a| a.name.to_lowercase().ends_with(&suffix))
                {
                    log::debug!("Selected asset (extension match): {}", asset.name);
                    return Some(asset);
                }
            }
        }

        log::debug!("No suitable asset found");
        None
    }
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

pub fn detect_source_type(url: &str) -> Option<SourceType> {
    if url.contains("github.com") {
        Some(SourceType::GitHub)
    } else if url.contains("gitlab.com") || url.contains("gitlab") {
        Some(SourceType::GitLab)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "http://github.com/owner/repo",
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
        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
        assert_eq!(selected.name, "tool-universal.dmg");
    }

    #[test]
    fn test_native_arch_preferred_over_listing_order() {
        let assets = vec![
            asset("tool-macos-x86_64.dmg"),
            asset("tool-macos-arm64.dmg"),
        ];
        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
        #[cfg(target_arch = "aarch64")]
        assert_eq!(selected.name, "tool-macos-arm64.dmg");
        #[cfg(not(target_arch = "aarch64"))]
        assert_eq!(selected.name, "tool-macos-x86_64.dmg");
    }

    #[test]
    fn test_dmg_fallback_without_keywords() {
        // dmg is macOS-specific, so it may match without a keyword
        let assets = vec![asset("MyTool-1.2.3.dmg")];
        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
        assert_eq!(selected.name, "MyTool-1.2.3.dmg");
    }

    #[test]
    fn test_generic_zip_requires_keyword() {
        let assets = vec![asset("tool-windows.zip"), asset("tool-linux.tar.gz")];
        assert!(GitHubAdapter::find_macos_asset(&assets).is_none());
    }
}
