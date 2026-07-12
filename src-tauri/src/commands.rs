use crate::models::{
    bounded_check_message, App, CheckAttempt, CheckAttemptState, Release, Settings, SourceType,
    SystemColors,
};
use crate::sources::GitHubAdapter;
use crate::storage::{CheckOwnedUpdate, PendingResultApplication, Storage};
use crate::system_colors;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, State};
use tokio::task::JoinSet;

pub struct AppState {
    pub storage: Arc<Storage>,
    pub active_download: Arc<Mutex<Option<String>>>,
}

const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const GITHUB_CHECK_CONCURRENCY: usize = 4;

struct InFlightDownloadGuard {
    active_download: Arc<Mutex<Option<String>>>,
}

impl InFlightDownloadGuard {
    fn acquire(
        operation: &str,
        active_download: Arc<Mutex<Option<String>>>,
    ) -> Result<Self, String> {
        let mut active = active_download
            .lock()
            .map_err(|_| "Download state is unavailable".to_string())?;
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
        let mut active = self
            .active_download
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active.take();
    }
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

#[derive(Clone)]
struct PendingCheck {
    app: App,
    current_version: Option<String>,
    install_path: Option<String>,
}

struct CompletedCheck {
    pending: PendingCheck,
    result: Result<Release>,
    failure_state: CheckAttemptState,
    attempted_at: String,
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
    let detection_name = name.clone();
    let installed = tokio::task::spawn_blocking(move || {
        crate::installer::detect_installed_app(&detection_name)
    })
    .await
    .map_err(|error| format!("Installed-app detection failed: {error}"))?;
    let (current_version, install_path) = if let Some((path, version)) = installed {
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
    let app = state
        .storage
        .get_app(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "App not found".to_string())?;

    let installed = if app.name != name {
        let detection_name = name.clone();
        Some(
            tokio::task::spawn_blocking(move || {
                crate::installer::detect_installed_app(&detection_name)
            })
            .await
            .map_err(|error| format!("Installed-app detection failed: {error}"))?,
        )
    } else {
        None
    };

    // Detection yields to other commands, so edit the latest stored record
    // rather than overwriting a check result with the earlier snapshot.
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
        match installed.expect("renaming an app must run installed-app detection") {
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
    let is_batch = app_id.is_none();
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
    let detected = detect_apps_for_check(apps.clone(), is_batch).await?;
    let pending_checks = apps
        .into_iter()
        .zip(detected)
        .map(|(app, (current_version, install_path))| PendingCheck {
            app,
            current_version,
            install_path,
        })
        .collect::<Vec<_>>();
    let github_token = settings.github_token;
    let completed_results = collect_bounded_ordered(
        0..pending_checks.len(),
        GITHUB_CHECK_CONCURRENCY,
        |index| {
            let pending = pending_checks[index].clone();
            let adapter = GitHubAdapter::new(github_token.clone());
            async move {
                let (result, failure_state) = match pending.app.source_type {
                    SourceType::GitHub => (
                        adapter.get_latest_release(&pending.app.source_url).await,
                        CheckAttemptState::Failed,
                    ),
                    SourceType::GitLab => (
                        Err(anyhow::anyhow!(
                            "This existing GitLab source is unsupported; only GitHub repositories can be checked"
                        )),
                        CheckAttemptState::Unsupported,
                    ),
                };
                CompletedCheck {
                    pending,
                    result,
                    failure_state,
                    attempted_at: chrono::Utc::now().to_rfc3339(),
                }
            }
        },
    )
    .await;
    let completed = completed_results
        .into_iter()
        .zip(pending_checks)
        .map(|(result, pending)| match result {
            Ok(completed) => completed,
            Err(error) => {
                log::error!("Update check task failed for {}: {error}", pending.app.name);
                CompletedCheck {
                    pending,
                    result: Err(anyhow::anyhow!("Update check task failed: {error}")),
                    failure_state: CheckAttemptState::Failed,
                    attempted_at: chrono::Utc::now().to_rfc3339(),
                }
            }
        })
        .collect::<Vec<_>>();
    let mut outcomes = Vec::with_capacity(completed.len());

    for completed in completed {
        let CompletedCheck {
            pending,
            result,
            failure_state,
            attempted_at,
        } = completed;
        let PendingCheck {
            app,
            current_version,
            install_path,
        } = pending;

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

async fn detect_apps_for_check(
    apps: Vec<App>,
    use_shared_index: bool,
) -> Result<Vec<(Option<String>, Option<String>)>, String> {
    tokio::task::spawn_blocking(move || {
        let index = use_shared_index.then(crate::installer::InstalledAppIndex::scan);
        apps.iter()
            .map(|app| {
                let installed = match &index {
                    Some(index) => index.detect(&app.name),
                    None => crate::installer::detect_installed_app(&app.name),
                };
                match installed {
                    Some((path, version)) => (Some(version), Some(path)),
                    // Obtainintosh itself is running right now, so it always has a
                    // current version, even without an /Applications install.
                    None if crate::updates::is_self_app(app) => {
                        let (path, version) = match crate::installer::detect_running_bundle() {
                            Some((path, version)) => (Some(path), version),
                            None => (None, env!("CARGO_PKG_VERSION").to_string()),
                        };
                        (Some(version), path)
                    }
                    None => (None, None),
                }
            })
            .collect()
    })
    .await
    .map_err(|error| format!("Installed-app detection failed: {error}"))
}

async fn collect_bounded_ordered<I, F, Fut, T>(
    inputs: I,
    limit: usize,
    mut operation: F,
) -> Vec<std::result::Result<T, tokio::task::JoinError>>
where
    I: IntoIterator,
    F: FnMut(I::Item) -> Fut,
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    assert!(limit > 0, "bounded operation limit must be positive");

    let mut inputs = inputs.into_iter().enumerate();
    let mut in_flight = JoinSet::new();
    let mut task_indices = HashMap::new();
    let mut results = Vec::new();

    for _ in 0..limit {
        let Some((index, input)) = inputs.next() else {
            break;
        };
        results.push(None);
        let task = in_flight.spawn(operation(input));
        task_indices.insert(task.id(), index);
    }

    while let Some(joined) = in_flight.join_next_with_id().await {
        match joined {
            Ok((task_id, result)) => {
                let index = task_indices
                    .remove(&task_id)
                    .expect("completed bounded task must have an input index");
                results[index] = Some(Ok(result));
            }
            Err(error) => {
                let index = task_indices
                    .remove(&error.id())
                    .expect("failed bounded task must have an input index");
                results[index] = Some(Err(error));
            }
        }

        if let Some((next_index, input)) = inputs.next() {
            results.push(None);
            let task = in_flight.spawn(operation(input));
            task_indices.insert(task.id(), next_index);
        }
    }

    debug_assert!(task_indices.is_empty());
    results
        .into_iter()
        .map(|result| result.expect("every bounded operation must complete"))
        .collect()
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
            "The app's source changed while downloading, so the old source's release was not written to its current metadata. The verified file is still available."
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
    std::fs::create_dir_all(&cache_dir).context("Failed to create download directory")?;
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
            validate_download_size(downloaded, expected_size)?;
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
        .with_context(|| format!("Failed to create private download directory at {:?}", path))
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
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[tokio::test]
    async fn bounded_operations_respect_the_limit_and_return_input_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let results = collect_bounded_ordered(0_u64..8, 2, |input| {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            async move {
                let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(now_active, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(8 - input)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                input
            }
        })
        .await
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

        assert_eq!(results, (0_u64..8).collect::<Vec<_>>());
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn bounded_operation_join_failures_keep_their_input_slot() {
        let results = collect_bounded_ordered(0..3, 2, |input| async move {
            assert_ne!(input, 1, "simulated task panic");
            input
        })
        .await;

        assert_eq!(*results[0].as_ref().unwrap(), 0);
        assert!(results[1].as_ref().unwrap_err().is_panic());
        assert_eq!(*results[2].as_ref().unwrap(), 2);
    }
}
