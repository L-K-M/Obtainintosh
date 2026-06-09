use crate::models::{App, Settings, SourceType};
use crate::sources::GitHubAdapter;
use crate::storage::Storage;
use anyhow::Result;
use std::sync::Arc;
use tauri::State;

pub struct AppState {
    pub storage: Arc<Storage>,
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

    // Prevent tracking the same source twice
    let existing = state.storage.get_all_apps().map_err(|e| e.to_string())?;
    if let Some(dup) = existing
        .iter()
        .find(|a| a.source_url.trim_end_matches('/') == url.trim_end_matches('/'))
    {
        return Err(format!("\"{}\" already tracks this URL", dup.name));
    }

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

    // Storage assigns the UUID and returns the stored record
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
        // Re-detect installed version
        if let Some((path, version)) = crate::installer::detect_installed_app(&app.name) {
            app.current_version = Some(version);
            app.install_path = Some(path);
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
    
    // Download file
    let download_path = download_file(&release.download_url, &release.file_name)
        .await
        .map_err(|e| e.to_string())?;
    
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

async fn download_file(url: &str, filename: &str) -> Result<String> {
    log::debug!("download_file called with url={}, filename={}", url, filename);

    // The filename comes from the GitHub API; refuse anything that could
    // escape the cache directory.
    if filename.is_empty() || filename.contains('/') || filename.contains('\\') || filename.starts_with('.') {
        anyhow::bail!("Refusing to download asset with unsafe file name: {}", filename);
    }

    // Use system temp directory
    let cache_dir = std::env::temp_dir().join("obtainintosh-downloads");

    log::debug!("Cache dir: {:?}", cache_dir);
    std::fs::create_dir_all(&cache_dir)?;

    let file_path = cache_dir.join(filename);
    log::debug!("File path: {:?}", file_path);

    log::debug!("Fetching URL...");
    // Connect timeout only: a total request timeout would abort large,
    // slow downloads that are progressing fine.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()?;
    let mut response = client
        .get(url)
        .send()
        .await?
        .error_for_status()
        .map_err(|e| anyhow::anyhow!("Download failed: {}", e))?;

    // Stream to disk instead of buffering the whole asset in memory
    let mut file = std::fs::File::create(&file_path)?;
    let mut downloaded: u64 = 0;
    while let Some(chunk) = response.chunk().await? {
        std::io::Write::write_all(&mut file, &chunk)?;
        downloaded += chunk.len() as u64;
    }
    std::io::Write::flush(&mut file)?;
    log::debug!("Downloaded {} bytes to {:?}", downloaded, file_path);

    Ok(file_path.to_string_lossy().to_string())
}


