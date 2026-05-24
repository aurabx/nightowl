//! Phantom Tauri library entrypoint.
//!
//! Per the project's `CLAUDE.md`: business logic lives in `core`; the
//! `#[tauri::command]` functions in this file are thin wrappers that
//! resolve Tauri-specific values (paths, managed state) and call into
//! `core`.

mod core;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use core::activity::{ActivityFilter, ActivityLog, ActivityPage};
use core::config::{AppConfig, load_or_default, save};
use core::dimse::{start_listener, ListenerHandle, ScpContext};
use core::error::AppError;
use core::store::{Index, InstanceRow, ScanReport, SeriesRow, StudyRow};

/// Process-wide state shared across Tauri commands.
struct AppState {
    config: Mutex<AppConfig>,
    /// SOP Instance index. Cloning the `Arc` is the cheap way to share
    /// the SQLite-backed store with the background rescan task and
    /// every command thread.
    index: Arc<Index>,
    /// DIMSE SCP listener handle. Kept alive for the process lifetime;
    /// dropping shuts the listener down.
    #[allow(dead_code)]
    listener: ListenerHandle,
}

// The activity log is managed as its own `Arc<ActivityLog>` (separate
// from `AppState`) so `dimse::emit` can fetch it from anywhere it has
// an `AppHandle`, without holding `AppState` for the persist call.

// ---------------------------------------------------------------------
// Tauri-aware path helpers
// ---------------------------------------------------------------------

/// Returns the path to `config.json` inside the platform-specific app
/// config directory.
fn config_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(app_config_dir(app)?.join("config.json"))
}

/// Returns the path to `store.sqlite` inside the platform-specific app
/// config directory. The SOP Instance index lives next to `config.json`.
fn index_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(app_config_dir(app)?.join("store.sqlite"))
}

fn app_config_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .app_config_dir()
        .map_err(|e| AppError::Tauri(e.to_string()))
}

fn home_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .home_dir()
        .map_err(|e| AppError::Tauri(e.to_string()))
}

fn read_config(state: &AppState) -> Result<AppConfig, AppError> {
    state
        .config
        .lock()
        .map(|guard| guard.clone())
        .map_err(|_| AppError::Internal("config mutex poisoned".to_string()))
}

// ---------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------

#[tauri::command]
fn ping() -> &'static str {
    core::ping()
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> Result<AppConfig, AppError> {
    read_config(&state)
}

#[tauri::command]
fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    cfg: AppConfig,
) -> Result<AppConfig, AppError> {
    let path = config_path(&app)?;
    save(&path, &cfg)?;
    let mut guard = state
        .config
        .lock()
        .map_err(|_| AppError::Internal("config mutex poisoned".to_string()))?;
    *guard = cfg.clone();
    Ok(cfg)
}

/// Walks the configured store directory and re-ingests every file.
///
/// Runs the synchronous scan in `spawn_blocking` so the Tauri async
/// runtime stays responsive for the rest of the UI. On completion the
/// `ScanReport` is also broadcast as a `store/scan-completed` event so
/// the Store page can re-fetch without a polling loop.
#[tauri::command]
async fn rescan_store(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ScanReport, AppError> {
    let idx = state.index.clone();
    let dir = read_config(&state)?.store_dir;

    let report = tauri::async_runtime::spawn_blocking(move || idx.rescan_dir(&dir))
        .await
        .map_err(|e| AppError::Internal(format!("rescan join error: {e}")))??;

    let _ = app.emit("store/scan-completed", &report);
    Ok(report)
}

#[tauri::command]
fn list_studies(state: State<'_, AppState>) -> Result<Vec<StudyRow>, AppError> {
    state.index.list_studies()
}

#[tauri::command]
fn list_series_for_study(
    state: State<'_, AppState>,
    study_uid: String,
) -> Result<Vec<SeriesRow>, AppError> {
    state.index.list_series_for_study(&study_uid)
}

#[tauri::command]
fn list_instances_for_series(
    state: State<'_, AppState>,
    series_uid: String,
) -> Result<Vec<InstanceRow>, AppError> {
    state.index.list_instances_for_series(&series_uid)
}

#[tauri::command]
fn total_instance_count(state: State<'_, AppState>) -> Result<i64, AppError> {
    state.index.total_instance_count()
}

// --- Activity log (M9) -----------------------------------------------

#[tauri::command]
fn list_activity(
    log: State<'_, Arc<ActivityLog>>,
    filter: Option<ActivityFilter>,
) -> Result<ActivityPage, AppError> {
    log.list(filter.unwrap_or_default())
}

#[tauri::command]
fn clear_activity(log: State<'_, Arc<ActivityLog>>) -> Result<(), AppError> {
    log.clear()
}

#[tauri::command]
fn activity_count(log: State<'_, Arc<ActivityLog>>) -> Result<i64, AppError> {
    log.count()
}

// ---------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Best-effort tracing setup; honour RUST_LOG when present, default
    // to phantom_lib=info,warn so we see scan summaries without drowning
    // in noise from upstream crates.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "phantom_lib=info,warn".parse().unwrap()),
        )
        .try_init();

    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle();
            let cfg_path = config_path(handle)?;

            if let Some(parent) = cfg_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let home = home_dir(handle)?;
            let default = AppConfig::default_with_home(&home);
            let cfg = load_or_default(&cfg_path, default)?;
            tracing::info!(
                config_path = %cfg_path.display(),
                store_dir = %cfg.store_dir.display(),
                ae_title = %cfg.local_ae_title,
                port = cfg.listen_port,
                "loaded config",
            );
            std::fs::create_dir_all(&cfg.store_dir)?;

            // Open the SOP Instance index alongside the config.
            let idx = Arc::new(Index::open(&index_path(handle)?)?);

            // Open the persistent activity log. We use the same
            // store.sqlite file but our own Connection so the activity
            // mutex does not contend with the SOP-index mutex.
            let activity = Arc::new(ActivityLog::open(&index_path(handle)?)?);
            app.manage(activity);

            // Start the SCP listener AFTER managing the activity log
            // so its startup `SCP listening …` event is persisted.
            // The ScpContext bundles the SOP index (queried by M4 and
            // refreshed by M5) and the on-disk store directory (where
            // M5 writes received SOP Instances).
            let scp = Arc::new(ScpContext {
                index: idx.clone(),
                store_dir: cfg.store_dir.clone(),
            });
            let listener = start_listener(
                cfg.listen_port,
                cfg.local_ae_title.clone(),
                handle.clone(),
                scp,
            )?;

            app.manage(AppState {
                config: Mutex::new(cfg.clone()),
                index: idx.clone(),
                listener,
            });

            // Initial background scan so the Store page is populated
            // without the user having to click anything. Reports back
            // via the `store/scan-completed` event when finished.
            let store_dir = cfg.store_dir.clone();
            let app_handle = handle.clone();
            tauri::async_runtime::spawn_blocking(move || match idx.rescan_dir(&store_dir) {
                Ok(report) => {
                    tracing::info!(
                        seen = report.files_seen,
                        inserted = report.files_inserted,
                        updated = report.files_updated,
                        skipped = report.files_skipped,
                        errored = report.files_errored,
                        elapsed_ms = report.elapsed_ms,
                        "initial scan completed"
                    );
                    let _ = app_handle.emit("store/scan-completed", &report);
                }
                Err(err) => {
                    tracing::error!(error = %err, "initial scan failed");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            get_config,
            save_config,
            rescan_store,
            list_studies,
            list_series_for_study,
            list_instances_for_series,
            total_instance_count,
            list_activity,
            clear_activity,
            activity_count,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
