use crate::models::{Release, SourceType};
use anyhow::{Context, Result};
use serde::Deserialize;

pub const USER_AGENT: &str = "Obtainintosh/0.1.0";

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
        let client = reqwest::Client::new();
        let mut request = client.get(url).header("User-Agent", USER_AGENT);

        if let Some(token) = &self.token {
            if !token.trim().is_empty() {
                request = request.header("Authorization", format!("Bearer {}", token));
            }
        }

        request
    }

    pub async fn get_latest_release(&self, repo_url: &str) -> Result<Release> {
        let (owner, repo) = Self::parse_github_url(repo_url)?;

        let api_url = format!("https://api.github.com/repos/{}/{}/releases/latest", owner, repo);

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
        let asset = Self::find_macos_asset(&release.assets)
            .context("No macOS-compatible asset found")?;
        
        Ok(Release {
            version: release.tag_name.trim_start_matches('v').to_string(),
            download_url: asset.browser_download_url.clone(),
            file_name: asset.name.clone(),
            file_size: Some(asset.size),
            checksum: None,
            release_notes: release.body,
        })
    }

    async fn get_newest_prerelease(&self, owner: &str, repo: &str) -> Result<GitHubRelease> {
        let api_url = format!("https://api.github.com/repos/{}/{}/releases?per_page=10", owner, repo);

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
        if !host.eq_ignore_ascii_case("github.com") && !host.eq_ignore_ascii_case("www.github.com") {
            anyhow::bail!("Invalid GitHub URL: expected github.com/<owner>/<repo>");
        }

        // Ignore anything past owner/repo (e.g. /releases, /tree/main) as well as
        // query strings, fragments, and a trailing .git.
        let strip = |s: &str| s.split(['?', '#']).next().unwrap_or("").to_string();
        let owner = strip(parts.next().context("GitHub URL is missing the repository owner")?);
        let repo = strip(parts.next().context("GitHub URL is missing the repository name")?);
        let repo = repo.trim_end_matches(".git").to_string();

        if owner.is_empty() || repo.is_empty() {
            anyhow::bail!("Invalid GitHub URL: expected github.com/<owner>/<repo>");
        }

        Ok((owner, repo))
    }

    // todo: prioritize universal binaries > ARM > x86_64
    fn find_macos_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
        log::debug!("Finding macOS asset from {} candidates", assets.len());
        // Priority order: dmg, pkg, app.tar.gz, tar.gz, zip
        // Note: Generic extensions (zip, tar.gz) require a keyword match to avoid picking up Windows/Linux files
        let extensions = ["dmg", "pkg", "app.tar.gz", "tar.gz", "zip"];
        let macos_keywords = ["mac", "macos", "darwin", "osx", "universal", "arm64", "x86_64"];
        
        for ext in &extensions {
            // Match ".{ext}" with the dot so e.g. "checksums-dmg" doesn't count as a dmg
            let suffix = format!(".{}", ext);

            // First try to find with macOS keywords
            if let Some(asset) = assets.iter().find(|a| {
                let name_lower = a.name.to_lowercase();
                name_lower.ends_with(&suffix) &&
                macos_keywords.iter().any(|kw| name_lower.contains(kw))
            }) {
                log::debug!("Selected asset (keyword match): {}", asset.name);
                return Some(asset);
            }

            // Fallback: match extension only, BUT NOT for generic extensions like zip/tar.gz
            // We don't want to accidentally pick up a windows zip just because it's the only zip
            let is_generic = ["zip", "tar.gz"].contains(ext);
            if !is_generic {
                if let Some(asset) = assets.iter().find(|a| {
                    a.name.to_lowercase().ends_with(&suffix)
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
    fn test_parse_github_url_extra_segments() {
        // URLs deeper than the repo root should still resolve to owner/repo
        assert_eq!(
            GitHubAdapter::parse_github_url("https://github.com/owner/repo/releases/latest").unwrap(),
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
    fn test_asset_extension_requires_dot() {
        // "checksums-dmg" must not be mistaken for a .dmg file
        let assets = vec![
            GitHubAsset {
                name: "checksums-dmg".to_string(),
                browser_download_url: "url".to_string(),
                size: 100,
            },
            GitHubAsset {
                name: "MyApp-macos.zip".to_string(),
                browser_download_url: "url".to_string(),
                size: 100,
            },
        ];

        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
        assert_eq!(selected.name, "MyApp-macos.zip");
    }

    #[test]
    fn test_ferrite_asset_selection() {
        // Reproduction case from user: Ferrite v0.2.3
        // Should favor macos tar.gz over windows zip
        let assets = vec![
            GitHubAsset {
                name: "ferrite-linux-x64.tar.gz".to_string(),
                browser_download_url: "url".to_string(),
                size: 100,
            },
            GitHubAsset {
                name: "ferrite-macos-arm64.tar.gz".to_string(),
                browser_download_url: "url".to_string(),
                size: 100,
            },
            GitHubAsset {
                name: "ferrite-macos-x64.tar.gz".to_string(),
                browser_download_url: "url".to_string(),
                size: 100,
            },
            GitHubAsset {
                name: "ferrite-windows-x64.zip".to_string(),
                browser_download_url: "url".to_string(),
                size: 100,
            },
        ];

        let selected = GitHubAdapter::find_macos_asset(&assets);
        assert!(selected.is_some());
        let name = selected.unwrap().name.as_str();
        // It could be arm64 or x64 (both match keywords), but definitely NOT windows zip
        // Since we iterate generic extensions, and tar.gz comes before zip (or after? in modification tar.gz is before zip)
        // AND both tar.gz and zip are generic, so they require keywords.
        // ferrite-windows-x64.zip has "x64" but not "mac"/"macos"/"darwin" etc? 
        // Wait, "x86_64" is in keywords. "x64" is NOT.
        // So windows zip should not match.
        // macos tar.gz has "macos". It should match.
        
        assert!(name.contains("macos"));
        assert!(name.contains("tar.gz"));
    }
}
