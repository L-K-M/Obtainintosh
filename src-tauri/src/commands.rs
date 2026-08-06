use crate::models::{
    bounded_check_message, App, CheckAttempt, CheckAttemptState, Settings, SourceType, SystemColors,
};
use crate::sources::{DownloadAuth, ForgeCredentials, ForgejoAdapter, GitHubAdapter};
use crate::storage::{CheckOwnedUpdate, PendingResultApplication, Storage};
use crate::system_colors;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tauri::{Emitter, State};

pub struct AppState {
    pub storage: Arc<Storage>,
    /// The one download allowed at a time, described for the error message.
    pub active_download: Arc<Mutex<Option<String>>>,
}

/// How long a download may make no progress before it is abandoned. There is
/// deliberately no *total* timeout — a large asset on a slow link is fine — so
/// without this a server that accepts the connection and then goes silent
/// would hang the download forever.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How many release lookups may be in flight during a batch check. Bounded
/// rather than unlimited so a long list of tracked programs cannot open a
/// connection per app at once, or trip a forge's rate limiting.
const RELEASE_CHECK_CONCURRENCY: usize = 4;

/// Holds the single global download slot for as long as it is alive, so a
/// second download — for any app — is refused rather than racing the first.
/// Released on drop, which covers the error and early-return paths too.
///
/// The limit is global rather than per-app because the frontend has exactly one
/// progress dialog, fed by a single `downloadProgress` value that each
/// `download-progress` event overwrites. Two concurrent downloads would
/// interleave in it, showing one app's byte count under the other's file name,
/// and the first to finish would close the dialog for both.
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

/// A panic while the slot was held would otherwise poison it and refuse every
/// later download. The tracked value is just a description of what is running,
/// so recovering the last known state is safe.
fn lock_active_download(active_download: &Mutex<Option<String>>) -> MutexGuard<'_, Option<String>> {
    active_download.lock().unwrap_or_else(|poisoned| {
        log::warn!("Active download state was poisoned; recovering its last known state");
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

/// Progress of a batch update check, for the frontend's modal progress
/// dialog. `position - 1` apps are finished when the event arrives, so a
/// progress bar driven by that count never overstates completion — the
/// invariant holds with several checks in flight because `position` is derived
/// from how many have actually completed, not how many have been started.
/// `app_name` is an app still outstanding, so the dialog names something that
/// really is being worked on.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckProgress {
    position: usize,
    total: usize,
    app_name: String,
    done: bool,
}

/// What the frontend is told about one app's check. `Skipped` covers a result
/// that could not be applied because the app moved out from under it.
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckOutcomeState {
    Succeeded,
    Failed,
    Unsupported,
    Skipped,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
            "The program was removed while its update check was running"
        }
        PendingResultApplication::DependenciesChanged => {
            "The program changed while its update check was running, so the result was discarded"
        }
        PendingResultApplication::Applied => unreachable!("applied results are not skipped"),
    };
    check_outcome(app, CheckOutcomeState::Skipped, Some(message.to_string()))
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
        last_check_attempt: None,
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
) -> Result<Vec<CheckOutcome>, String> {
    // Only a batch check pays for indexing the application directories; a
    // single-app check would scan them just to answer one question.
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
    let total = apps.len();

    // Re-detect installed versions for every app in one pass, off the async
    // runtime. This walks /Applications, which is blocking filesystem work: on
    // the runtime it stalls the executor, and once per app it re-reads the
    // same directories N times.
    let detected = detect_apps_for_check(apps.clone(), is_batch).await?;
    let names: Arc<Vec<String>> = Arc::new(apps.iter().map(|app| app.name.clone()).collect());
    // The records as they were before the check ran. apply_check_result
    // compares against these, so a result cannot overwrite an edit the user
    // made while the network work was in flight.
    let snapshots = apps.clone();
    let pending: Vec<App> = apps
        .into_iter()
        .zip(detected)
        .map(|(mut app, (current_version, install_path))| {
            app.current_version = current_version;
            app.install_path = install_path;
            app
        })
        .collect();

    // Fire-and-forget: progress display must never fail a check.
    if total > 0 {
        let _ = app_handle.emit(
            "check-progress",
            CheckProgress {
                position: 1,
                total,
                app_name: names[0].clone(),
                done: false,
            },
        );
    }

    let github_token = settings.github_token;
    let finished = Arc::new(Mutex::new(vec![false; total]));
    let checked = collect_bounded_ordered(
        pending.iter().cloned().enumerate(),
        RELEASE_CHECK_CONCURRENCY,
        |(index, app)| {
            let adapter = GitHubAdapter::new(github_token.clone());
            let handle = app_handle.clone();
            let finished = Arc::clone(&finished);
            let names = Arc::clone(&names);
            async move {
                let result = match app.source_type {
                    SourceType::GitHub => adapter.get_latest_release(&app.source_url).await,
                    SourceType::Forgejo => {
                        ForgejoAdapter::new(forge_credentials(&app))
                            .get_latest_release(&app.source_url)
                            .await
                    }
                    SourceType::GitLab => {
                        Err(anyhow::anyhow!("GitLab support not yet implemented"))
                    }
                };

                // Report against the count actually finished, so the dialog's
                // `position - 1` still means "this many are done" with several
                // checks in flight. The name is the earliest app still
                // outstanding, which is one genuinely being worked on.
                let (position, app_name) = {
                    let mut flags = finished.lock().unwrap_or_else(|e| e.into_inner());
                    flags[index] = true;
                    let done_count = flags.iter().filter(|done| **done).count();
                    let outstanding = flags.iter().position(|done| !*done);
                    let name = names[outstanding.unwrap_or(index)].clone();
                    ((done_count + 1).min(total), name)
                };
                let _ = handle.emit(
                    "check-progress",
                    CheckProgress {
                        position,
                        total,
                        app_name,
                        done: false,
                    },
                );

                (app, result)
            }
        },
    )
    .await;

    let mut outcomes = Vec::with_capacity(total);
    for (joined, snapshot) in checked.into_iter().zip(snapshots) {
        let (app, result) = match joined {
            Ok(checked) => checked,
            Err(error) => {
                // The task died rather than returning an error. Record that
                // against the app and carry on with the rest.
                log::error!("Update check task failed for {}: {error}", snapshot.name);
                (
                    snapshot.clone(),
                    Err(anyhow::anyhow!("Update check failed")),
                )
            }
        };

        let failure_state = match app.source_type {
            SourceType::GitLab => CheckAttemptState::Unsupported,
            _ => CheckAttemptState::Failed,
        };
        let attempted_at = chrono::Utc::now().to_rfc3339();

        // The installed-version re-detection is fresh either way, so it is
        // persisted even when the release lookup failed. What a failure must
        // not do is move latest_version or last_checked — that is what makes a
        // stale figure distinguishable from a current one.
        let (update, outcome) = match result {
            Ok(release) => (
                CheckOwnedUpdate {
                    current_version: app.current_version.clone(),
                    install_path: app.install_path.clone(),
                    latest_version: Some(release.version),
                    attempt: CheckAttempt::succeeded(attempted_at),
                },
                (CheckOutcomeState::Succeeded, None),
            ),
            Err(error) => {
                log::error!("Failed to check updates for {}: {error:#}", app.name);
                let message = bounded_check_message(&error.to_string());
                let outcome_state = if failure_state == CheckAttemptState::Unsupported {
                    CheckOutcomeState::Unsupported
                } else {
                    CheckOutcomeState::Failed
                };
                (
                    CheckOwnedUpdate {
                        current_version: app.current_version.clone(),
                        install_path: app.install_path.clone(),
                        latest_version: None,
                        attempt: CheckAttempt::unsuccessful(attempted_at, failure_state, &message),
                    },
                    (outcome_state, Some(message)),
                )
            }
        };

        match state.storage.apply_check_result(&snapshot, update) {
            Ok(PendingResultApplication::Applied) => {
                outcomes.push(check_outcome(&app, outcome.0, outcome.1))
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

    let _ = app_handle.emit(
        "check-progress",
        CheckProgress {
            position: total,
            total,
            app_name: String::new(),
            done: true,
        },
    );

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

    // Held for the rest of the command: any other download is refused rather
    // than racing this one, and the slot is freed on every exit path including
    // the error returns below. Named so the refusal can say what is running.
    let download_operation = format!("{} ({})", app.name, app.id);
    let _download_guard =
        InFlightDownloadGuard::acquire(&download_operation, Arc::clone(&state.active_download))?;

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

    // Without a size the download cannot be validated, and an unvalidated
    // download is what puts a truncated file in front of the user.
    let expected_size = release
        .file_size
        .context("The release asset is missing its expected size")
        .map_err(|e| e.to_string())?;

    // Download file, emitting progress events for the frontend
    let progress_handle = app_handle.clone();
    let progress_app_id = app_id.clone();
    let progress_file_name = release.file_name.clone();
    let download_result = download_file(
        &release.download_url,
        &release.file_name,
        expected_size,
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
            downloaded: expected_size,
            total: Some(expected_size),
            done: true,
        },
    );

    let download_path = download_result.map_err(|e| e.to_string())?;

    log::info!("Downloaded to {}", download_path.display());

    // Instead of trying to install automatically (which requires special entitlements),
    // just reveal the file in Finder so the user can install it manually
    log::info!("Revealing file in Finder...");
    // The message has to reflect what actually happened: telling the user to
    // look in Finder when the reveal failed sends them hunting for a window
    // that was never opened.
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

    // Update app to mark that we've downloaded the latest version
    let mut updated_app = app;
    updated_app.last_checked = Some(chrono::Utc::now().to_rfc3339());
    // Downloading can happen before the first update check, so record the
    // release we just fetched — otherwise a successful download still shows
    // the latest version as unknown.
    updated_app.latest_version = Some(release.version.clone());
    state
        .storage
        .update_app(updated_app)
        .map_err(|e| e.to_string())?;

    // Return success message with instructions
    Ok(format!(
        "Download finished: {}\n\n{reveal_message}",
        download_path.display()
    ))
}

/// Re-detects the installed version of every app in one blocking pass.
///
/// `use_shared_index` builds the directory listing once for the whole batch;
/// a single-app check skips it and does the direct lookup instead.
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
                    // Obtainintosh itself is running right now, so it always
                    // has a current version — even without an /Applications
                    // install. Prefer the bundle the process runs from (stays
                    // accurate if that bundle is replaced on disk by an
                    // update); the compiled-in version is the fallback for dev
                    // builds outside a bundle.
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

/// Runs `operation` over `inputs` with at most `limit` in flight, returning
/// results in input order regardless of the order they finish in.
///
/// Ordering matters because the caller persists each result against the app it
/// came from; matching them up by position is only sound if the positions are
/// preserved.
async fn collect_bounded_ordered<I, F, Fut, T>(
    inputs: I,
    limit: usize,
    mut operation: F,
) -> Vec<std::result::Result<T, tokio::task::JoinError>>
where
    I: IntoIterator,
    F: FnMut(I::Item) -> Fut,
    Fut: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    assert!(limit > 0, "bounded operation limit must be positive");

    let mut inputs = inputs.into_iter().enumerate();
    let mut in_flight = tokio::task::JoinSet::new();
    let mut task_indices = std::collections::HashMap::new();
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
                // A panicked or cancelled task still owns its slot, so the
                // failure lands against the input it came from rather than
                // shifting every later result up by one.
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

/// Where a single download operation writes. Each operation gets its own
/// directory, so two downloads that happen to share an asset file name — a
/// plain `App.dmg` is common — cannot write over each other.
struct DownloadPaths {
    directory: PathBuf,
    partial: PathBuf,
    completed: PathBuf,
}

/// Owns a download in progress. Until `published` is set, the partial file and
/// its directory are scratch, and dropping cleans them up — so a failed or
/// abandoned download never leaves a truncated file sitting where a complete
/// one belongs.
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

fn download_paths(root: &Path, filename: &str, operation_id: uuid::Uuid) -> Result<DownloadPaths> {
    // Asset names come from the forge API; keep only the final path component
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

fn set_private_directory_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("Failed to secure download directory at {:?}", path))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
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

fn validate_download_size(actual: u64, expected: u64) -> Result<()> {
    if actual != expected {
        anyhow::bail!(
            "Downloaded {actual} bytes, but the forge reported an asset size of {expected}"
        );
    }
    Ok(())
}

/// Reveals a file in Finder, reporting whether it actually worked. The caller
/// tells the user what happened, so a failure here must not be swallowed.
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

/// Credentials stored on a tracked app, for the forges that need them.
fn forge_credentials(app: &App) -> ForgeCredentials {
    ForgeCredentials::new(app.username.clone(), app.access_token.clone())
}

async fn download_file(
    url: &str,
    filename: &str,
    expected_size: u64,
    auth: Option<&DownloadAuth>,
    on_progress: impl Fn(u64, Option<u64>),
) -> Result<PathBuf> {
    log::debug!(
        "download_file called with url={}, filename={}",
        url,
        filename
    );

    // Connect and per-read idle timeouts, but no total request timeout that
    // would abort large, slow downloads that are still progressing.
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

    let mut response = tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, request.send())
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
        // Stop as soon as the response outgrows what the forge advertised,
        // rather than filling the disk with a body that will be rejected.
        if downloaded > expected_size {
            anyhow::bail!(
                "Download exceeded the reported asset size: received {downloaded} bytes, expected {expected_size} bytes"
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

    // Only a fully written, correctly sized file gets the real name.
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

    #[test]
    fn download_paths_are_scoped_to_the_operation_directory() {
        let root = Path::new("/tmp/cache");
        let id = uuid::Uuid::nil();
        let paths = download_paths(root, "App.dmg", id).unwrap();

        assert_eq!(paths.directory, root.join(id.to_string()));
        assert_eq!(paths.completed, paths.directory.join("App.dmg"));
        assert_eq!(paths.partial, paths.directory.join("App.dmg.part"));
    }

    #[test]
    fn download_paths_keep_a_traversing_asset_name_inside_the_directory() {
        let root = Path::new("/tmp/cache");
        let id = uuid::Uuid::nil();

        let paths = download_paths(root, "../../etc/passwd", id).unwrap();

        assert_eq!(paths.completed, paths.directory.join("passwd"));
        assert_eq!(paths.completed.parent(), Some(paths.directory.as_path()));
        assert!(download_paths(root, "..", id).is_err());
    }

    #[test]
    fn two_operations_never_share_a_download_directory() {
        let root = Path::new("/tmp/cache");
        let first = download_paths(root, "App.dmg", uuid::Uuid::new_v4()).unwrap();
        let second = download_paths(root, "App.dmg", uuid::Uuid::new_v4()).unwrap();

        assert_ne!(first.directory, second.directory);
        assert_ne!(first.completed, second.completed);
    }

    #[test]
    fn download_size_must_match_exactly() {
        assert!(validate_download_size(10, 10).is_ok());
        let short = validate_download_size(9, 10).unwrap_err().to_string();
        assert!(short.contains("Downloaded 9 bytes"), "{short}");
        assert!(validate_download_size(11, 10).is_err());
    }

    #[test]
    fn an_unpublished_partial_download_cleans_up_its_directory() {
        let root =
            std::env::temp_dir().join(format!("obtainintosh-dl-test-{}", uuid::Uuid::new_v4()));
        ensure_private_cache_directory(&root).unwrap();
        let paths = download_paths(&root, "App.dmg", uuid::Uuid::new_v4()).unwrap();
        create_private_directory(&paths.directory).unwrap();
        let partial = paths.partial.clone();
        let directory = paths.directory.clone();
        std::fs::write(&partial, b"half a file").unwrap();

        drop(PartialDownload::new(paths));

        assert!(!partial.exists(), "partial file survived");
        assert!(!directory.exists(), "operation directory survived");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_published_download_is_left_in_place() {
        let root =
            std::env::temp_dir().join(format!("obtainintosh-dl-test-{}", uuid::Uuid::new_v4()));
        ensure_private_cache_directory(&root).unwrap();
        let paths = download_paths(&root, "App.dmg", uuid::Uuid::new_v4()).unwrap();
        create_private_directory(&paths.directory).unwrap();
        let completed = paths.completed.clone();
        std::fs::write(&completed, b"a whole file").unwrap();
        let mut download = PartialDownload::new(paths);
        download.published = true;

        drop(download);

        assert_eq!(std::fs::read(&completed).unwrap(), b"a whole file");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn downloads_are_globally_serialized_and_the_slot_is_reusable() {
        let active_download = Arc::new(Mutex::new(None));
        let first = InFlightDownloadGuard::acquire("App One (app-1)", Arc::clone(&active_download))
            .unwrap();

        // A different app is refused too, not just the same one: there is only
        // one progress dialog to report into.
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
        assert!(
            InFlightDownloadGuard::acquire("App Two (app-2)", Arc::clone(&active_download)).is_ok()
        );
    }

    #[test]
    fn a_poisoned_download_slot_still_admits_the_next_download() {
        let active_download = Arc::new(Mutex::new(None));
        let poisoned = Arc::clone(&active_download);
        // Panic while the lock is held, poisoning it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoned.lock().unwrap();
            panic!("poison the slot");
        }));
        assert!(active_download.is_poisoned());

        assert!(InFlightDownloadGuard::acquire("App One (app-1)", active_download).is_ok());
    }

    #[tokio::test]
    async fn bounded_operations_respect_the_limit_and_return_input_order() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let results = collect_bounded_ordered(0..20_usize, 4, |index| {
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
            async move {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // Yield so tasks genuinely overlap rather than running to
                // completion one at a time.
                tokio::task::yield_now().await;
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                in_flight.fetch_sub(1, Ordering::SeqCst);
                index * 10
            }
        })
        .await;

        let values: Vec<usize> = results.into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(values, (0..20).map(|i| i * 10).collect::<Vec<_>>());
        assert!(peak.load(Ordering::SeqCst) <= 4, "limit exceeded");
        assert!(
            peak.load(Ordering::SeqCst) > 1,
            "never actually ran concurrently"
        );
    }

    #[tokio::test]
    async fn a_panicking_bounded_operation_keeps_its_input_slot() {
        let results = collect_bounded_ordered(0..4_usize, 2, |index| async move {
            if index == 1 {
                panic!("task {index} fails");
            }
            index
        })
        .await;

        assert_eq!(results.len(), 4);
        assert_eq!(*results[0].as_ref().unwrap(), 0);
        // The failure stays at index 1 rather than shifting the rest up.
        assert!(
            results[1].is_err(),
            "the panicking task should report an error"
        );
        assert_eq!(*results[2].as_ref().unwrap(), 2);
        assert_eq!(*results[3].as_ref().unwrap(), 3);
    }
}
