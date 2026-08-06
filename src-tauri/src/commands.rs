use crate::models::{App, Settings, SourceType, SystemColors};
use crate::sources::{DownloadAuth, ForgeCredentials, ForgejoAdapter, GitHubAdapter};
use crate::storage::Storage;
use crate::system_colors;
use anyhow::{Context, Result};
use std::sync::Arc;
use tauri::{Emitter, State};

pub struct AppState {
    pub storage: Arc<Storage>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    app_id: String,
    file_name: String,
    downloaded: u64,
    total: Option<u64>,
    done: bool,
}

/// Progress of a batch update check, for the frontend's modal progress
/// dialog. Emitted before each app is checked, so `position - 1` apps are
/// finished when the event arrives — a progress bar driven by that count
/// never overstates completion.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckProgress {
    position: usize,
    total: usize,
    app_name: String,
    done: bool,
}

#[tauri::command]
pub async fn get_all_apps(state: State<'_, AppState>) -> Result<Vec<App>, String> {
    state.storage.get_all_apps().map_err(|e| e.to_string())
}

/// The forge a URL belongs to: the type the user picked in the dialog, or a
/// guess from the URL when they left it on "Detect automatically". A private
/// Forgejo instance can be at any host, so detection alone can't be relied on.
fn resolve_source_type(source_type: Option<SourceType>, url: &str) -> Result<SourceType, String> {
    source_type
        .or_else(|| crate::sources::detect_source_type(url))
        .ok_or_else(|| {
            "Unsupported source URL. Pick the source type (for example Forgejo) if the \
             URL is not a github.com repository."
                .to_string()
        })
}

/// Credentials the dialog sent, kept only for a forge that authenticates with
/// them. The dialog already blanks the fields for the other source types, but
/// enforcing it here too means a bug or a hand-made IPC call can't park an
/// application key in the plaintext data file against a source that would
/// never send it. Matched exhaustively on purpose: a new forge has to make
/// this decision rather than inherit one.
fn credentials_for(
    source_type: SourceType,
    username: Option<String>,
    access_token: Option<String>,
) -> ForgeCredentials {
    match source_type {
        SourceType::Forgejo => ForgeCredentials::new(username, access_token),
        SourceType::GitHub | SourceType::GitLab => ForgeCredentials::default(),
    }
}

#[tauri::command]
pub async fn add_app(
    url: String,
    name: String,
    source_type: Option<SourceType>,
    username: Option<String>,
    access_token: Option<String>,
    state: State<'_, AppState>,
) -> Result<App, String> {
    let source_type = resolve_source_type(source_type, &url)?;
    let credentials = credentials_for(source_type, username, access_token);

    // Check if app is already installed
    let (current_version, install_path) =
        if let Some((path, version)) = crate::installer::detect_installed_app(&name) {
            (Some(version), Some(path))
        } else {
            (None, None)
        };

    let app = App {
        id: String::new(),
        name,
        source_type,
        source_url: url,
        current_version,
        latest_version: None,
        install_path,
        last_checked: None,
        username: credentials.username().map(str::to_string),
        access_token: credentials.token().map(str::to_string),
    };

    // Storage assigns the UUID, rejects duplicate source URLs, and returns
    // the stored record
    state.storage.add_app(app).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_app(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.storage.remove_app(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_app(
    id: String,
    url: String,
    name: String,
    source_type: Option<SourceType>,
    username: Option<String>,
    access_token: Option<String>,
    state: State<'_, AppState>,
) -> Result<App, String> {
    // Get existing app
    let mut app = state
        .storage
        .get_app(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "App not found".to_string())?;

    // Update fields
    app.name = name;
    if app.source_url != url {
        // Version info from the old source is meaningless for the new one
        app.latest_version = None;
        app.last_checked = None;
        app.source_url = url;
    }

    // The dialog sends the source type it displayed; fall back to detection so
    // an edit that only changed the URL still re-detects it.
    app.source_type = resolve_source_type(source_type, &app.source_url)?;

    // Credentials come from the dialog every time, so clearing a field there
    // clears the stored one too — as does retyping the app to a forge that
    // does not use them, which drops any key left over from the old type.
    let credentials = credentials_for(app.source_type, username, access_token);
    app.username = credentials.username().map(str::to_string);
    app.access_token = credentials.token().map(str::to_string);

    state
        .storage
        .update_app(app.clone())
        .map_err(|e| e.to_string())?;

    Ok(app)
}

#[tauri::command]
pub async fn check_for_updates(
    app_id: Option<String>,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<App>, String> {
    let apps = if let Some(id) = app_id {
        vec![state
            .storage
            .get_app(&id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "App not found".to_string())?]
    } else {
        state.storage.get_all_apps().map_err(|e| e.to_string())?
    };

    let settings = state.storage.get_settings().map_err(|e| e.to_string())?;
    let total = apps.len();
    let mut updated_apps = Vec::new();

    for (index, mut app) in apps.into_iter().enumerate() {
        // Fire-and-forget: progress display must never fail a check.
        let _ = app_handle.emit(
            "check-progress",
            CheckProgress {
                position: index + 1,
                total,
                app_name: app.name.clone(),
                done: false,
            },
        );
        // Re-detect installed version; clear stale state if the app was uninstalled
        match crate::installer::detect_installed_app(&app.name) {
            Some((path, version)) => {
                app.current_version = Some(version);
                app.install_path = Some(path);
            }
            // Obtainintosh itself is running right now, so it always has a
            // current version — even without an /Applications install. Prefer
            // the bundle the process runs from (stays accurate if that bundle
            // is replaced on disk by an update); the compiled-in version is
            // the fallback for dev builds outside a bundle.
            None if crate::updates::is_self_app(&app) => {
                let (path, version) = match crate::installer::detect_running_bundle() {
                    Some((path, version)) => (Some(path), version),
                    None => (None, env!("CARGO_PKG_VERSION").to_string()),
                };
                app.current_version = Some(version);
                app.install_path = path;
            }
            None => {
                app.current_version = None;
                app.install_path = None;
            }
        }

        let result = match app.source_type {
            SourceType::GitHub => {
                let adapter = GitHubAdapter::new(settings.github_token.clone());
                adapter.get_latest_release(&app.source_url).await
            }
            SourceType::Forgejo => {
                let adapter = ForgejoAdapter::new(forge_credentials(&app));
                adapter.get_latest_release(&app.source_url).await
            }
            SourceType::GitLab => Err(anyhow::anyhow!("GitLab support not yet implemented")),
        };

        match result {
            Ok(release) => {
                app.latest_version = Some(release.version);
                app.last_checked = Some(chrono::Utc::now().to_rfc3339());
            }
            Err(e) => {
                log::error!("Failed to check updates for {}: {}", app.name, e);
            }
        }

        // Persist even when the release check failed: the installed-version
        // re-detection above is fresh either way, and dropping it on a flaky
        // network would leave the stored state stale until the next
        // successful check. last_checked stays untouched on failure.
        state
            .storage
            .update_app(app.clone())
            .map_err(|e| e.to_string())?;
        updated_apps.push(app);
    }

    let _ = app_handle.emit(
        "check-progress",
        CheckProgress {
            position: total,
            total,
            app_name: String::new(),
            done: true,
        },
    );

    Ok(updated_apps)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state.storage.get_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_settings(settings: Settings, state: State<'_, AppState>) -> Result<(), String> {
    state
        .storage
        .update_settings(settings)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_system_colors() -> Result<SystemColors, String> {
    Ok(system_colors::get_system_colors())
}

#[tauri::command]
pub async fn download_and_install(
    app_id: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let app = state
        .storage
        .get_app(&app_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "App not found".to_string())?;

    let settings = state.storage.get_settings().map_err(|e| e.to_string())?;

    // Get download URL, plus whatever the asset download itself needs to
    // authenticate with (a private Forgejo repository serves its assets behind
    // the same credentials the API call used).
    log::info!("Fetching release info for {}", app.name);
    let (release, download_auth) = match app.source_type {
        SourceType::GitHub => {
            let adapter = GitHubAdapter::new(settings.github_token);
            let release = adapter
                .get_latest_release(&app.source_url)
                .await
                .map_err(|e| e.to_string())?;
            (release, None)
        }
        SourceType::Forgejo => {
            let adapter = ForgejoAdapter::new(forge_credentials(&app));
            let release = adapter
                .get_latest_release(&app.source_url)
                .await
                .map_err(|e| e.to_string())?;
            let auth = adapter
                .download_auth(&app.source_url)
                .map_err(|e| e.to_string())?;
            (release, Some(auth))
        }
        SourceType::GitLab => {
            return Err("GitLab support not yet implemented".to_string());
        }
    };

    log::info!(
        "Downloading {} from {}",
        release.file_name,
        release.download_url
    );

    // Download file, emitting progress events for the frontend
    let progress_handle = app_handle.clone();
    let progress_app_id = app_id.clone();
    let progress_file_name = release.file_name.clone();
    let download_result = download_file(
        &release.download_url,
        &release.file_name,
        download_auth.as_ref(),
        move |downloaded, total| {
            let _ = progress_handle.emit(
                "download-progress",
                DownloadProgress {
                    app_id: progress_app_id.clone(),
                    file_name: progress_file_name.clone(),
                    downloaded,
                    total,
                    done: false,
                },
            );
        },
    )
    .await;

    // Always emit a final "done" event so the frontend can close its progress UI
    let _ = app_handle.emit(
        "download-progress",
        DownloadProgress {
            app_id: app_id.clone(),
            file_name: release.file_name.clone(),
            downloaded: release.file_size.unwrap_or(0),
            total: release.file_size,
            done: true,
        },
    );

    let download_path = download_result.map_err(|e| e.to_string())?;

    log::info!("Downloaded to {}", download_path);

    // Instead of trying to install automatically (which requires special entitlements),
    // just reveal the file in Finder so the user can install it manually
    log::info!("Revealing file in Finder...");

    // Use 'open -R' to reveal the file in Finder
    let reveal_output = std::process::Command::new("open")
        .args(["-R", &download_path])
        .output();

    match reveal_output {
        Ok(output) if output.status.success() => {
            log::info!("File revealed in Finder successfully");
        }
        Ok(output) => {
            log::warn!(
                "Failed to reveal in Finder: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            log::error!("Error running 'open' command: {}", e);
        }
    }

    // Update app to mark that we've downloaded the latest version
    let mut updated_app = app;
    updated_app.last_checked = Some(chrono::Utc::now().to_rfc3339());
    state
        .storage
        .update_app(updated_app)
        .map_err(|e| e.to_string())?;

    // Return success message with instructions
    Ok(format!("Download finished: {}\n\nThe file has been revealed in Finder. Please double-click it to install.", download_path))
}

/// Credentials stored on a tracked app, for the forges that need them.
fn forge_credentials(app: &App) -> ForgeCredentials {
    ForgeCredentials::new(app.username.clone(), app.access_token.clone())
}

async fn download_file(
    url: &str,
    filename: &str,
    auth: Option<&DownloadAuth>,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<String> {
    log::debug!(
        "download_file called with url={}, filename={}",
        url,
        filename
    );

    // Asset names come from the GitHub API; keep only the final path component
    // so a malicious name can't escape the cache directory
    let filename = std::path::Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .context("Invalid asset file name")?;

    // Use system temp directory
    let cache_dir = std::env::temp_dir().join("obtainintosh-downloads");
    std::fs::create_dir_all(&cache_dir)?;

    let file_path = cache_dir.join(filename);
    log::debug!("File path: {:?}", file_path);

    // Deliberately not sources::http_client(): its total request timeout
    // would abort large, slow downloads that are progressing fine, so this
    // client has a connect timeout only.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .user_agent(crate::sources::USER_AGENT)
        .build()
        .context("Failed to build HTTP client")?;

    // Private-instance assets need the same credentials the release lookup
    // used. `DownloadAuth` only attaches them to the instance's own origin;
    // reqwest additionally drops the header if a redirect leaves that host.
    let mut request = client.get(url);
    if let Some(auth) = auth {
        request = auth.authorize(request, url);
    }

    let mut response = request.send().await.context("Failed to start download")?;

    if !response.status().is_success() {
        anyhow::bail!("Download failed: server returned {}", response.status());
    }

    let total = response.content_length();
    on_progress(0, total);

    // Stream to disk instead of buffering the whole asset in memory
    let mut file = tokio::fs::File::create(&file_path)
        .await
        .context("Failed to create download file")?;
    let mut downloaded: u64 = 0;
    let mut last_reported: u64 = 0;

    while let Some(chunk) = response.chunk().await.context("Failed while downloading")? {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("Failed to write download file")?;
        downloaded += chunk.len() as u64;

        // Throttle progress events to roughly every 256 KB
        if downloaded - last_reported >= 256 * 1024 {
            on_progress(downloaded, total);
            last_reported = downloaded;
        }
    }

    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    on_progress(downloaded, total);
    log::debug!("Downloaded {} bytes to {:?}", downloaded, file_path);

    Ok(file_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_kept_only_for_the_forge_that_uses_them() {
        let entered = || (Some("alice".to_string()), Some("key123".to_string()));

        let (username, access_token) = entered();
        let forgejo = credentials_for(SourceType::Forgejo, username, access_token);
        assert_eq!(forgejo.username(), Some("alice"));
        assert_eq!(forgejo.token(), Some("key123"));

        // Retyping an app to a forge that authenticates differently drops the
        // key rather than leaving it in the plaintext data file.
        for source_type in [SourceType::GitHub, SourceType::GitLab] {
            let (username, access_token) = entered();
            let dropped = credentials_for(source_type, username, access_token);
            assert_eq!(dropped, ForgeCredentials::default(), "{source_type:?}");
        }
    }

    #[test]
    fn source_type_falls_back_to_detection_only_when_unset() {
        // An explicit pick wins, so a private instance on an unremarkable host
        // is reachable at all.
        assert_eq!(
            resolve_source_type(
                Some(SourceType::Forgejo),
                "https://git.example.internal/o/r"
            ),
            Ok(SourceType::Forgejo)
        );
        assert_eq!(
            resolve_source_type(None, "https://github.com/owner/repo"),
            Ok(SourceType::GitHub)
        );
        assert!(resolve_source_type(None, "https://git.example.internal/o/r").is_err());
    }
}
