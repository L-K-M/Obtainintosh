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
        let target_arch = if cfg!(target_arch = "aarch64") {
            MacArch::AppleSilicon
        } else {
            MacArch::X86_64
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
        for target_arch in [MacArch::AppleSilicon, MacArch::X86_64] {
            let selected = GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).unwrap();
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
            let selected = GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).unwrap();
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

        let selected = GitHubAdapter::find_macos_asset(&assets).unwrap();
        let expected = GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).unwrap();
        assert_eq!(selected.name, expected.name);
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
        for target_arch in [MacArch::AppleSilicon, MacArch::X86_64] {
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
                MacArch::X86_64,
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
    fn test_equal_scores_use_asset_name_as_tiebreaker() {
        for names in [
            ["tool-macos-x86_64-zeta.dmg", "tool-macos-x86_64-alpha.dmg"],
            ["tool-macos-x86_64-alpha.dmg", "tool-macos-x86_64-zeta.dmg"],
        ] {
            let assets: Vec<_> = names.iter().map(|name| asset(name)).collect();
            let selected =
                GitHubAdapter::find_macos_asset_for_arch(&assets, MacArch::X86_64).unwrap();
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
        assert!(GitHubAdapter::find_macos_asset_for_arch(&assets, MacArch::X86_64).is_none());
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
            assert!(GitHubAdapter::find_macos_asset_for_arch(&assets, target_arch).is_none());
        }
    }
}
