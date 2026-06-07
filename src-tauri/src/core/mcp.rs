//! Local MCP (Model Context Protocol) server (M24).
//!
//! Exposes a curated subset of NightOwl's capabilities — read-only views
//! of the SOP index, peers, worklist and activity log, plus the four
//! active SCU operations (C-ECHO, C-FIND, C-MOVE, C-STORE) — as MCP
//! tools that external LLM clients can call.
//!
//! The server runs inside the existing Tauri tokio runtime via an axum
//! router that nests `rmcp::transport::StreamableHttpService` at `/mcp`.
//! It binds to `127.0.0.1:<config.mcp.port>` so only processes on the
//! local machine can connect; there is no other authentication. The
//! "developer tool, no TLS, no auth" posture is consistent with the
//! rest of the app (see `PLAN.md` decision log).
//!
//! The MCP layer is a thin wrapper over the same `core::dimse`,
//! `core::store`, `core::peers`, `core::worklist` and `core::activity`
//! functions the existing Tauri commands call. It does not duplicate any
//! business logic.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::activity::{ActivityFilter, ActivityLog};
use super::config::AppConfig;
use super::dimse::{self, QrRoot, ScuQueryKeys};
use super::error::AppError;
use super::peers::PeerStore;
use super::store::{FindLevel, Index};
use super::worklist::WorklistStore;

// ---------------------------------------------------------------------
// Handle returned from `start_server`
// ---------------------------------------------------------------------

/// Owns the background task serving the MCP HTTP endpoint.
///
/// `shutdown` carries a oneshot sender wired to axum's graceful
/// shutdown future; sending on it causes the serve loop to stop
/// accepting and drain. `serve_task` is awaited after the signal so
/// the port is fully released before this returns. Both are wrapped
/// in `Mutex<Option<...>>` so `shutdown()` is idempotent via a
/// `&self` receiver — second calls find empty options and return
/// quickly.
pub struct ServerHandle {
    pub bind_addr: SocketAddr,
    shutdown: std::sync::Mutex<Option<oneshot::Sender<()>>>,
    serve_task: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl ServerHandle {
    /// Stops the server gracefully: signals axum to exit its serve
    /// loop, waits for the serve task to finish so the bound port is
    /// fully released, then returns. Safe to call multiple times.
    pub async fn shutdown(&self) {
        let tx = self.shutdown.lock().ok().and_then(|mut g| g.take());
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
        let task = self.serve_task.lock().ok().and_then(|mut g| g.take());
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

/// Runtime state of the MCP server, held by the Tauri-managed
/// `AppState`. The serialisable projection used by the frontend is
/// `McpStatus` below.
pub enum McpRuntimeState {
    /// `AppConfig.mcp.enabled` was false at boot.
    Disabled,
    /// Bind succeeded; the server is serving requests.
    Running(ServerHandle),
    /// Enabled in config but bind / start failed at boot. Held so the
    /// UI can surface the error inline rather than silently saying
    /// "disabled".
    Failed(String),
}

/// Serialisable view of `McpRuntimeState` returned to the frontend via
/// the `mcp_status` Tauri command. Discriminated by `state` for the
/// same TypeScript pattern the existing `AppError` discriminated union
/// uses.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum McpStatus {
    Disabled,
    Running { bind_addr: String },
    Failed { reason: String },
}

impl McpRuntimeState {
    pub fn status(&self) -> McpStatus {
        match self {
            McpRuntimeState::Disabled => McpStatus::Disabled,
            McpRuntimeState::Running(handle) => McpStatus::Running {
                bind_addr: handle.bind_addr.to_string(),
            },
            McpRuntimeState::Failed(reason) => McpStatus::Failed {
                reason: reason.clone(),
            },
        }
    }
}

// ---------------------------------------------------------------------
// Tool input parameter shapes
// ---------------------------------------------------------------------
//
// Each tool that takes parameters uses a dedicated struct here so the
// generated JSON schema names the fields explicitly. Single-field tools
// still go through a struct rather than a bare `String` so the wire
// shape is uniformly `{ "field_name": "..." }` instead of positional.

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct StudyUidParams {
    /// DICOM Study Instance UID (`0020,000D`).
    pub study_instance_uid: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SeriesUidParams {
    /// DICOM Series Instance UID (`0020,000E`).
    pub series_instance_uid: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct PeerIdParams {
    /// NightOwl-assigned peer UUID (from `list_peers`).
    pub peer_id: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ScuFindParams {
    /// NightOwl-assigned peer UUID (from `list_peers`).
    pub peer_id: String,
    /// `patient` for Patient Root Q/R; `study` for Study Root Q/R.
    pub root: QrRoot,
    /// Query/Retrieve level. PS3.4 Annex C.
    pub level: FindLevel,
    /// Matching keys. Empty fields become Universal Matching (return key
    /// only). Wildcards `*` and `?` allowed in patient_id / patient_name.
    #[serde(default)]
    pub keys: ScuQueryKeys,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ScuMoveParams {
    /// NightOwl-assigned peer UUID (from `list_peers`).
    pub peer_id: String,
    pub root: QrRoot,
    pub level: FindLevel,
    #[serde(default)]
    pub keys: ScuQueryKeys,
    /// AE Title of the Move Destination. PS3.7 §9.1.4. Typically the
    /// requester itself; the responder must already know the destination
    /// by AE Title.
    pub destination_ae: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ScuStoreParams {
    /// NightOwl-assigned peer UUID (from `list_peers`).
    pub peer_id: String,
    /// Absolute filesystem paths to the DICOM Part-10 files to send.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ListActivityParams {
    /// Filter applied to the activity log. Every field is optional.
    pub filter: Option<ActivityFilter>,
}

// ---------------------------------------------------------------------
// Redacted config view returned by the `get_config` tool
// ---------------------------------------------------------------------

/// Public view of `AppConfig` returned through the MCP `get_config`
/// tool. We omit the nested `mcp` block because it is recursive metadata
/// (the LLM is already talking to the MCP server it would describe).
#[derive(Debug, Clone, Serialize)]
pub struct PublicConfig {
    pub local_ae_title: String,
    pub listen_port: u16,
    pub store_dir: PathBuf,
}

impl From<&AppConfig> for PublicConfig {
    fn from(cfg: &AppConfig) -> Self {
        Self {
            local_ae_title: cfg.local_ae_title.clone(),
            listen_port: cfg.listen_port,
            store_dir: cfg.store_dir.clone(),
        }
    }
}

// ---------------------------------------------------------------------
// The MCP handler
// ---------------------------------------------------------------------

/// Holds Arc-shared references to the same stores the Tauri commands
/// use, plus a snapshot of `AppConfig` taken at server-start time. The
/// snapshot is intentional: hot-reload of MCP settings is out of scope
/// for v1, so the LLM sees the boot-time AE Title for the lifetime of
/// the server.
///
/// `#[derive(Clone)]` is required because rmcp clones the handler per
/// session. All fields are cheap to clone: `Arc<T>` (one atomic
/// increment) and `AppHandle` (Tauri's own internal Arc).
#[derive(Clone)]
pub struct NightowlMcp {
    /// Tauri app handle, used by the SCU tools to emit DICOM activity
    /// events through `core::dimse::emit`. `Option` so the
    /// `core::mcp::tests::end_to_end_initialize` integration test can
    /// construct a handler without a Tauri runtime; production always
    /// passes `Some`. SCU tool calls return an internal error if it is
    /// `None`, which never happens in production.
    app: Option<AppHandle>,
    config: AppConfig,
    index: Arc<Index>,
    peers: Arc<PeerStore>,
    worklist: Arc<WorklistStore>,
    activity: Arc<ActivityLog>,
    /// Populated by the `#[tool_router]` macro and consumed by the
    /// `#[tool_handler]` impl below. The dead-code analyser does not
    /// see the macro-generated use site.
    #[allow(dead_code)]
    tool_router: ToolRouter<NightowlMcp>,
}

#[tool_router]
impl NightowlMcp {
    pub fn new(
        app: Option<AppHandle>,
        config: AppConfig,
        index: Arc<Index>,
        peers: Arc<PeerStore>,
        worklist: Arc<WorklistStore>,
        activity: Arc<ActivityLog>,
    ) -> Self {
        Self {
            app,
            config,
            index,
            peers,
            worklist,
            activity,
            tool_router: Self::tool_router(),
        }
    }

    /// Returns a cloned `AppHandle` for SCU tools that need to emit
    /// activity events. Returns an internal error in the (test-only)
    /// case where the handler was constructed without a handle.
    fn require_app(&self) -> Result<AppHandle, McpError> {
        self.app.clone().ok_or_else(|| {
            McpError::internal_error(
                "SCU tools require a Tauri app handle; this server was \
                 constructed without one (test mode)."
                    .to_string(),
                None,
            )
        })
    }

    // ----- Read tools (10) -----

    #[tool(
        description = "Return NightOwl's effective configuration: local AE Title, DICOM listen port, store directory. Does NOT include the MCP server's own settings."
    )]
    fn get_config(&self) -> Result<CallToolResult, McpError> {
        ok_json(&PublicConfig::from(&self.config))
    }

    #[tool(
        description = "List every configured remote DICOM peer (Application Entity) with id, name, AE Title, host and port."
    )]
    fn list_peers(&self) -> Result<CallToolResult, McpError> {
        let peers = self.peers.list().map_err(to_mcp_err)?;
        ok_json(&peers)
    }

    #[tool(
        description = "List every DICOM study present in the local SOP Instance index. Returns patient demographics, study description, study date, modalities present, series count and instance count per study."
    )]
    fn list_studies(&self) -> Result<CallToolResult, McpError> {
        let studies = self.index.list_studies().map_err(to_mcp_err)?;
        ok_json(&studies)
    }

    #[tool(
        description = "List every series under the given Study Instance UID. Returns series UID, description, modality and instance count per series."
    )]
    fn list_series_for_study(
        &self,
        Parameters(params): Parameters<StudyUidParams>,
    ) -> Result<CallToolResult, McpError> {
        let rows = self
            .index
            .list_series_for_study(&params.study_instance_uid)
            .map_err(to_mcp_err)?;
        ok_json(&rows)
    }

    #[tool(
        description = "List every SOP Instance under the given Series Instance UID. Returns SOP Instance UID, SOP Class UID, transfer syntax, on-disk path and size for each instance."
    )]
    fn list_instances_for_series(
        &self,
        Parameters(params): Parameters<SeriesUidParams>,
    ) -> Result<CallToolResult, McpError> {
        let rows = self
            .index
            .list_instances_for_series(&params.series_instance_uid)
            .map_err(to_mcp_err)?;
        ok_json(&rows)
    }

    #[tool(description = "Return the total SOP Instance count in the local store index.")]
    fn count_instances(&self) -> Result<CallToolResult, McpError> {
        let n = self.index.total_instance_count().map_err(to_mcp_err)?;
        ok_json(&n)
    }

    #[tool(
        description = "Rescan the configured store directory: walk every file, ingest DICOM Part-10 files into the index, skip non-DICOM. Returns a ScanReport summarising the scan."
    )]
    async fn rescan_store(&self) -> Result<CallToolResult, McpError> {
        let idx = self.index.clone();
        let dir = self.config.store_dir.clone();
        let report = tauri::async_runtime::spawn_blocking(move || idx.rescan_dir(&dir))
            .await
            .map_err(|e| McpError::internal_error(format!("rescan join: {e}"), None))?
            .map_err(to_mcp_err)?;
        ok_json(&report)
    }

    #[tool(description = "List every Modality Worklist (DMWL) scheduled procedure step entry.")]
    fn list_worklist(&self) -> Result<CallToolResult, McpError> {
        let rows = self.worklist.list().map_err(to_mcp_err)?;
        ok_json(&rows)
    }

    #[tool(
        description = "Return the total number of persisted activity log entries (every DIMSE association event NightOwl has seen, capped at 50,000)."
    )]
    fn count_activity(&self) -> Result<CallToolResult, McpError> {
        let n = self.activity.count().map_err(to_mcp_err)?;
        ok_json(&n)
    }

    #[tool(
        description = "Return a paginated, filtered page of activity log entries. The filter supports direction, status, peer AE Title, command, association id, free-text search, since-ms cutoff, limit (max 5000) and offset."
    )]
    fn list_activity(
        &self,
        Parameters(params): Parameters<ListActivityParams>,
    ) -> Result<CallToolResult, McpError> {
        let page = self
            .activity
            .list(params.filter.unwrap_or_default())
            .map_err(to_mcp_err)?;
        ok_json(&page)
    }

    // ----- Active SCU tools (4) -----

    #[tool(
        description = "Send a DICOM C-ECHO (Verification SOP Class — the DICOM ping) to the given peer. Returns success, status code and elapsed milliseconds."
    )]
    async fn scu_echo(
        &self,
        Parameters(params): Parameters<PeerIdParams>,
    ) -> Result<CallToolResult, McpError> {
        let peer = resolve_peer(&self.peers, &params.peer_id).map_err(to_mcp_err)?;
        let local_ae = self.config.local_ae_title.clone();
        let app = self.require_app()?;
        let result = tauri::async_runtime::spawn_blocking(move || {
            let emitter = dimse::TauriEmitter::new(app);
            dimse::scu_echo(&emitter, &local_ae, &peer)
        })
        .await
        .map_err(|e| McpError::internal_error(format!("scu_echo join: {e}"), None))?
        .map_err(to_mcp_err)?;
        ok_json(&result)
    }

    #[tool(
        description = "Send a DICOM C-FIND query (Patient Root or Study Root, level PATIENT/STUDY/SERIES/IMAGE) to the given peer. Returns matching identifiers."
    )]
    async fn scu_find(
        &self,
        Parameters(params): Parameters<ScuFindParams>,
    ) -> Result<CallToolResult, McpError> {
        let peer = resolve_peer(&self.peers, &params.peer_id).map_err(to_mcp_err)?;
        let local_ae = self.config.local_ae_title.clone();
        let app = self.require_app()?;
        let ScuFindParams {
            root, level, keys, ..
        } = params;
        let result = tauri::async_runtime::spawn_blocking(move || {
            let emitter = dimse::TauriEmitter::new(app);
            dimse::scu_find(&emitter, &local_ae, &peer, root, level, keys)
        })
        .await
        .map_err(|e| McpError::internal_error(format!("scu_find join: {e}"), None))?
        .map_err(to_mcp_err)?;
        ok_json(&result)
    }

    #[tool(
        description = "Send a DICOM C-MOVE request to the given peer, asking it to transfer matching SOP Instances to the named destination AE. Returns completed / failed sub-operation counts and the final status."
    )]
    async fn scu_move(
        &self,
        Parameters(params): Parameters<ScuMoveParams>,
    ) -> Result<CallToolResult, McpError> {
        let peer = resolve_peer(&self.peers, &params.peer_id).map_err(to_mcp_err)?;
        let local_ae = self.config.local_ae_title.clone();
        let app = self.require_app()?;
        let ScuMoveParams {
            root,
            level,
            keys,
            destination_ae,
            ..
        } = params;
        let result = tauri::async_runtime::spawn_blocking(move || {
            let emitter = dimse::TauriEmitter::new(app);
            dimse::scu_move(&emitter, &local_ae, &peer, root, level, keys, &destination_ae)
        })
        .await
        .map_err(|e| McpError::internal_error(format!("scu_move join: {e}"), None))?
        .map_err(to_mcp_err)?;
        ok_json(&result)
    }

    #[tool(
        description = "Send DICOM C-STORE for each given file to the given peer. Returns per-file outcome (success / failure / extracted SOP Instance UID / message)."
    )]
    async fn scu_store(
        &self,
        Parameters(params): Parameters<ScuStoreParams>,
    ) -> Result<CallToolResult, McpError> {
        let peer = resolve_peer(&self.peers, &params.peer_id).map_err(to_mcp_err)?;
        let local_ae = self.config.local_ae_title.clone();
        let app = self.require_app()?;
        let paths: Vec<PathBuf> = params.files.into_iter().map(PathBuf::from).collect();
        let outcomes = tauri::async_runtime::spawn_blocking(move || {
            let emitter = dimse::TauriEmitter::new(app);
            dimse::scu_store(&emitter, &local_ae, &peer, &paths)
        })
        .await
        .map_err(|e| McpError::internal_error(format!("scu_store join: {e}"), None))?
        .map_err(to_mcp_err)?;
        ok_json(&outcomes)
    }
}

#[tool_handler]
impl ServerHandler for NightowlMcp {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` is `#[non_exhaustive]` upstream, so we start from
        // `Default::default()` and overwrite only the fields we care
        // about. Future rmcp versions may add fields without breaking
        // this construction.
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::from_build_env();
        info.instructions = Some(
            "NightOwl MCP server. Use the read tools to inspect peers, studies, \
             series, instances, the modality worklist and the activity log. Use \
             the scu_* tools to actively send C-ECHO / C-FIND / C-MOVE / C-STORE \
             to a configured peer (peer_id values come from `list_peers`). \
             NightOwl is a developer tool — do not use it against production PACS \
             without explicit operator approval."
                .to_string(),
        );
        info
    }
}

// ---------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------

/// Binds the MCP HTTP service on `127.0.0.1:<config.mcp.port>` and spawns
/// the serve loop on the current tokio runtime.
///
/// Returns immediately with a `ServerHandle` carrying the actual bound
/// address (useful when `port == 0`, even though we currently always
/// require a non-zero port). Bind failure is returned as a Tauri error
/// rather than panicking — the caller in `lib.rs::setup` logs the error
/// and continues, treating MCP as ancillary.
pub async fn start_server(
    app: Option<AppHandle>,
    config: AppConfig,
    index: Arc<Index>,
    peers: Arc<PeerStore>,
    worklist: Arc<WorklistStore>,
    activity: Arc<ActivityLog>,
) -> Result<ServerHandle, AppError> {
    let addr: SocketAddr = format!("127.0.0.1:{}", config.mcp.port)
        .parse()
        .map_err(|e| AppError::Internal(format!("mcp bind address parse: {e}")))?;

    let prototype = NightowlMcp::new(app, config, index, peers, worklist, activity);

    let service = StreamableHttpService::new(
        move || Ok(prototype.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| AppError::Internal(format!("mcp bind {addr} failed: {e}")))?;
    let bind_addr = listener.local_addr().unwrap_or(addr);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let serve_task = tokio::spawn(async move {
        let outcome = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                // Treat sender-dropped as "exit anyway" — equivalent to
                // an explicit shutdown signal. This avoids the serve
                // loop outliving the ServerHandle if the handle gets
                // dropped without an explicit shutdown call.
                let _ = shutdown_rx.await;
            })
            .await;
        if let Err(err) = outcome {
            tracing::error!(error = %err, "mcp serve loop exited with error");
        }
    });

    tracing::info!(%bind_addr, "MCP server listening on http://{bind_addr}/mcp");

    Ok(ServerHandle {
        bind_addr,
        shutdown: std::sync::Mutex::new(Some(shutdown_tx)),
        serve_task: std::sync::Mutex::new(Some(serve_task)),
    })
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// Looks up a peer by NightOwl id, returning a Validation error if the
/// id is unknown. Mirrors `resolve_peer` in `lib.rs` — duplicated rather
/// than imported because that helper lives in the Tauri command layer
/// and depends on `tauri::State`.
fn resolve_peer(peers: &PeerStore, id: &str) -> Result<super::peers::Peer, AppError> {
    peers
        .list()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::validation("peer_id", format!("unknown peer id {id}")))
}

/// Renders a serialisable value as a pretty-printed JSON string wrapped
/// in a single `Content::text(...)` entry. Pretty-printed because the
/// primary consumer is an LLM and the slightly larger payload is offset
/// by improved readability inside the model's context.
fn ok_json<T: Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let body = serde_json::to_string_pretty(value).map_err(|e| {
        McpError::internal_error(format!("serialise tool result: {e}"), None)
    })?;
    Ok(CallToolResult::success(vec![Content::text(body)]))
}

/// Converts our internal `AppError` to the MCP `ErrorData` the SDK
/// expects. Validation errors map to `invalid_params` (the LLM is being
/// asked to fix its input); everything else maps to `internal_error`.
fn to_mcp_err(err: AppError) -> McpError {
    match err {
        AppError::Validation(v) => {
            McpError::invalid_params(format!("{}: {}", v.field, v.reason), None)
        }
        other => McpError::internal_error(other.to_string(), None),
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------
//
// The tests exercise the per-tool dispatch shape without booting Tauri
// or the HTTP transport: each test opens a temp store, builds a thin
// NightowlMcp without a real AppHandle, and calls the tool method
// directly. Tests for SCU tools are NOT included here — they require a
// live peer and an AppHandle and are covered by the existing dimse
// integration tests.

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // Smoke-test the helpers; the full tool path is exercised by the
    // integration verification described in PLAN-NEXT.md M24.

    #[test]
    fn to_mcp_err_maps_validation_to_invalid_params() {
        let err = AppError::validation("peer_id", "unknown");
        let mcp = to_mcp_err(err);
        // The exact code is rmcp-internal; we just check the message
        // contains the field name so an LLM can see what was wrong.
        assert!(format!("{mcp:?}").contains("peer_id"));
    }

    #[test]
    fn to_mcp_err_maps_other_to_internal_error() {
        let err = AppError::Internal("boom".to_string());
        let mcp = to_mcp_err(err);
        assert!(format!("{mcp:?}").contains("boom"));
    }

    #[test]
    fn ok_json_round_trips_through_call_tool_result() {
        #[derive(Serialize)]
        struct Sample {
            name: String,
            count: i32,
        }
        let result = ok_json(&Sample {
            name: "hi".into(),
            count: 7,
        })
        .expect("ok");

        // Extract the JSON body by serialising the result and parsing
        // the JSON-RPC envelope. This is more robust than depending on
        // CallToolResult's Debug representation, which is internal to
        // rmcp and may change between versions.
        let envelope = serde_json::to_value(&result).expect("serialise");
        let text = envelope
            .pointer("/content/0/text")
            .and_then(|v| v.as_str())
            .expect("text content present");

        let parsed: serde_json::Value =
            serde_json::from_str(text).expect("body is valid JSON");
        assert_eq!(parsed["name"], "hi");
        assert_eq!(parsed["count"], 7);
        // Pretty-printed: contains a newline between fields. A regression
        // to `to_string` (no formatting) would collapse this.
        assert!(text.contains('\n'));
    }

    #[test]
    fn public_config_drops_mcp_block() {
        let cfg = AppConfig::default_with_home(Path::new("/tmp"));
        let public = PublicConfig::from(&cfg);
        let json = serde_json::to_string(&public).unwrap();
        assert!(!json.contains("mcp"));
        assert!(json.contains("local_ae_title"));
    }

    /// Every tool advertised in the M24 plan must register through the
    /// `#[tool]` macro expansion. This test catches accidental drops
    /// (e.g. a macro typo that silently omits a method from the router)
    /// without needing to boot the HTTP transport or mock an AppHandle.
    /// The static `Self::tool_router()` constructor generated by
    /// `#[tool_router]` returns a `ToolRouter<Self>` we can introspect.
    #[test]
    fn tool_router_registers_every_documented_tool() {
        let router = NightowlMcp::tool_router();
        let expected = [
            "get_config",
            "list_peers",
            "list_studies",
            "list_series_for_study",
            "list_instances_for_series",
            "count_instances",
            "rescan_store",
            "list_worklist",
            "list_activity",
            "count_activity",
            "scu_echo",
            "scu_find",
            "scu_move",
            "scu_store",
        ];
        for name in expected {
            assert!(
                router.has_route(name),
                "MCP tool `{name}` is missing from the router — \
                 the #[tool] macro likely failed to register it.",
            );
        }
        // Reverse direction: catch tools that exist but are not
        // documented. A new tool added without updating this list and
        // the PLAN-NEXT.md M24 entry is a documentation drift bug.
        let registered: Vec<String> = router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            registered.len(),
            expected.len(),
            "tool count drifted: expected {} ({:?}), router has {} ({:?})",
            expected.len(),
            expected,
            registered.len(),
            registered,
        );
    }

    /// End-to-end smoke test: boot the real MCP server on a random
    /// loopback port, POST a JSON-RPC `initialize` request via HTTP,
    /// and assert the response contains `serverInfo`. Verifies the
    /// entire rmcp + axum + JSON-RPC wiring without going through
    /// Claude Code or any other external client.
    ///
    /// The handler is constructed with `None` for the Tauri app handle
    /// (production always passes `Some`). All read tools work without
    /// it; the four SCU tools would return an internal error if called,
    /// which is fine — we only exercise the protocol envelope here.
    #[tokio::test]
    async fn end_to_end_initialize_returns_server_info() {
        let tmp = std::env::temp_dir().join(format!(
            "nightowl-mcp-e2e-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        let index = std::sync::Arc::new(
            crate::core::store::Index::open(&tmp.join("store.sqlite")).expect("open index"),
        );
        let peers = std::sync::Arc::new(
            crate::core::peers::PeerStore::open(&tmp.join("peers.json")).expect("open peers"),
        );
        let worklist = std::sync::Arc::new(
            crate::core::worklist::WorklistStore::open(&tmp.join("worklist.sqlite"))
                .expect("open worklist"),
        );
        let activity = std::sync::Arc::new(
            crate::core::activity::ActivityLog::open(&tmp.join("activity.sqlite"))
                .expect("open activity"),
        );

        // Port 0 → OS picks a free ephemeral port. The actual bound
        // address comes back via `handle.bind_addr`.
        let mut cfg = AppConfig::default_with_home(&tmp);
        cfg.mcp = crate::core::config::McpConfig {
            enabled: true,
            port: 0,
        };

        let handle = start_server(None, cfg, index, peers, worklist, activity)
            .await
            .expect("start_server");
        let bind = handle.bind_addr;

        // JSON-RPC initialize. The protocol version is the original MCP
        // spec version — broadly supported by rmcp 1.7. The
        // Mcp-Protocol-Version header is required by rmcp's streamable
        // HTTP transport for initialize requests.
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "nightowl-test", "version": "0.0.0" }
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://{bind}/mcp"))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .json(&body)
            .send()
            .await
            .expect("HTTP POST initialize");

        assert!(
            response.status().is_success(),
            "initialize HTTP status: {}",
            response.status(),
        );
        let body = response.text().await.expect("read body");
        // The server may respond with plain JSON or with an SSE stream
        // (`data: {...}\n\n`). Either way, a successful initialise
        // result contains `serverInfo` — a substring match is enough
        // to confirm the wiring without coupling to the exact wire
        // format.
        assert!(
            body.contains("serverInfo"),
            "initialize response missing serverInfo. Body was:\n{body}",
        );

        handle.shutdown().await;

        // Best-effort cleanup; failure is harmless (temp).
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Sanity-check the schemars-derived input schema for a tool that
    /// takes a non-trivial structured input. If `JsonSchema` ever stops
    /// being derived on `ScuQueryKeys` or its surrounding wrapper, the
    /// rmcp macro will fall back to an `any` schema and clients lose
    /// per-field hinting. We assert the schema mentions the field names
    /// rather than depending on the exact JSON Schema spelling, which
    /// can vary between schemars releases.
    #[test]
    fn scu_find_tool_input_schema_includes_query_fields() {
        let router = NightowlMcp::tool_router();
        let tool = router
            .get("scu_find")
            .expect("scu_find tool registered");
        let schema = serde_json::to_string(&tool.input_schema)
            .expect("serialise input schema");
        for needle in [
            "peer_id",
            "root",
            "level",
            "keys",
            "patient_id",
            "study_instance_uid",
        ] {
            assert!(
                schema.contains(needle),
                "scu_find input schema is missing field `{needle}`: {schema}",
            );
        }
    }
}
