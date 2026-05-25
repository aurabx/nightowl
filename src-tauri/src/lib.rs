//! NightOwl Tauri library entrypoint.
//!
//! Per the project's `CLAUDE.md`: business logic lives in `core`; the
//! `#[tauri::command]` functions in this file are thin wrappers that
//! resolve Tauri-specific values (paths, managed state) and call into
//! `core`.

mod core;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use std::path::PathBuf as StdPathBuf;

use core::activity::{ActivityFilter, ActivityLog, ActivityPage};
use core::config::{AppConfig, load_or_default, save};
use core::dimse::{
    scu_echo, scu_find, scu_move, scu_store, start_listener, ListenerHandle, QrRoot,
    ScpContext, ScuEchoResult, ScuFindResult, ScuMoveResult, ScuQueryKeys, ScuStoreOutcome,
};
use core::error::AppError;
use core::mcp;
use core::peers::{NewPeer, Peer, PeerStore, UpdatePeer};
use core::store::{FindLevel, Index, InstanceRow, ScanReport, SeriesRow, StudyRow};
use core::worklist::{NewWorklistEntry, WorklistEntry, WorklistStore};

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
    /// Local MCP server handle (M24). `None` when the server is
    /// disabled in `AppConfig.mcp.enabled` or when bind failed at
    /// boot. Held purely to keep the background serve task alive for
    /// the process lifetime.
    #[allow(dead_code)]
    mcp: Option<mcp::ServerHandle>,
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

fn peers_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(app_config_dir(app)?.join("peers.json"))
}

fn worklist_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(app_config_dir(app)?.join("worklist.sqlite"))
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
fn open_url(url: String) -> Result<(), AppError> {
    core::open_url(&url)
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

// --- Peers (M7) ------------------------------------------------------

#[tauri::command]
fn list_peers(peers: State<'_, Arc<PeerStore>>) -> Result<Vec<Peer>, AppError> {
    peers.list()
}

#[tauri::command]
fn create_peer(
    peers: State<'_, Arc<PeerStore>>,
    peer: NewPeer,
) -> Result<Peer, AppError> {
    peers.create(peer)
}

#[tauri::command]
fn update_peer(
    peers: State<'_, Arc<PeerStore>>,
    peer: UpdatePeer,
) -> Result<Peer, AppError> {
    peers.update(peer)
}

#[tauri::command]
fn delete_peer(peers: State<'_, Arc<PeerStore>>, id: String) -> Result<(), AppError> {
    peers.delete(&id)
}

// --- SCU operations (M8) --------------------------------------------

/// Helper: look a Peer up by id, returning a Validation error if the
/// id is unknown. The frontend already restricts the choice via a
/// dropdown, so the lookup failure path is mostly for hand-typed IDs.
fn resolve_peer(peers: &PeerStore, id: &str) -> Result<Peer, AppError> {
    peers
        .list()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::validation("peer_id", format!("unknown peer id {id}")))
}

#[tauri::command]
async fn scu_echo_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    peers: State<'_, Arc<PeerStore>>,
    peer_id: String,
) -> Result<ScuEchoResult, AppError> {
    let local_ae = read_config(&state)?.local_ae_title;
    let peer = resolve_peer(&peers, &peer_id)?;
    tauri::async_runtime::spawn_blocking(move || scu_echo(&app, &local_ae, &peer))
        .await
        .map_err(|e| AppError::Internal(format!("scu_echo join: {e}")))?
}

#[tauri::command]
async fn scu_find_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    peers: State<'_, Arc<PeerStore>>,
    peer_id: String,
    root: QrRoot,
    level: FindLevel,
    keys: ScuQueryKeys,
) -> Result<ScuFindResult, AppError> {
    let local_ae = read_config(&state)?.local_ae_title;
    let peer = resolve_peer(&peers, &peer_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        scu_find(&app, &local_ae, &peer, root, level, keys)
    })
    .await
    .map_err(|e| AppError::Internal(format!("scu_find join: {e}")))?
}

#[tauri::command]
async fn scu_move_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    peers: State<'_, Arc<PeerStore>>,
    peer_id: String,
    root: QrRoot,
    level: FindLevel,
    keys: ScuQueryKeys,
    destination_ae: String,
) -> Result<ScuMoveResult, AppError> {
    let local_ae = read_config(&state)?.local_ae_title;
    let peer = resolve_peer(&peers, &peer_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        scu_move(&app, &local_ae, &peer, root, level, keys, &destination_ae)
    })
    .await
    .map_err(|e| AppError::Internal(format!("scu_move join: {e}")))?
}

#[tauri::command]
async fn scu_store_cmd(
    app: AppHandle,
    state: State<'_, AppState>,
    peers: State<'_, Arc<PeerStore>>,
    peer_id: String,
    files: Vec<String>,
) -> Result<Vec<ScuStoreOutcome>, AppError> {
    let local_ae = read_config(&state)?.local_ae_title;
    let peer = resolve_peer(&peers, &peer_id)?;
    let paths: Vec<StdPathBuf> = files.into_iter().map(StdPathBuf::from).collect();
    tauri::async_runtime::spawn_blocking(move || scu_store(&app, &local_ae, &peer, &paths))
        .await
        .map_err(|e| AppError::Internal(format!("scu_store join: {e}")))?
}

// --- Worklist (M11) --------------------------------------------------

#[tauri::command]
fn list_worklist(
    worklist: State<'_, Arc<WorklistStore>>,
) -> Result<Vec<WorklistEntry>, AppError> {
    worklist.list()
}

#[tauri::command]
fn create_worklist_entry(
    worklist: State<'_, Arc<WorklistStore>>,
    entry: NewWorklistEntry,
) -> Result<WorklistEntry, AppError> {
    worklist.create(entry)
}

#[tauri::command]
fn update_worklist_entry(
    worklist: State<'_, Arc<WorklistStore>>,
    entry: WorklistEntry,
) -> Result<WorklistEntry, AppError> {
    worklist.update(entry)
}

#[tauri::command]
fn delete_worklist_entry(
    worklist: State<'_, Arc<WorklistStore>>,
    id: String,
) -> Result<(), AppError> {
    worklist.delete(&id)
}

// ---------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Best-effort tracing setup; honour RUST_LOG when present, default
    // to nightowl_lib=info,warn so we see scan summaries without drowning
    // in noise from upstream crates.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nightowl_lib=info,warn".parse().unwrap()),
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
            app.manage(activity.clone());

            // Open the persistent peer list (peers.json next to
            // config.json). Empty on first launch.
            let peers_path = peers_path(handle)?;
            let peers = Arc::new(PeerStore::open(&peers_path)?);
            tracing::info!(
                peers_path = %peers_path.display(),
                peer_count = peers.list().map(|p| p.len()).unwrap_or(0),
                "loaded peers",
            );
            app.manage(peers.clone());

            // Open the worklist store (M11). Its own SQLite file so a
            // user reset of the SOP index does not nuke their
            // worklist data.
            let worklist = Arc::new(WorklistStore::open(&worklist_path(handle)?)?);
            tracing::info!(
                worklist_count = worklist.count().unwrap_or(0),
                "loaded worklist",
            );
            app.manage(worklist.clone());

            // Start the SCP listener AFTER managing the activity log
            // so its startup `SCP listening …` event is persisted.
            // The ScpContext bundles the SOP index (queried by M4 and
            // refreshed by M5) and the on-disk store directory (where
            // M5 writes received SOP Instances).
            let scp = Arc::new(ScpContext {
                index: idx.clone(),
                store_dir: cfg.store_dir.clone(),
                peers: peers.clone(),
                local_ae_title: cfg.local_ae_title.clone(),
                worklist: worklist.clone(),
            });
            let listener = start_listener(
                cfg.listen_port,
                cfg.local_ae_title.clone(),
                handle.clone(),
                scp,
            )?;

            // M24: start the local MCP server when enabled. Treated as
            // ancillary — bind failure logs an error and continues, so
            // a busy port does not block app launch the way an SCP
            // bind failure does.
            let mcp_handle = if cfg.mcp.enabled {
                match tauri::async_runtime::block_on(mcp::start_server(
                    handle.clone(),
                    cfg.clone(),
                    idx.clone(),
                    peers.clone(),
                    worklist.clone(),
                    activity.clone(),
                )) {
                    Ok(h) => Some(h),
                    Err(err) => {
                        tracing::error!(error = %err, port = cfg.mcp.port, "MCP server failed to start");
                        None
                    }
                }
            } else {
                None
            };

            app.manage(AppState {
                config: Mutex::new(cfg.clone()),
                index: idx.clone(),
                listener,
                mcp: mcp_handle,
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
            open_url,
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
            list_peers,
            create_peer,
            update_peer,
            delete_peer,
            scu_echo_cmd,
            scu_find_cmd,
            scu_move_cmd,
            scu_store_cmd,
            list_worklist,
            create_worklist_entry,
            update_worklist_entry,
            delete_worklist_entry,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
