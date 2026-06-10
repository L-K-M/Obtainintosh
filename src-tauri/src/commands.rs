use crate::models::{App, Settings, SourceType};
use crate::sources::GitHubAdapter;
use crate::storage::Storage;
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

#[tauri::command]
pub async fn get_all_apps(state: State<'_, AppState>) -> Result<Vec<App>, String> {
    state.storage.get_all_apps().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_app(
    url: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<App, String> {
    // Detect source type
    let source_type = crate::sources::detect_source_type(&url)
        .ok_or_else(|| "Unsupported source URL".to_string())?;

    // Check if app is already installed
    let (current_version, install_path) = if let Some((path, version)) = crate::installer::detect_installed_app(&name) {
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
    state: State<'_, AppState>,
) -> Result<App, String> {
    // Get existing app
    let mut app = state.storage.get_app(&id)
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

    // Re-detect source type in case URL changed
    app.source_type = crate::sources::detect_source_type(&app.source_url)
        .ok_or_else(|| "Unsupported source URL".to_string())?;

    state.storage.update_app(app.clone()).map_err(|e| e.to_string())?;
    
    Ok(app)
}

#[tauri::command]
pub async fn check_for_updates(
    app_id: Option<String>,
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
    let mut updated_apps = Vec::new();
    
    for mut app in apps {
        // Re-detect installed version; clear stale state if the app was uninstalled
        match crate::installer::detect_installed_app(&app.name) {
            Some((path, version)) => {
                app.current_version = Some(version);
                app.install_path = Some(path);
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
            SourceType::GitLab => {
                Err(anyhow::anyhow!("GitLab support not yet implemented"))
            }
        };
        
        match result {
            Ok(release) => {
                app.latest_version = Some(release.version);
                app.last_checked = Some(chrono::Utc::now().to_rfc3339());
                state.storage.update_app(app.clone()).map_err(|e| e.to_string())?;
                updated_apps.push(app);
            }

            Err(e) => {
                log::error!("Failed to check updates for {}: {}", app.name, e);
                updated_apps.push(app);
            }
        }
    }
    
    Ok(updated_apps)
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    state.storage.get_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_settings(
    settings: Settings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.storage.update_settings(settings).map_err(|e| e.to_string())
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
    
    // Get download URL
    log::info!("Fetching release info for {}", app.name);
    let release = match app.source_type {
        SourceType::GitHub => {
            let adapter = GitHubAdapter::new(settings.github_token);
            adapter
                .get_latest_release(&app.source_url)
                .await
                .map_err(|e| e.to_string())?
        }
        SourceType::GitLab => {
            return Err("GitLab support not yet implemented".to_string());
        }
    };
    
    log::info!("Downloading {} from {}", release.file_name, release.download_url);

    // Download file, emitting progress events for the frontend
    let progress_handle = app_handle.clone();
    let progress_app_id = app_id.clone();
    let progress_file_name = release.file_name.clone();
    let download_result = download_file(&release.download_url, &release.file_name, move |downloaded, total| {
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
    })
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
        .args(&["-R", &download_path])
        .output();
    
    match reveal_output {
        Ok(output) if output.status.success() => {
            log::info!("File revealed in Finder successfully");
        }
        Ok(output) => {
            log::warn!("Failed to reveal in Finder: {}", String::from_utf8_lossy(&output.stderr));
        }
        Err(e) => {
            log::error!("Error running 'open' command: {}", e);
        }
    }
    
    // Update app to mark that we've downloaded the latest version
    let mut updated_app = app;
    updated_app.last_checked = Some(chrono::Utc::now().to_rfc3339());
    state.storage.update_app(updated_app).map_err(|e| e.to_string())?;
    
    // Return success message with instructions
    Ok(format!("Download finished: {}\n\nThe file has been revealed in Finder. Please double-click it to install.", download_path))
}

async fn download_file(
    url: &str,
    filename: &str,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<String> {
    log::debug!("download_file called with url={}, filename={}", url, filename);

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

    // Connect timeout only: a total request timeout would abort large,
    // slow downloads that are progressing fine.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")?;
    let mut response = client
        .get(url)
        .header("User-Agent", crate::sources::USER_AGENT)
        .send()
        .await
        .context("Failed to start download")?;

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


