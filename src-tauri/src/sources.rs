use crate::models::{Release, SourceType};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    assets: Vec<GitHubAsset>,
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

    pub async fn get_latest_release(&self, repo_url: &str) -> Result<Release> {
        let (owner, repo) = Self::parse_github_url(repo_url)?;

        let api_url = format!("https://api.github.com/repos/{}/{}/releases/latest", owner, repo);

        let release = match self.fetch_release(&api_url).await {
            Ok(release) => release,
            // /releases/latest 404s for repos that only have prereleases.
            // Fall back to the full release list and take the newest non-draft.
            Err(FetchError::NotFound) => {
                let list_url =
                    format!("https://api.github.com/repos/{}/{}/releases?per_page=10", owner, repo);
                self.fetch_release_list(&list_url)
                    .await?
                    .into_iter()
                    .find(|r| !r.draft)
                    .with_context(|| format!("No releases found for {}/{}", owner, repo))?
            }
            Err(FetchError::Other(e)) => return Err(e),
        };

        // Find macOS-compatible asset
        let asset = Self::find_macos_asset(&release.assets)
            .context("No macOS-compatible asset found")?;

        Ok(Release {
            version: clean_version_tag(&release.tag_name),
            download_url: asset.browser_download_url.clone(),
            file_name: asset.name.clone(),
            file_size: Some(asset.size),
            checksum: None,
            release_notes: release.body,
        })
    }

    fn request(&self, url: &str) -> Result<reqwest::RequestBuilder> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        let mut request = client
            .get(url)
            .header("User-Agent", concat!("Obtainintosh/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");

        if let Some(token) = &self.token {
            if !token.trim().is_empty() {
                request = request.header("Authorization", format!("Bearer {}", token));
            }
        }

        Ok(request)
    }

    async fn fetch_release(&self, url: &str) -> std::result::Result<GitHubRelease, FetchError> {
        let response = self.send(url).await?;
        response
            .json()
            .await
            .context("Failed to parse GitHub release")
            .map_err(FetchError::Other)
    }

    async fn fetch_release_list(&self, url: &str) -> Result<Vec<GitHubRelease>> {
        let response = self.send(url).await.map_err(|e| match e {
            FetchError::NotFound => anyhow::anyhow!("Repository not found"),
            FetchError::Other(e) => e,
        })?;
        response
            .json()
            .await
            .context("Failed to parse GitHub release list")
    }

    async fn send(&self, url: &str) -> std::result::Result<reqwest::Response, FetchError> {
        let response = self
            .request(url)
            .map_err(FetchError::Other)?
            .send()
            .await
            .context("Failed to fetch GitHub release")
            .map_err(FetchError::Other)?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FetchError::NotFound);
        }
        if !response.status().is_success() {
            return Err(FetchError::Other(anyhow::anyhow!(
                "GitHub API error: {}",
                response.status()
            )));
        }

        Ok(response)
    }

    fn parse_github_url(url: &str) -> Result<(String, String)> {
        // Accept forms like:
        //   https://github.com/owner/repo
        //   https://github.com/owner/repo.git
        //   https://github.com/owner/repo/releases (extra segments ignored)
        //   github.com/owner/repo
        let stripped = url
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.");

        let path = stripped
            .strip_prefix("github.com/")
            .context("Invalid GitHub URL: expected github.com/<owner>/<repo>")?;

        // Drop query string / fragment before splitting the path
        let path = path.split(['?', '#']).next().unwrap_or("");

        let mut segments = path.split('/').filter(|s| !s.is_empty());
        let owner = segments
            .next()
            .context("Invalid GitHub URL: missing owner")?;
        let repo = segments
            .next()
            .context("Invalid GitHub URL: missing repository name")?;

        let repo = repo.trim_end_matches(".git");
        if owner.is_empty() || repo.is_empty() {
            anyhow::bail!("Invalid GitHub URL format");
        }

        Ok((owner.to_string(), repo.to_string()))
    }

    fn find_macos_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
        log::debug!("Finding macOS asset from {} candidates", assets.len());
        // Priority order: dmg, pkg, app.tar.gz, tar.gz, zip
        // Note: Generic extensions (zip, tar.gz) require a keyword match to avoid picking up Windows/Linux files
        let extensions = [".dmg", ".pkg", ".app.tar.gz", ".tar.gz", ".zip"];
        let macos_keywords = ["mac", "macos", "darwin", "osx", "universal", "arm64", "aarch64", "x86_64"];

        // Architecture preference: universal first, then the native architecture,
        // then anything else that still looks like a macOS build.
        #[cfg(target_arch = "aarch64")]
        let arch_tiers: [&[&str]; 2] = [&["universal"], &["arm64", "aarch64"]];
        #[cfg(not(target_arch = "aarch64"))]
        let arch_tiers: [&[&str]; 2] = [&["universal"], &["x86_64", "intel", "x64"]];

        for ext in &extensions {
            let candidates: Vec<&GitHubAsset> = assets
                .iter()
                .filter(|a| {
                    let name_lower = a.name.to_lowercase();
                    name_lower.ends_with(ext)
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
            let is_generic = [".zip", ".tar.gz"].contains(ext);
            if !is_generic {
                if let Some(asset) = assets.iter().find(|a| {
                    a.name.to_lowercase().ends_with(ext)
                }) {
                    log::debug!("Selected asset (extension match): {}", asset.name);
                    return Some(asset);
                }
            }
        }

        log::debug!("No suitable asset found");
        None
    }
}

enum FetchError {
    NotFound,
    Other(anyhow::Error),
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
    }

    #[test]
    fn test_parse_github_url_variants() {
        for url in [
            "https://github.com/owner/repo/",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/releases/latest",
            "https://www.github.com/owner/repo",
            "github.com/owner/repo",
            "https://github.com/owner/repo?tab=readme-ov-file",
            "  https://github.com/owner/repo  ",
        ] {
            assert_eq!(
                GitHubAdapter::parse_github_url(url).unwrap(),
                ("owner".to_string(), "repo".to_string()),
                "failed for {}",
                url
            );
        }
    }

    #[test]
    fn test_parse_github_url_invalid() {
        assert!(GitHubAdapter::parse_github_url("https://github.com/owner").is_err());
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
    fn test_extension_requires_dot() {
        // "amdmg" must not satisfy the ".dmg" rule
        let assets = vec![asset("tool-amdmg")];
        assert!(GitHubAdapter::find_macos_asset(&assets).is_none());
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
