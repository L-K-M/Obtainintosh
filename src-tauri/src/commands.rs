use crate::models::{
    bounded_check_message, App, CheckAttempt, CheckAttemptState, Settings, SourceType, SystemColors,
};
use crate::sources::GitHubAdapter;
use crate::storage::{CheckOwnedUpdate, PendingResultApplication, Storage};
use crate::system_colors;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{Emitter, State};

pub struct AppState {
    pub storage: Arc<Storage>,
    pub active_download: Arc<Mutex<Option<String>>>,
}

const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

struct InFlightDownloadGuard {
    active_download: Arc<Mutex<Option<String>>>,
}

impl InFlightDownloadGuard {
    fn acquire(
        operation: &str,
        active_download: Arc<Mutex<Option<String>>>,
    ) -> Result<Self, String> {
        let mut active = lock_active_download(&active_download);
        if let Some(active_operation) = active.as_deref() {
            return Err(format!(
                "A download for {active_operation} is already in progress; wait for it to finish before starting another"
            ));
        }
        *active = Some(operation.to_string());
        drop(active);

        Ok(Self { active_download })
    }
}

impl Drop for InFlightDownloadGuard {
    fn drop(&mut self) {
        let mut active = lock_active_download(&self.active_download);
        active.take();
    }
}

fn lock_active_download(active_download: &Mutex<Option<String>>) -> MutexGuard<'_, Option<String>> {
    active_download.lock().unwrap_or_else(|poisoned| {
        log::warn!("In-flight download state was poisoned; recovering its last known state");
        let guard = poisoned.into_inner();
        active_download.clear_poison();
        guard
    })
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

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcomeState {
    Succeeded,
    Failed,
    Unsupported,
    Skipped,
}

#[derive(serde::Serialize)]
pub struct CheckOutcome {
    app_id: String,
    app_name: String,
    state: CheckOutcomeState,
    message: Option<String>,
}

fn check_outcome(app: &App, state: CheckOutcomeState, message: Option<String>) -> CheckOutcome {
    CheckOutcome {
        app_id: app.id.clone(),
        app_name: app.name.clone(),
        state,
        message,
    }
}

fn skipped_check_outcome(app: &App, application: PendingResultApplication) -> CheckOutcome {
    let message = match application {
        PendingResultApplication::AppRemoved => {
            "The app was removed before its update check finished".to_string()
        }
        PendingResultApplication::DependenciesChanged => {
            "The app changed before its update check finished; the old result was discarded"
                .to_string()
        }
        PendingResultApplication::Applied => unreachable!("an applied result is not skipped"),
    };
    check_outcome(app, CheckOutcomeState::Skipped, Some(message))
}

#[tauri::command]
pub async fn get_all_apps(state: State<'_, AppState>) -> Result<Vec<App>, String> {
    state.storage.get_all_apps().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_app(url: String, name: String, state: State<'_, AppState>) -> Result<App, String> {
    let source_type = crate::sources::validate_new_source(&url).map_err(|e| e.to_string())?;

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
        last_check_attempt: None,
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
    let mut app = state
        .storage
        .get_app(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "App not found".to_string())?;

    let mut source_url = url;
    let source_changed = match app.source_type {
        SourceType::GitLab if app.source_url.trim() == source_url.trim() => {
            source_url = app.source_url.clone();
            false
        }
        _ => {
            let new_identity =
                crate::sources::normalize_repo_url(&source_url).map_err(|e| e.to_string())?;
            let old_identity = crate::sources::normalize_repo_url(&app.source_url).ok();
            app.source_type = SourceType::GitHub;
            old_identity.as_deref() != Some(&new_identity)
        }
    };

    if source_changed {
        // Version info from the old source is meaningless for the new one
        app.latest_version = None;
        app.last_checked = None;
        app.last_check_attempt = None;
    }
    app.source_url = source_url;

    if app.name != name {
        match crate::installer::detect_installed_app(&name) {
            Some((path, version)) => {
                app.current_version = Some(version);
                app.install_path = Some(path);
            }
            None => {
                app.current_version = None;
                app.install_path = None;
            }
        }
    }
    app.name = name;

    state
        .storage
        .update_app(app.clone())
        .map_err(|e| e.to_string())?;

    Ok(app)
}

#[tauri::command]
pub async fn check_for_updates(
    app_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<CheckOutcome>, String> {
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
    let mut outcomes = Vec::new();

    for app in apps {
        // Re-detect installed version; clear stale state if the app was uninstalled
        let (current_version, install_path) =
            match crate::installer::detect_installed_app(&app.name) {
                Some((path, version)) => (Some(version), Some(path)),
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
                    (Some(version), path)
                }
                None => (None, None),
            };

        let (result, failure_state) = match app.source_type {
            SourceType::GitHub => {
                let adapter = GitHubAdapter::new(settings.github_token.clone());
                (
                    adapter.get_latest_release(&app.source_url).await,
                    CheckAttemptState::Failed,
                )
            }
            SourceType::GitLab => (
                Err(anyhow::anyhow!(
                    "This existing GitLab source is unsupported; only GitHub repositories can be checked"
                )),
                CheckAttemptState::Unsupported,
            ),
        };
        let attempted_at = chrono::Utc::now().to_rfc3339();

        match result {
            Ok(release) => {
                let update = CheckOwnedUpdate {
                    current_version,
                    install_path,
                    latest_version: Some(release.version),
                    attempt: CheckAttempt::succeeded(attempted_at),
                };
                match state.storage.apply_check_result(&app, update) {
                    Ok(PendingResultApplication::Applied) => {
                        outcomes.push(check_outcome(&app, CheckOutcomeState::Succeeded, None))
                    }
                    Ok(application) => outcomes.push(skipped_check_outcome(&app, application)),
                    Err(error) => {
                        log::error!("Failed to save update check for {}: {error:#}", app.name);
                        outcomes.push(check_outcome(
                            &app,
                            CheckOutcomeState::Failed,
                            Some(bounded_check_message(&format!(
                                "Could not save the update check: {error}"
                            ))),
                        ));
                    }
                }
            }

            Err(error) => {
                log::error!("Failed to check updates for {}: {error:#}", app.name);
                let message = bounded_check_message(&error.to_string());
                let update = CheckOwnedUpdate {
                    current_version,
                    install_path,
                    latest_version: None,
                    attempt: CheckAttempt::unsuccessful(attempted_at, failure_state, &message),
                };
                match state.storage.apply_check_result(&app, update) {
                    Ok(PendingResultApplication::Applied) => {
                        let state = if failure_state == CheckAttemptState::Unsupported {
                            CheckOutcomeState::Unsupported
                        } else {
                            CheckOutcomeState::Failed
                        };
                        outcomes.push(check_outcome(&app, state, Some(message)));
                    }
                    Ok(application) => outcomes.push(skipped_check_outcome(&app, application)),
                    Err(save_error) => {
                        log::error!(
                            "Failed to save unsuccessful update check for {}: {save_error:#}",
                            app.name
                        );
                        outcomes.push(check_outcome(
                            &app,
                            CheckOutcomeState::Failed,
                            Some(bounded_check_message(&format!(
                                "{message}. The failure state could not be saved: {save_error}"
                            ))),
                        ));
                    }
                }
            }
        }
    }

    Ok(outcomes)
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
    let download_operation = format!("{} ({})", app.name, app.id);
    let _download_guard =
        InFlightDownloadGuard::acquire(&download_operation, Arc::clone(&state.active_download))?;

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
            return Err(
                "This existing GitLab source is unsupported; only GitHub repositories can be downloaded"
                    .to_string(),
            );
        }
    };

    log::info!(
        "Downloading {} from {}",
        release.file_name,
        release.download_url
    );
    let expected_size = release
        .file_size
        .context("GitHub release asset is missing its expected size")
        .map_err(|e| e.to_string())?;

    // Download file, emitting progress events for the frontend
    let progress_handle = app_handle.clone();
    let progress_app_id = app_id.clone();
    let progress_file_name = release.file_name.clone();
    let download_result = download_file(
        &release.download_url,
        &release.file_name,
        expected_size,
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
            downloaded: expected_size,
            total: Some(expected_size),
            done: true,
        },
    );

    let download_path = download_result.map_err(|e| e.to_string())?;

    log::info!("Downloaded to {}", download_path.display());

    // Downloading can happen before the first update check. Persist only the
    // release fields if this is still the same tracked source.
    let metadata_guidance = match state.storage.apply_download_release(
        &app,
        release.version.clone(),
        chrono::Utc::now().to_rfc3339(),
    ) {
        Ok(PendingResultApplication::Applied) => None,
        Ok(PendingResultApplication::AppRemoved) => Some(
            "The app was removed while downloading, so its tracked metadata was not updated. The verified file is still available."
                .to_string(),
        ),
        Ok(PendingResultApplication::DependenciesChanged) => Some(
            "The app's source or update metadata changed while downloading, so the older release was not written to its current metadata. The verified file is still available."
                .to_string(),
        ),
        Err(error) => {
            log::error!("Failed to save downloaded release metadata: {error:#}");
            Some(format!(
                "The verified file is available, but Obtainintosh could not save its release metadata: {}",
                bounded_check_message(&error.to_string())
            ))
        }
    };

    // Instead of trying to install automatically (which requires special entitlements),
    // just reveal the file in Finder so the user can install it manually
    log::info!("Revealing file in Finder...");
    let reveal_message = match reveal_in_finder(&download_path) {
        Ok(()) => {
            log::info!("File revealed in Finder successfully");
            "The file was revealed in Finder. Please double-click it to install.".to_string()
        }
        Err(error) => {
            log::warn!("Failed to reveal in Finder: {error}");
            format!(
                "Finder could not reveal the file ({error}). Please open its containing folder manually."
            )
        }
    };

    // Return success message with instructions
    let metadata_guidance = metadata_guidance
        .map(|guidance| format!("\n\n{guidance}"))
        .unwrap_or_default();
    Ok(format!(
        "Download finished: {}\n\n{reveal_message}{metadata_guidance}",
        download_path.display()
    ))
}

async fn download_file(
    url: &str,
    filename: &str,
    expected_size: u64,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<PathBuf> {
    log::debug!(
        "download_file called with url={}, filename={}",
        url,
        filename
    );

    // Use connection and per-read idle timeouts, but no total request timeout
    // that would abort large, slow downloads that are still progressing.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Failed to build HTTP client")?;
    let mut response = tokio::time::timeout(
        DOWNLOAD_IDLE_TIMEOUT,
        client
            .get(url)
            .header("User-Agent", crate::sources::USER_AGENT)
            .send(),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "Download timed out after {} seconds while waiting for a response",
            DOWNLOAD_IDLE_TIMEOUT.as_secs()
        )
    })?
    .context("Failed to start download")?;

    if !response.status().is_success() {
        anyhow::bail!("Download failed: server returned {}", response.status());
    }

    on_progress(0, Some(expected_size));

    let cache_dir = std::env::temp_dir().join("obtainintosh-downloads");
    ensure_private_cache_directory(&cache_dir)?;
    let paths = download_paths(&cache_dir, filename, uuid::Uuid::new_v4())?;
    create_private_directory(&paths.directory)?;
    let mut partial_download = PartialDownload::new(paths);
    log::debug!(
        "Partial download path: {:?}",
        partial_download.paths.partial
    );

    // Stream to disk instead of buffering the whole asset in memory
    let mut file = tokio::fs::File::create(&partial_download.paths.partial)
        .await
        .context("Failed to create download file")?;
    let mut downloaded: u64 = 0;
    let mut last_reported: u64 = 0;

    loop {
        let chunk = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, response.chunk())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Download timed out after {} seconds without receiving data",
                    DOWNLOAD_IDLE_TIMEOUT.as_secs()
                )
            })?
            .context("Failed while downloading")?;
        let Some(chunk) = chunk else {
            break;
        };

        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .context("Failed to write download file")?;
        downloaded += chunk.len() as u64;
        if downloaded > expected_size {
            anyhow::bail!(
                "Download exceeded the GitHub asset size: received {downloaded} bytes, expected {expected_size} bytes"
            );
        }

        // Throttle progress events to roughly every 256 KB
        if downloaded - last_reported >= 256 * 1024 {
            on_progress(downloaded, Some(expected_size));
            last_reported = downloaded;
        }
    }

    validate_download_size(downloaded, expected_size)?;
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .context("Failed to flush download file")?;
    file.sync_all()
        .await
        .context("Failed to sync download file")?;
    drop(file);

    tokio::fs::rename(
        &partial_download.paths.partial,
        &partial_download.paths.completed,
    )
    .await
    .context("Failed to publish completed download")?;
    partial_download.published = true;

    on_progress(downloaded, Some(expected_size));
    log::debug!(
        "Downloaded {} bytes to {:?}",
        downloaded,
        partial_download.paths.completed
    );

    Ok(partial_download.paths.completed.clone())
}

#[derive(Debug)]
struct DownloadPaths {
    directory: PathBuf,
    partial: PathBuf,
    completed: PathBuf,
}

fn download_paths(root: &Path, filename: &str, operation_id: uuid::Uuid) -> Result<DownloadPaths> {
    // Asset names come from the GitHub API; keep only the final path component
    // so a malicious name can't escape the operation directory.
    let filename = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())
        .context("Invalid asset file name")?;
    let directory = root.join(operation_id.to_string());

    Ok(DownloadPaths {
        partial: directory.join(format!("{filename}.part")),
        completed: directory.join(filename),
        directory,
    })
}

fn create_private_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("Failed to create private download directory at {:?}", path))?;
    set_private_directory_permissions(path)
}

fn ensure_private_cache_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("Failed to create download cache directory at {:?}", path))?;
    set_private_directory_permissions(path)
}

fn set_private_directory_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("Failed to secure download directory at {:?}", path))?;
    }
    Ok(())
}

struct PartialDownload {
    paths: DownloadPaths,
    published: bool,
}

impl PartialDownload {
    fn new(paths: DownloadPaths) -> Self {
        Self {
            paths,
            published: false,
        }
    }
}

impl Drop for PartialDownload {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.paths.partial);
            let _ = std::fs::remove_file(&self.paths.completed);
            let _ = std::fs::remove_dir(&self.paths.directory);
        }
    }
}

fn validate_download_size(actual: u64, expected: u64) -> Result<()> {
    if actual != expected {
        anyhow::bail!("Downloaded {actual} bytes, but GitHub reported an asset size of {expected}");
    }
    Ok(())
}

fn reveal_in_finder(path: &Path) -> Result<()> {
    let output = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .output()
        .context("could not run the macOS 'open' command")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        anyhow::bail!("the macOS 'open' command exited with {}", output.status);
    }
    anyhow::bail!("the macOS 'open' command failed: {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_paths_are_unique_and_keep_asset_names_inside_the_operation_directory() {
        let root = Path::new("download-tests");
        let first = download_paths(
            root,
            "../Example.dmg",
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
        )
        .unwrap();
        let second = download_paths(
            root,
            "../Example.dmg",
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
        )
        .unwrap();

        assert_ne!(first.directory, second.directory);
        assert_ne!(first.partial, second.partial);
        assert_ne!(first.completed, second.completed);
        assert_eq!(first.completed, first.directory.join("Example.dmg"));
        assert_eq!(first.partial, first.directory.join("Example.dmg.part"));
    }

    #[test]
    fn download_paths_collapse_traversal_to_the_asset_file_name() {
        let paths = download_paths(
            Path::new("download-tests"),
            "../../etc/passwd",
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap(),
        )
        .unwrap();

        assert_eq!(paths.completed, paths.directory.join("passwd"));
        assert_eq!(paths.partial, paths.directory.join("passwd.part"));
    }

    #[test]
    fn download_size_must_exactly_match_github_asset_size() {
        assert!(validate_download_size(1024, 1024).is_ok());
        assert!(validate_download_size(1023, 1024).is_err());
        assert!(validate_download_size(1025, 1024).is_err());
    }

    #[test]
    fn downloads_are_globally_serialized_and_can_be_reacquired() {
        let active_download = Arc::new(Mutex::new(None));
        let first = InFlightDownloadGuard::acquire("App One (app-1)", Arc::clone(&active_download))
            .unwrap();

        let error =
            match InFlightDownloadGuard::acquire("App Two (app-2)", Arc::clone(&active_download)) {
                Ok(_) => panic!("a cross-app download should be rejected"),
                Err(error) => error,
            };
        assert_eq!(
            error,
            "A download for App One (app-1) is already in progress; wait for it to finish before starting another"
        );

        drop(first);
        assert!(InFlightDownloadGuard::acquire("App Two (app-2)", active_download).is_ok());
    }

    #[test]
    fn poisoned_download_state_is_recovered() {
        let active_download = Arc::new(Mutex::new(None));
        let poisoned = Arc::clone(&active_download);
        assert!(std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("poison download state for test");
        })
        .join()
        .is_err());

        let guard = InFlightDownloadGuard::acquire("App One (app-1)", Arc::clone(&active_download))
            .unwrap();
        assert!(!active_download.is_poisoned());
        drop(guard);
        assert!(lock_active_download(&active_download).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn cache_and_operation_directories_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "obtainintosh-permissions-test-{}",
            uuid::Uuid::new_v4()
        ));
        let cache = root.join("obtainintosh-downloads");

        ensure_private_cache_directory(&cache).unwrap();
        assert_eq!(
            std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
            0o700
        );

        std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o755)).unwrap();
        ensure_private_cache_directory(&cache).unwrap();
        let operation = cache.join(uuid::Uuid::new_v4().to_string());
        create_private_directory(&operation).unwrap();

        assert_eq!(
            std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&operation).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unpublished_download_cleanup_removes_every_possible_file() {
        let root = std::env::temp_dir().join(format!(
            "obtainintosh-cleanup-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let paths = download_paths(&root, "Example.dmg", uuid::Uuid::new_v4()).unwrap();
        create_private_directory(&paths.directory).unwrap();
        std::fs::write(&paths.partial, b"partial").unwrap();
        std::fs::write(&paths.completed, b"renamed").unwrap();

        let directory = paths.directory.clone();
        let partial = paths.partial.clone();
        let completed = paths.completed.clone();
        drop(PartialDownload::new(paths));

        assert!(!partial.exists());
        assert!(!completed.exists());
        assert!(!directory.exists());
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn published_download_cleanup_leaves_completed_file() {
        let root = std::env::temp_dir().join(format!(
            "obtainintosh-published-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let paths = download_paths(&root, "Example.dmg", uuid::Uuid::new_v4()).unwrap();
        create_private_directory(&paths.directory).unwrap();
        std::fs::write(&paths.completed, b"complete").unwrap();

        let completed = paths.completed.clone();
        let mut download = PartialDownload::new(paths);
        download.published = true;
        drop(download);

        assert!(completed.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
