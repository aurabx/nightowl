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
use core::mcp::{self, McpRuntimeState, McpStatus};
use core::peers::{NewPeer, Peer, PeerStore, UpdatePeer};
use core::store::{FindLevel, Index, InstanceRow, ScanReport, SeriesRow, StudyRow};
use core::worklist::{NewWorklistEntry, WorklistEntry, WorklistStore};

/// Process-wide state shared across Tauri commands.
///
/// M15 wraps the SCP listener and the MCP runtime state in `Mutex` so
/// `save_config` can swap them in place when the user changes the port,
/// AE Title, store directory, or MCP toggle from Settings — no restart
/// required. The Mutexes are held only across the swap (microseconds),
/// never across an `.await`.
struct AppState {
    config: Mutex<AppConfig>,
    /// SOP Instance index. The SQLite database itself is at the fixed
    /// path `<app config dir>/store.sqlite` and is never swapped. Only
    /// the directory it scans (`AppConfig.store_dir`) is user-mutable,
    /// and that triggers an SCP rebind (so the new `ScpContext.store_dir`
    /// is in force) without touching the index database.
    index: Arc<Index>,
    /// DIMSE SCP listener handle. Replaced by `save_config` when the
    /// AE Title, port, or store directory changes. `None` after a
    /// failed rebind — the previous listener has already been shut
    /// down at that point, and the user has to fix the config and
    /// save again to bring it back.
    listener: Mutex<Option<ListenerHandle>>,
    /// Local MCP server runtime state (M24). Carries the live
    /// `ServerHandle` when running, the reason string when start failed,
    /// or `Disabled` when the user has not opted in. Replaced by
    /// `save_config` when `mcp.enabled` or `mcp.port` changes.
    mcp: Mutex<McpRuntimeState>,
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

/// Returns the live MCP server runtime status. The frontend renders a
/// badge from this in the Settings page so the user sees whether the
/// server is actually bound (and on what address), failed at boot or a
/// hot-reload (with the failure reason), or is intentionally disabled.
#[tauri::command]
fn mcp_status(state: State<'_, AppState>) -> Result<McpStatus, AppError> {
    let guard = state
        .mcp
        .lock()
        .map_err(|_| AppError::Internal("mcp mutex poisoned".to_string()))?;
    Ok(guard.status())
}

/// Validates the new config, persists it to disk, swaps the in-memory
/// copy, and applies any rebinds the change implies — no app restart
/// required (M15).
///
/// Rebind triggers:
/// - SCP listener: `local_ae_title`, `listen_port`, or `store_dir`
///   changed. The new bind is pre-validated (when the port also
///   changed) before the old listener is torn down, so a port that is
///   already in use surfaces as a save error without leaving the user
///   without a listener.
/// - MCP server: `mcp.enabled` or `mcp.port` changed. Old server is
///   gracefully shut down, new server starts in its place. Bind
///   failure transitions `mcp` to the `Failed` state rather than
///   propagating the error — MCP is opt-in ancillary and the rest of
///   the config save should still succeed.
#[tauri::command]
fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    cfg: AppConfig,
) -> Result<AppConfig, AppError> {
    let path = config_path(&app)?;
    save(&path, &cfg)?;

    let old_cfg = {
        let mut guard = state
            .config
            .lock()
            .map_err(|_| AppError::Internal("config mutex poisoned".to_string()))?;
        let old = guard.clone();
        *guard = cfg.clone();
        old
    };

    if cfg.local_ae_title != old_cfg.local_ae_title
        || cfg.listen_port != old_cfg.listen_port
        || cfg.store_dir != old_cfg.store_dir
    {
        // SCP rebind failure is fatal to the save: without a listener
        // the app is non-functional for inbound DICOM, and the user
        // needs to know immediately so they can revert or pick a
        // different port.
        rebind_scp(&app, &state, &cfg)?;
    }

    if cfg.mcp != old_cfg.mcp {
        rebind_mcp(&app, &state, &cfg);
    }

    Ok(cfg)
}

/// Rebuilds the `ScpContext` from current state + the new config, then
/// swaps the listener atomically.
///
/// Strategy:
/// 1. If the port changed, do a throwaway test-bind on the new port
///    first. Catches "port already in use" before tearing down the old
///    listener. Same-port rebinds (AE Title or store_dir only) skip
///    this — the old listener is on that port, so a test would always
///    fail.
/// 2. Shut down the old listener (joins its accept thread → port
///    released).
/// 3. Start the new listener. Failure leaves `state.listener` empty
///    and returns the error to the caller.
fn rebind_scp(
    app: &AppHandle,
    state: &State<'_, AppState>,
    cfg: &AppConfig,
) -> Result<(), AppError> {
    let old_port = {
        let guard = state
            .listener
            .lock()
            .map_err(|_| AppError::Internal("listener mutex poisoned".to_string()))?;
        guard.as_ref().map(|l| l.bind_port())
    };
    if Some(cfg.listen_port) != old_port {
        try_test_bind(cfg.listen_port)?;
    }

    let peers = app
        .try_state::<Arc<PeerStore>>()
        .ok_or_else(|| AppError::Internal("peers state missing".to_string()))?
        .inner()
        .clone();
    let worklist = app
        .try_state::<Arc<WorklistStore>>()
        .ok_or_else(|| AppError::Internal("worklist state missing".to_string()))?
        .inner()
        .clone();
    let scp = Arc::new(ScpContext {
        index: state.index.clone(),
        store_dir: cfg.store_dir.clone(),
        peers,
        local_ae_title: cfg.local_ae_title.clone(),
        worklist,
    });

    let old = {
        let mut guard = state
            .listener
            .lock()
            .map_err(|_| AppError::Internal("listener mutex poisoned".to_string()))?;
        guard.take()
    };
    if let Some(old) = old {
        old.shutdown();
    }

    let new_listener = start_listener(
        cfg.listen_port,
        cfg.local_ae_title.clone(),
        app.clone(),
        scp,
    )?;

    let mut guard = state
        .listener
        .lock()
        .map_err(|_| AppError::Internal("listener mutex poisoned".to_string()))?;
    *guard = Some(new_listener);
    tracing::info!(port = cfg.listen_port, ae = %cfg.local_ae_title, "SCP rebound");
    Ok(())
}

/// Shuts down the previous MCP server (if any) and starts a new one
/// when the new config has it enabled. Errors are absorbed into
/// `McpRuntimeState::Failed` rather than propagated — see the
/// `save_config` doc comment.
fn rebind_mcp(app: &AppHandle, state: &State<'_, AppState>, cfg: &AppConfig) {
    let old = {
        let mut guard = match state.mcp.lock() {
            Ok(g) => g,
            Err(_) => {
                tracing::error!("mcp mutex poisoned during rebind");
                return;
            }
        };
        std::mem::replace(&mut *guard, McpRuntimeState::Disabled)
    };
    if let McpRuntimeState::Running(handle) = old {
        // Block_on is OK here: we are inside a sync Tauri command, and
        // `shutdown` is bounded by the graceful-shutdown future plus
        // serve-task await — typically sub-second.
        tauri::async_runtime::block_on(handle.shutdown());
    }

    let new_state = if cfg.mcp.enabled {
        match build_and_start_mcp(app, state.index.clone(), cfg) {
            Ok(handle) => McpRuntimeState::Running(handle),
            Err(reason) => {
                tracing::error!(error = %reason, port = cfg.mcp.port, "MCP rebind failed");
                McpRuntimeState::Failed(reason)
            }
        }
    } else {
        McpRuntimeState::Disabled
    };

    if let Ok(mut guard) = state.mcp.lock() {
        *guard = new_state;
    }
}

/// Fetches the Arc-shared stores from Tauri-managed state and starts a
/// fresh MCP server. Errors are stringified for storage in
/// `McpRuntimeState::Failed`.
fn build_and_start_mcp(
    app: &AppHandle,
    index: Arc<Index>,
    cfg: &AppConfig,
) -> Result<mcp::ServerHandle, String> {
    let activity = app
        .try_state::<Arc<ActivityLog>>()
        .ok_or_else(|| "activity log state missing".to_string())?
        .inner()
        .clone();
    let peers = app
        .try_state::<Arc<PeerStore>>()
        .ok_or_else(|| "peers state missing".to_string())?
        .inner()
        .clone();
    let worklist = app
        .try_state::<Arc<WorklistStore>>()
        .ok_or_else(|| "worklist state missing".to_string())?
        .inner()
        .clone();
    tauri::async_runtime::block_on(mcp::start_server(
        Some(app.clone()),
        cfg.clone(),
        index,
        peers,
        worklist,
        activity,
    ))
    .map_err(|e| e.to_string())
}

/// Verifies a port is bindable by opening and immediately dropping a
/// throwaway listener. Used by `rebind_scp` to fail fast before tearing
/// down the active listener.
fn try_test_bind(port: u16) -> Result<(), AppError> {
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let _probe = std::net::TcpListener::bind(addr)
        .map_err(|e| AppError::Io(format!("bind {addr} failed: {e}")))?;
    Ok(())
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

#[tauri::command]
fn list_instance_files_for_studies(
    state: State<'_, AppState>,
    study_uids: Vec<String>,
) -> Result<Vec<String>, AppError> {
    state.index.list_instance_files_for_studies(&study_uids)
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

/// Runs every step of app boot, returning the first `AppError` rather
/// than panicking. Split out from the `.setup()` closure so any failure
/// can be reported visibly before the process exits — see the
/// `setup` block in [`run`] and `report_setup_failure`.
///
/// In release this binary is built with `panic = "abort"` and
/// `strip = true`. A bare `?` inside the setup closure therefore
/// produces an unsymbolicated `SIGABRT` with nothing on stderr when the
/// app is launched from Finder. Keeping this body as `Result<_, _>` is
/// what makes the failure recoverable enough to surface to the user.
fn try_setup(app: &mut tauri::App) -> Result<(), AppError> {
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
    let mcp_state = if cfg.mcp.enabled {
        match tauri::async_runtime::block_on(mcp::start_server(
            Some(handle.clone()),
            cfg.clone(),
            idx.clone(),
            peers.clone(),
            worklist.clone(),
            activity.clone(),
        )) {
            Ok(h) => McpRuntimeState::Running(h),
            Err(err) => {
                let reason = err.to_string();
                tracing::error!(error = %reason, port = cfg.mcp.port, "MCP server failed to start");
                McpRuntimeState::Failed(reason)
            }
        }
    } else {
        McpRuntimeState::Disabled
    };

    app.manage(AppState {
        config: Mutex::new(cfg.clone()),
        index: idx.clone(),
        listener: Mutex::new(Some(listener)),
        mcp: Mutex::new(mcp_state),
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
}

/// Surfaces a setup failure to the user before the process exits.
///
/// Three best-effort channels, in order of usefulness:
/// 1. `tracing::error!` — visible on stderr if the app was launched
///    from a terminal or with `RUST_LOG` set.
/// 2. A `startup-error.log` file in the platform app-log directory
///    (`~/Library/Logs/<bundle id>/` on macOS), with the temp dir as
///    a fallback if the app log dir cannot be resolved.
/// 3. On macOS, a native dialog via `osascript` pointing at the log
///    path. This is the channel that actually reaches a user who
///    double-clicked the `.app` from Finder.
///
/// Each step swallows its own error rather than propagating — a
/// failure to report must not block the setup-failure error itself.
fn report_setup_failure(handle: &AppHandle, err: &AppError) {
    tracing::error!(error = %err, "setup failed");

    let log_path: PathBuf = match handle.path().app_log_dir() {
        Ok(dir) => {
            let _ = std::fs::create_dir_all(&dir);
            dir.join("startup-error.log")
        }
        Err(_) => std::env::temp_dir().join("nightowl-startup-error.log"),
    };

    let body = format!("NightOwl startup failed.\n\nError: {err}\n");
    let _ = std::fs::write(&log_path, body);

    #[cfg(target_os = "macos")]
    {
        // AppleScript dialog. The log path has no characters that
        // require escaping under our path resolution (the bundle id
        // and the OS app-log dir both produce only `[A-Za-z0-9./_-]`),
        // so a straight format is safe. We deliberately keep the
        // message body short and point at the log file rather than
        // try to escape an arbitrary error string for AppleScript.
        let script = format!(
            "display dialog \"NightOwl failed to start.\" & return & return & \
             \"See log file:\" & return & \"{}\" \
             with title \"NightOwl\" with icon stop \
             buttons {{\"OK\"}} default button \"OK\"",
            log_path.display(),
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .status();
    }
}

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
            if let Err(err) = try_setup(app) {
                report_setup_failure(app.handle(), &err);
                return Err(err.into());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ping,
            open_url,
            get_config,
            save_config,
            mcp_status,
            rescan_store,
            list_studies,
            list_series_for_study,
            list_instances_for_series,
            total_instance_count,
            list_instance_files_for_studies,
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
