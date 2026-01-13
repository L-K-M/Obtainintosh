use crate::models::{Release, SourceType};
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
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
        
        let client = reqwest::Client::new();
        let mut request = client
            .get(&api_url)
            .header("User-Agent", "Obtainintosh/0.1.0");
        
        if let Some(token) = &self.token {
            if !token.trim().is_empty() {
                request = request.header("Authorization", format!("Bearer {}", token));
            }
        }
        
        let response = request
            .send()
            .await
            .context("Failed to fetch GitHub release")?;
        
        if !response.status().is_success() {
            anyhow::bail!("GitHub API error: {}", response.status());
        }
        
        let release: GitHubRelease = response
            .json()
            .await
            .context("Failed to parse GitHub release")?;
        
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

    fn parse_github_url(url: &str) -> Result<(String, String)> {
        let url = url.trim_end_matches('/');
        let parts: Vec<&str> = url.split('/').collect();
        
        if parts.len() < 2 {
            anyhow::bail!("Invalid GitHub URL format");
        }
        
        let owner = parts[parts.len() - 2].to_string();
        let repo = parts[parts.len() - 1].to_string();
        
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
            // First try to find with macOS keywords
            if let Some(asset) = assets.iter().find(|a| {
                let name_lower = a.name.to_lowercase();
                name_lower.ends_with(ext) && 
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

    // Note: The following `test_extract_version_*` tests are commented out
    // because the `extract_version_from_text` function is not present in the
    // provided code document. Adding them would result in a compilation error.
    // If this function is intended to be added, it should be provided separately.

    // #[test]
    // fn test_extract_version_basic() {
    //     assert_eq!(GitHubAdapter::extract_version_from_text("v1.2.3"), Some("1.2.3".to_string()));
    //     assert_eq!(GitHubAdapter::extract_version_from_text("1.2.3"), Some("1.2.3".to_string()));
    // }

    // #[test]
    // fn test_extract_version_with_text() {
    //     assert_eq!(GitHubAdapter::extract_version_from_text("Release v1.2.3"), Some("1.2.3".to_string()));
    //      assert_eq!(GitHubAdapter::extract_version_from_text("Stable 1.25.0"), Some("1.25.0".to_string()));
    // }

    // #[test]
    // fn test_extract_version_extended() {
    //     assert_eq!(GitHubAdapter::extract_version_from_text("Continuous v1.25.0.98"), Some("1.25.0.98".to_string()));
    //     assert_eq!(GitHubAdapter::extract_version_from_text("Build 1.25.0.90"), Some("1.25.0.90".to_string()));
    // }

    // #[test]
    // fn test_extract_no_version() {
    //     assert_eq!(GitHubAdapter::extract_version_from_text("Just some text"), None);
    //     assert_eq!(GitHubAdapter::extract_version_from_text("v abc"), None);
    // }

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
