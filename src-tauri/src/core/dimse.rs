//! DIMSE — DICOM Message Service Element.
//!
//! This module owns the SCP listener: it binds a TCP socket on the
//! configured port, negotiates each inbound association via
//! `dicom-ul`, and dispatches the DIMSE commands carried in P-Data
//! PDUs. At M3 the only DIMSE service we implement is C-ECHO
//! (Verification SOP Class) — the DICOM "ping". Later milestones add
//! C-FIND (M4), C-STORE (M5), and C-MOVE / C-GET (M6).
//!
//! Vocabulary:
//! - **Association**: a negotiated TCP connection between two
//!   Application Entities. Both sides agree on which SOP Classes and
//!   Transfer Syntaxes they support before any DIMSE message flows.
//! - **PDU** (Protocol Data Unit): the wire-level message. An
//!   association is a sequence of PDUs.
//! - **Command Set**: the small DICOM data set inside a P-Data PDU
//!   that carries the DIMSE command (which operation, message id,
//!   status, …). Always encoded in Implicit VR Little Endian.
//! - **Verification SOP Class** (UID `1.2.840.10008.1.1`): the
//!   abstract syntax for C-ECHO.
//!
//! All inbound and outbound DIMSE messages emit `activity` events
//! that the M9 Activity page will subscribe to. The payload is a
//! stable `ActivityEvent` JSON shape so the frontend code can land
//! before the persistent activity log does.

use std::collections::HashMap;
use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use dicom_core::{dicom_value, DataElement, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_dictionary_std::uids::{
    COMPUTED_RADIOGRAPHY_IMAGE_STORAGE, CT_IMAGE_STORAGE,
    DIGITAL_X_RAY_IMAGE_STORAGE_FOR_PRESENTATION, ENCAPSULATED_PDF_STORAGE,
    EXPLICIT_VR_LITTLE_ENDIAN, IMPLICIT_VR_LITTLE_ENDIAN, JPEG_BASELINE8_BIT,
    MR_IMAGE_STORAGE, PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
    PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_GET,
    PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE,
    SECONDARY_CAPTURE_IMAGE_STORAGE, STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
    STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_GET,
    STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE, ULTRASOUND_IMAGE_STORAGE,
    VERIFICATION,
};
use dicom_object::{open_file, FileMetaTableBuilder};
use dicom_ul::association::client::ClientAssociationOptions;
use dicom_ul::association::ClientAssociation;
use dicom_ul::pdu::PresentationContextResultReason;
use dicom_encoding::transfer_syntax::{TransferSyntax, TransferSyntaxIndex};
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::{ServerAssociation, ServerAssociationOptions};
use dicom_ul::association::Association;
use dicom_ul::pdu::{PDataValue, PDataValueType, Pdu};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use super::activity::{ActivityLog, PersistedActivityEvent};
use super::error::AppError;
use super::peers::{Peer, PeerStore};
use super::store::{FindLevel, FindQuery, FindRow, Index, KeyMatch, RetrieveInstance};

// DIMSE Command Field values (PS3.7 Table 7.1-1). Listed even when
// unused at M3 so the table is in one place for M4/M5/M6.
#[allow(dead_code)]
mod cmd {
    pub const C_STORE_RQ: u16 = 0x0001;
    pub const C_STORE_RSP: u16 = 0x8001;
    pub const C_FIND_RQ: u16 = 0x0020;
    pub const C_FIND_RSP: u16 = 0x8020;
    pub const C_GET_RQ: u16 = 0x0010;
    pub const C_GET_RSP: u16 = 0x8010;
    pub const C_MOVE_RQ: u16 = 0x0021;
    pub const C_MOVE_RSP: u16 = 0x8021;
    pub const C_ECHO_RQ: u16 = 0x0030;
    pub const C_ECHO_RSP: u16 = 0x8030;
}

// CommandDataSetType: "No Data Set Present" (PS3.7 Section 9.3.5).
const NO_DATASET: u16 = 0x0101;
// CommandDataSetType: "Data Set Present" (any value other than 0x0101).
const DATASET_PRESENT: u16 = 0x0000;

// Maximum PDU length we advertise on every association (both as SCP
// and as SCU). Has to be big enough to receive A-ASSOCIATE-RQ PDUs
// from peers that offer many SOP classes (DCMTK's getscu can easily
// produce 30 KB association requests). The negotiated working PDU
// size is the min of both sides' advertised values.
const MAX_PDU_LENGTH: u32 = 256 * 1024;

// DIMSE status codes (PS3.7 Annex C).
const STATUS_SUCCESS: u16 = 0x0000;
const STATUS_PENDING: u16 = 0xFF00;
const STATUS_REFUSED_OUT_OF_RESOURCES: u16 = 0xA700;
const STATUS_FAILED_IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS: u16 = 0xA900;
const STATUS_FAILED_UNABLE_TO_PROCESS: u16 = 0xC000;
// C-MOVE / C-GET sub-operation outcomes (PS3.7 §9.1.4 / §9.1.3).
const STATUS_MOVE_DESTINATION_UNKNOWN: u16 = 0xA801;
const STATUS_WARNING_SUBOPS_COMPLETE_WITH_FAILURES: u16 = 0xB000;

// ---------------------------------------------------------------------
// Activity events
// ---------------------------------------------------------------------

/// Payload of the `activity` Tauri event.
///
/// The event name is intentionally stable: M9 will consume it from the
/// activity page, and persisting these to SQLite is a future task.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityEvent {
    /// Wall-clock millisecond timestamp (UTC). The frontend converts
    /// to the local zone for display.
    pub timestamp_ms: i64,
    /// Direction of the message relative to Phantom: inbound is from
    /// the network into us; outbound is from us back out. `Info` is
    /// for non-message events (listener up, association open/close).
    pub direction: Direction,
    /// Peer Application Entity title, once it is known. None for the
    /// pre-negotiation phase.
    pub peer_ae_title: Option<String>,
    /// Peer network address, when available.
    pub peer_host: Option<String>,
    /// Short DIMSE command tag, e.g. "C-ECHO-RQ", "C-ECHO-RSP". None
    /// for lifecycle events.
    pub command: Option<String>,
    /// Categorical status used by the UI for colouring rows.
    pub status: Status,
    /// Human-readable summary.
    pub message: String,
    /// Stable identifier that groups every event from one association
    /// together. Generated when the association is accepted.
    pub association_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Inbound,
    Outbound,
    Info,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Info,
    Success,
    Warning,
    Error,
}

fn emit(app: &AppHandle, event: ActivityEvent) {
    tracing::info!(
        target: "phantom_lib::dimse",
        direction = ?event.direction,
        peer = ?event.peer_ae_title,
        command = ?event.command,
        message = %event.message,
        "activity",
    );

    // Persist when the activity log is in Tauri state. A negative id is
    // emitted only when persistence failed; the UI can render that as
    // "live but unpersisted" if it cares.
    let payload: PersistedActivityEvent = match app.try_state::<Arc<ActivityLog>>() {
        Some(log) => match log.record(event.clone()) {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = %err, "activity persist failed");
                PersistedActivityEvent { id: -1, event }
            }
        },
        None => PersistedActivityEvent { id: -1, event },
    };

    if let Err(err) = app.emit("activity", &payload) {
        tracing::warn!(error = %err, "activity emit failed");
    }
}

// ---------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------

/// Shared context the per-association threads need.
///
/// - `index` — SOP Instance index for query (M4) + ingestion (M5).
/// - `store_dir` — on-disk store directory where M5's C-STORE writes
///   inbound files.
/// - `peers` — configured remote DICOM nodes; M6's C-MOVE handler
///   resolves a Move Destination AE Title against this list.
/// - `local_ae_title` — Phantom's own AE Title, sent as the
///   `MoveOriginatorApplicationEntityTitle` in M6's outbound C-STORE
///   sub-operations so the destination knows which AE asked.
pub struct ScpContext {
    pub index: Arc<Index>,
    pub store_dir: PathBuf,
    pub peers: Arc<PeerStore>,
    pub local_ae_title: String,
}

/// Handle to the running SCP listener. Holding this struct alive is
/// what keeps the listener bound; dropping the inner `shutdown` flag
/// causes the accept loop to exit on its next iteration.
pub struct ListenerHandle {
    pub bind_addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
}

/// The full set of Storage SOP Classes Phantom accepts as a C-STORE
/// SCP. Adding a new modality here is the one-line change required to
/// accept its objects. Kept as a constant array so the negotiation
/// builder and any future "what do we support" query share the same
/// source of truth.
const STORAGE_SOP_CLASSES: &[&str] = &[
    CT_IMAGE_STORAGE,
    MR_IMAGE_STORAGE,
    SECONDARY_CAPTURE_IMAGE_STORAGE,
    ULTRASOUND_IMAGE_STORAGE,
    COMPUTED_RADIOGRAPHY_IMAGE_STORAGE,
    DIGITAL_X_RAY_IMAGE_STORAGE_FOR_PRESENTATION,
    ENCAPSULATED_PDF_STORAGE,
];

impl ListenerHandle {
    /// Asks the accept loop to stop. Currently best-effort: the loop
    /// only checks between `accept()` calls. Reserved for the M3+
    /// follow-up where Settings rebinds the listener on port change.
    #[allow(dead_code)]
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

/// Binds a TCP listener on `0.0.0.0:<port>` and spawns a thread that
/// accepts and services associations until shutdown.
///
/// The bind itself is synchronous so callers learn immediately whether
/// the port is in use. Accepted connections are handed to a per-
/// association thread; `dicom-ul`'s server API is synchronous, so a
/// thread-per-association is the simple correct choice for a dev tool
/// that does not expect more than a few concurrent peers.
pub fn start_listener(
    port: u16,
    local_ae_title: String,
    app: AppHandle,
    scp: Arc<ScpContext>,
) -> Result<ListenerHandle, AppError> {
    let bind = SocketAddr::from(([0, 0, 0, 0], port));
    let listener =
        TcpListener::bind(bind).map_err(|e| AppError::Io(format!("bind {bind}: {e}")))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| AppError::Io(e.to_string()))?;
    let bind_addr = listener
        .local_addr()
        .map_err(|e| AppError::Io(e.to_string()))?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_for_thread = shutdown.clone();
    let app_for_thread = app.clone();
    let ae_for_thread = local_ae_title.clone();
    let scp_for_thread = scp.clone();

    std::thread::Builder::new()
        .name("phantom-scp-accept".to_string())
        .spawn(move || {
            run_accept_loop(
                listener,
                ae_for_thread,
                app_for_thread,
                scp_for_thread,
                shutdown_for_thread,
            )
        })
        .map_err(|e| AppError::Internal(format!("spawn accept thread: {e}")))?;

    emit(
        &app,
        ActivityEvent {
            timestamp_ms: Utc::now().timestamp_millis(),
            direction: Direction::Info,
            peer_ae_title: None,
            peer_host: None,
            command: None,
            status: Status::Info,
            message: format!("SCP listening on {bind_addr} as AE {local_ae_title}"),
            association_id: "_listener".to_string(),
        },
    );

    Ok(ListenerHandle {
        bind_addr,
        shutdown,
    })
}

fn run_accept_loop(
    listener: TcpListener,
    local_ae_title: String,
    app: AppHandle,
    scp: Arc<ScpContext>,
    shutdown: Arc<AtomicBool>,
) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    for incoming in listener.incoming() {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let stream = match incoming {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "accept failed");
                continue;
            }
        };
        let peer = stream.peer_addr().ok();
        let local_seq = COUNTER.fetch_add(1, Ordering::SeqCst);
        let association_id = format!("a-{}-{}", local_seq, Uuid::new_v4().simple());

        let app_clone = app.clone();
        let ae_clone = local_ae_title.clone();
        let scp_clone = scp.clone();
        if let Err(err) = std::thread::Builder::new()
            .name(format!("phantom-scp-{}", local_seq))
            .spawn(move || {
                handle_association(
                    stream,
                    peer,
                    ae_clone,
                    app_clone,
                    scp_clone,
                    association_id,
                );
            })
        {
            tracing::error!(error = %err, "spawn association thread failed");
        }
    }
}

// ---------------------------------------------------------------------
// Association handler
// ---------------------------------------------------------------------

fn handle_association(
    stream: TcpStream,
    peer: Option<SocketAddr>,
    local_ae_title: String,
    app: AppHandle,
    scp: Arc<ScpContext>,
    association_id: String,
) {
    let peer_host = peer.map(|p| p.to_string());

    if let Err(err) = stream.set_read_timeout(Some(Duration::from_secs(30))) {
        tracing::warn!(error = %err, "set_read_timeout failed");
    }

    // Negotiate the abstract syntaxes (the DICOM services) we provide:
    // - Verification (M3 — C-ECHO)
    // - Patient Root + Study Root Q/R Find (M4 — C-FIND)
    // - The common Storage SOP Classes (M5 — C-STORE)
    // …and the transfer syntaxes (wire encodings) we accept:
    // - Implicit VR Little Endian (the universal baseline)
    // - Explicit VR Little Endian (preferred when both sides support it)
    // - JPEG Baseline 8-bit (covers most lossy-compressed imagery; we
    //   do not decode pixel data — we just write the bytes back out)
    //
    // Explicit VR Big Endian (1.2.840.10008.1.2.2) was on the plan but
    // is a retired DICOM transfer syntax; dicom_dictionary_std marks
    // the constant deprecated. Anything still offering it is on
    // retired equipment. Re-add via the raw UID string if needed.
    let mut options = ServerAssociationOptions::new()
        .accept_any()
        // Some SCUs (notably DCMTK getscu) offer dozens of Storage SOP
        // Classes in their A-ASSOCIATE-RQ for C-GET. The default
        // 16 KB receive buffer is too small for the resulting PDU,
        // which gets rejected as "incoming PDU was too large". Bump
        // to 256 KB to accept those — the actual data PDUs use the
        // negotiated maximum which both sides agree to.
        .max_pdu_length(MAX_PDU_LENGTH)
        .with_abstract_syntax(VERIFICATION)
        .with_abstract_syntax(PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND)
        .with_abstract_syntax(STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND)
        .with_abstract_syntax(PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE)
        .with_abstract_syntax(STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE)
        .with_abstract_syntax(PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_GET)
        .with_abstract_syntax(STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_GET)
        .with_transfer_syntax(IMPLICIT_VR_LITTLE_ENDIAN)
        .with_transfer_syntax(EXPLICIT_VR_LITTLE_ENDIAN)
        .with_transfer_syntax(JPEG_BASELINE8_BIT)
        .ae_title(local_ae_title.clone());
    for sop_class in STORAGE_SOP_CLASSES {
        options = options.with_abstract_syntax(*sop_class);
    }

    let mut association = match options.establish(stream) {
        Ok(a) => a,
        Err(err) => {
            emit(
                &app,
                ActivityEvent {
                    timestamp_ms: Utc::now().timestamp_millis(),
                    direction: Direction::Info,
                    peer_ae_title: None,
                    peer_host: peer_host.clone(),
                    command: None,
                    status: Status::Error,
                    message: format!("association rejected: {err}"),
                    association_id: association_id.clone(),
                },
            );
            return;
        }
    };

    let calling_ae = association.peer_ae_title().to_string();

    emit(
        &app,
        ActivityEvent {
            timestamp_ms: Utc::now().timestamp_millis(),
            direction: Direction::Info,
            peer_ae_title: Some(calling_ae.clone()),
            peer_host: peer_host.clone(),
            command: None,
            status: Status::Info,
            message: format!("association accepted from {calling_ae}"),
            association_id: association_id.clone(),
        },
    );

    let ctx = AssociationCtx {
        association_id: association_id.clone(),
        peer_ae_title: calling_ae.clone(),
        peer_host: peer_host.clone(),
        app: app.clone(),
    };

    // DIMSE messages may span multiple P-DATA PDVs: typically a Command
    // PDV followed by one or more Data PDVs carrying the identifier or
    // data set. We hold the Command set and the accumulated data bytes
    // here until the last Data PDV arrives, then dispatch.
    let mut in_flight: Option<InFlightCommand> = None;

    loop {
        let pdu = match association.receive() {
            Ok(p) => p,
            Err(err) => {
                ctx.emit_lifecycle(Status::Warning, format!("receive error: {err}"));
                break;
            }
        };

        match pdu {
            Pdu::PData { data } => {
                let mut continue_loop = true;
                for pdv in data {
                    match handle_pdv(&mut association, &mut in_flight, &ctx, &scp, pdv) {
                        Ok(true) => {}
                        Ok(false) => {
                            continue_loop = false;
                            break;
                        }
                        Err(err) => {
                            ctx.emit_lifecycle(
                                Status::Error,
                                format!("dispatch failed: {err}"),
                            );
                            let _ = association.inner_stream().shutdown(Shutdown::Both);
                            return;
                        }
                    }
                }
                if !continue_loop {
                    return;
                }
            }
            Pdu::ReleaseRQ => {
                ctx.emit_inbound("A-RELEASE-RQ", "release requested");
                if let Err(err) = association.send(&Pdu::ReleaseRP) {
                    ctx.emit_lifecycle(Status::Warning, format!("release reply failed: {err}"));
                } else {
                    ctx.emit_outbound("A-RELEASE-RP", "release acknowledged");
                }
                break;
            }
            Pdu::AbortRQ { source } => {
                ctx.emit_lifecycle(Status::Warning, format!("abort received from {:?}", source));
                break;
            }
            other => {
                ctx.emit_lifecycle(
                    Status::Warning,
                    format!("unexpected pdu: {}", other.short_description()),
                );
            }
        }
    }

    ctx.emit_lifecycle(Status::Info, "association closed".to_string());
}

/// Command awaiting its data set across multiple P-DATA PDVs.
struct InFlightCommand {
    command: InMemDicomObject,
    presentation_context_id: u8,
    data: Vec<u8>,
}

/// Processes one P-DATA Value. Routes Command PDVs through the
/// dispatcher (immediately if no data set is expected, otherwise
/// after collecting the trailing Data PDVs).
fn handle_pdv(
    association: &mut ServerAssociation<TcpStream>,
    in_flight: &mut Option<InFlightCommand>,
    ctx: &AssociationCtx,
    scp: &ScpContext,
    pdv: PDataValue,
) -> Result<bool, AppError> {
    match pdv.value_type {
        PDataValueType::Command => {
            let command = parse_command_set(&pdv.data)?;
            let data_expected =
                read_u16(&command, tags::COMMAND_DATA_SET_TYPE).unwrap_or(NO_DATASET)
                    != NO_DATASET;
            if data_expected {
                *in_flight = Some(InFlightCommand {
                    command,
                    presentation_context_id: pdv.presentation_context_id,
                    data: Vec::new(),
                });
                Ok(true)
            } else {
                dispatch(
                    association,
                    ctx,
                    scp,
                    &command,
                    &[],
                    pdv.presentation_context_id,
                )
            }
        }
        PDataValueType::Data => {
            let Some(in_f) = in_flight.as_mut() else {
                ctx.emit_lifecycle(
                    Status::Warning,
                    "data pdv received without a preceding command".to_string(),
                );
                return Ok(true);
            };
            in_f.data.extend_from_slice(&pdv.data);
            if pdv.is_last {
                let taken = in_flight.take().expect("checked above");
                dispatch(
                    association,
                    ctx,
                    scp,
                    &taken.command,
                    &taken.data,
                    taken.presentation_context_id,
                )
            } else {
                Ok(true)
            }
        }
    }
}

struct AssociationCtx {
    association_id: String,
    peer_ae_title: String,
    peer_host: Option<String>,
    app: AppHandle,
}

impl AssociationCtx {
    fn emit_inbound(&self, command: &str, message: impl Into<String>) {
        emit(
            &self.app,
            ActivityEvent {
                timestamp_ms: Utc::now().timestamp_millis(),
                direction: Direction::Inbound,
                peer_ae_title: Some(self.peer_ae_title.clone()),
                peer_host: self.peer_host.clone(),
                command: Some(command.to_string()),
                status: Status::Info,
                message: message.into(),
                association_id: self.association_id.clone(),
            },
        );
    }

    fn emit_outbound(&self, command: &str, message: impl Into<String>) {
        emit(
            &self.app,
            ActivityEvent {
                timestamp_ms: Utc::now().timestamp_millis(),
                direction: Direction::Outbound,
                peer_ae_title: Some(self.peer_ae_title.clone()),
                peer_host: self.peer_host.clone(),
                command: Some(command.to_string()),
                status: Status::Success,
                message: message.into(),
                association_id: self.association_id.clone(),
            },
        );
    }

    fn emit_lifecycle(&self, status: Status, message: String) {
        emit(
            &self.app,
            ActivityEvent {
                timestamp_ms: Utc::now().timestamp_millis(),
                direction: Direction::Info,
                peer_ae_title: Some(self.peer_ae_title.clone()),
                peer_host: self.peer_host.clone(),
                command: None,
                status,
                message,
                association_id: self.association_id.clone(),
            },
        );
    }
}

// ---------------------------------------------------------------------
// DIMSE command dispatch
// ---------------------------------------------------------------------

/// Routes a fully-assembled DIMSE message to the right handler.
///
/// Returns `Ok(true)` to continue the receive loop, `Ok(false)` to
/// stop after a clean abort. `Err` indicates a protocol-level failure
/// the caller should treat as fatal for this association.
fn dispatch(
    association: &mut ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    scp: &ScpContext,
    command: &InMemDicomObject,
    data: &[u8],
    pc_id: u8,
) -> Result<bool, AppError> {
    let command_field = read_u16(command, tags::COMMAND_FIELD)?;
    match command_field {
        cmd::C_ECHO_RQ => handle_c_echo(association, ctx, command, pc_id),
        cmd::C_FIND_RQ => handle_c_find(association, ctx, &scp.index, command, data, pc_id),
        cmd::C_STORE_RQ => handle_c_store(association, ctx, scp, command, data, pc_id),
        cmd::C_MOVE_RQ => handle_c_move(association, ctx, scp, command, data, pc_id),
        cmd::C_GET_RQ => handle_c_get(association, ctx, scp, command, data, pc_id),
        cmd::C_STORE_RSP => {
            // C-GET sub-operations get acknowledged by the requester
            // with a C-STORE-RSP back to us. We don't act on them
            // (the requester counts what it received; we already
            // counted what we sent), so swallow silently rather than
            // warning.
            Ok(true)
        }
        other => {
            // High bit set (0x8xxx) marks a response. If we see a
            // response we didn't expect we still don't act on it but
            // we don't need to warn loudly either.
            if other & 0x8000 != 0 {
                tracing::debug!(command_field = other, "received unsolicited DIMSE response");
            } else {
                ctx.emit_lifecycle(
                    Status::Warning,
                    format!("unsupported DIMSE command 0x{:04X}", other),
                );
            }
            Ok(true)
        }
    }
}

fn parse_command_set(bytes: &[u8]) -> Result<InMemDicomObject, AppError> {
    let ts = TransferSyntaxRegistry
        .get(IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| AppError::Internal("Implicit VR LE not in registry".to_string()))?;
    InMemDicomObject::read_dataset_with_ts(bytes, ts)
        .map_err(|e| AppError::Internal(format!("command set decode: {e}")))
}

fn handle_c_echo(
    association: &mut ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    command: &InMemDicomObject,
    presentation_context_id: u8,
) -> Result<bool, AppError> {
    let message_id = read_u16(command, tags::MESSAGE_ID)?;

    ctx.emit_inbound("C-ECHO-RQ", format!("message id {message_id}"));

    let response = build_c_echo_rsp(message_id);
    let response_bytes = encode_command_set(&response)?;

    let response_pdu = Pdu::PData {
        data: vec![PDataValue {
            presentation_context_id,
            value_type: PDataValueType::Command,
            is_last: true,
            data: response_bytes,
        }],
    };

    association
        .send(&response_pdu)
        .map_err(|e| AppError::Internal(format!("send C-ECHO-RSP: {e}")))?;

    ctx.emit_outbound(
        "C-ECHO-RSP",
        format!("message id {message_id} status 0x0000 (Success)"),
    );

    Ok(true)
}

// ---------------------------------------------------------------------
// C-FIND handler (M4)
// ---------------------------------------------------------------------

fn handle_c_find(
    association: &mut ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    index: &Index,
    command: &InMemDicomObject,
    identifier_bytes: &[u8],
    pc_id: u8,
) -> Result<bool, AppError> {
    let message_id = read_u16(command, tags::MESSAGE_ID)?;
    let sop_class_uid = read_str(command, tags::AFFECTED_SOP_CLASS_UID)?;

    // The identifier dataset uses the negotiated transfer syntax for
    // this presentation context — not Implicit VR LE like the command
    // set always is. We pull the UID as an owned String and resolve to
    // a `&TransferSyntax` on each use; that keeps the immutable borrow
    // on the association brief so we can call `send` later.
    let ts_uid = transfer_syntax_uid_for(association, pc_id)?;
    let ts = lookup_ts(&ts_uid)?;

    let request_identifier = InMemDicomObject::read_dataset_with_ts(identifier_bytes, ts)
        .map_err(|e| AppError::Internal(format!("C-FIND identifier decode: {e}")))?;

    let level_str = read_str(&request_identifier, tags::QUERY_RETRIEVE_LEVEL)?;
    let level = match level_str.as_str() {
        "PATIENT" => FindLevel::Patient,
        "STUDY" => FindLevel::Study,
        "SERIES" => FindLevel::Series,
        "IMAGE" => FindLevel::Image,
        other => {
            ctx.emit_inbound(
                "C-FIND-RQ",
                format!("message id {message_id} unknown level {other}"),
            );
            send_c_find_final(
                association,
                ctx,
                &sop_class_uid,
                message_id,
                pc_id,
                STATUS_FAILED_IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            )?;
            return Ok(true);
        }
    };

    ctx.emit_inbound(
        "C-FIND-RQ",
        format!("message id {message_id} level {level_str}"),
    );

    let query = build_find_query(&request_identifier, level);
    let matches = index.find(&query)?;
    let match_count = matches.len();

    for row in matches {
        let response_identifier = build_response_identifier(&request_identifier, &row, level);
        let rsp_command = build_c_find_rsp(message_id, &sop_class_uid, STATUS_PENDING, true);
        let cmd_bytes = encode_command_set(&rsp_command)?;
        let id_bytes = encode_identifier(&response_identifier, lookup_ts(&ts_uid)?)?;

        let pdu = Pdu::PData {
            data: vec![
                PDataValue {
                    presentation_context_id: pc_id,
                    value_type: PDataValueType::Command,
                    is_last: true,
                    data: cmd_bytes,
                },
                PDataValue {
                    presentation_context_id: pc_id,
                    value_type: PDataValueType::Data,
                    is_last: true,
                    data: id_bytes,
                },
            ],
        };
        association
            .send(&pdu)
            .map_err(|e| AppError::Internal(format!("send C-FIND-RSP Pending: {e}")))?;
    }

    send_c_find_final(
        association,
        ctx,
        &sop_class_uid,
        message_id,
        pc_id,
        STATUS_SUCCESS,
    )?;

    ctx.emit_outbound(
        "C-FIND-RSP",
        format!(
            "{match_count} match{} + final Success",
            if match_count == 1 { "" } else { "es" }
        ),
    );

    Ok(true)
}

fn send_c_find_final(
    association: &mut ServerAssociation<TcpStream>,
    _ctx: &AssociationCtx,
    sop_class_uid: &str,
    message_id: u16,
    pc_id: u8,
    status: u16,
) -> Result<(), AppError> {
    let rsp = build_c_find_rsp(message_id, sop_class_uid, status, false);
    let bytes = encode_command_set(&rsp)?;
    association
        .send(&Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: pc_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: bytes,
            }],
        })
        .map_err(|e| AppError::Internal(format!("send final C-FIND-RSP: {e}")))
}

fn build_c_find_rsp(
    message_id: u16,
    sop_class_uid: &str,
    status: u16,
    has_dataset: bool,
) -> InMemDicomObject {
    let dataset_type = if has_dataset {
        DATASET_PRESENT
    } else {
        NO_DATASET
    };
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            sop_class_uid.to_string(),
        ),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_FIND_RSP]),
        ),
        DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            dicom_value!(U16, [message_id]),
        ),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [dataset_type]),
        ),
        DataElement::new(tags::STATUS, VR::US, dicom_value!(U16, [status])),
    ])
}

/// Translates the inbound C-FIND-RQ identifier into a `FindQuery`.
///
/// `*` and `?` are wildcard matches. Backslash-separated UID values
/// become a `List` match. `YYYYMMDD-YYYYMMDD` on `StudyDate` becomes a
/// `Range` match. Empty values mean Universal Matching — `None` in the
/// `FindQuery` — so the response identifier will still carry that key
/// populated from each matched row.
fn build_find_query(identifier: &InMemDicomObject, level: FindLevel) -> FindQuery {
    let mut q = FindQuery::new(level);
    q.patient_id = key_match_text(identifier, tags::PATIENT_ID);
    q.patient_name = key_match_text(identifier, tags::PATIENT_NAME);
    q.study_instance_uid = key_match_uid(identifier, tags::STUDY_INSTANCE_UID);
    q.study_date = key_match_date(identifier, tags::STUDY_DATE);
    q.modality = key_match_text(identifier, tags::MODALITY);
    q.series_instance_uid = key_match_uid(identifier, tags::SERIES_INSTANCE_UID);
    q.sop_instance_uid = key_match_uid(identifier, tags::SOP_INSTANCE_UID);
    q.sop_class_uid = key_match_uid(identifier, tags::SOP_CLASS_UID);
    q
}

fn key_match_text(obj: &InMemDicomObject, tag: Tag) -> Option<KeyMatch> {
    let raw = obj.element(tag).ok()?.to_str().ok()?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    if raw.contains('*') || raw.contains('?') {
        Some(KeyMatch::Wildcard(raw))
    } else {
        Some(KeyMatch::Single(raw))
    }
}

fn key_match_uid(obj: &InMemDicomObject, tag: Tag) -> Option<KeyMatch> {
    let raw = obj.element(tag).ok()?.to_str().ok()?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    if raw.contains('\\') {
        let values: Vec<String> = raw
            .split('\\')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if values.len() == 1 {
            Some(KeyMatch::Single(values.into_iter().next().unwrap()))
        } else {
            Some(KeyMatch::List(values))
        }
    } else {
        Some(KeyMatch::Single(raw))
    }
}

fn key_match_date(obj: &InMemDicomObject, tag: Tag) -> Option<KeyMatch> {
    let raw = obj.element(tag).ok()?.to_str().ok()?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    if let Some((start, end)) = raw.split_once('-') {
        if !start.is_empty() && !end.is_empty() {
            return Some(KeyMatch::Range(start.to_string(), end.to_string()));
        }
    }
    Some(KeyMatch::Single(raw))
}

/// Builds the response identifier for one matched row.
///
/// Every tag in the request gets a corresponding tag in the response.
/// QueryRetrieveLevel is copied (well, re-derived from `level`); every
/// other key is populated from the `FindRow` if we have data, or
/// returned with an empty value if we do not. This matches DICOM's
/// behaviour for Universal Matching keys we do not track.
fn build_response_identifier(
    request: &InMemDicomObject,
    row: &FindRow,
    level: FindLevel,
) -> InMemDicomObject {
    let mut rsp = InMemDicomObject::new_empty();

    let level_str = match level {
        FindLevel::Patient => "PATIENT",
        FindLevel::Study => "STUDY",
        FindLevel::Series => "SERIES",
        FindLevel::Image => "IMAGE",
    };
    rsp.put_element(DataElement::new(
        tags::QUERY_RETRIEVE_LEVEL,
        VR::CS,
        level_str.to_string(),
    ));

    for elem in request.iter() {
        let tag = elem.header().tag;
        if tag == tags::QUERY_RETRIEVE_LEVEL {
            continue;
        }
        let vr = elem.header().vr();
        let value = response_value_for(tag, row);
        rsp.put_element(DataElement::new(tag, vr, value));
    }

    rsp
}

fn response_value_for(tag: Tag, row: &FindRow) -> String {
    match tag {
        tags::PATIENT_ID => row.patient_id.clone(),
        tags::PATIENT_NAME => row.patient_name.clone().unwrap_or_default(),
        tags::STUDY_INSTANCE_UID => row.study_instance_uid.clone().unwrap_or_default(),
        tags::STUDY_DESCRIPTION => row.study_description.clone().unwrap_or_default(),
        tags::STUDY_DATE => row.study_date.clone().unwrap_or_default(),
        tags::MODALITY => row.modality.clone().unwrap_or_default(),
        tags::MODALITIES_IN_STUDY => row.modalities_in_study.clone().unwrap_or_default(),
        tags::SERIES_INSTANCE_UID => row.series_instance_uid.clone().unwrap_or_default(),
        tags::SERIES_DESCRIPTION => row.series_description.clone().unwrap_or_default(),
        tags::SOP_INSTANCE_UID => row.sop_instance_uid.clone().unwrap_or_default(),
        tags::SOP_CLASS_UID => row.sop_class_uid.clone().unwrap_or_default(),
        tags::NUMBER_OF_STUDY_RELATED_SERIES => row
            .number_of_study_related_series
            .map(|n| n.to_string())
            .unwrap_or_default(),
        tags::NUMBER_OF_STUDY_RELATED_INSTANCES => row
            .number_of_study_related_instances
            .map(|n| n.to_string())
            .unwrap_or_default(),
        tags::NUMBER_OF_SERIES_RELATED_INSTANCES => row
            .number_of_series_related_instances
            .map(|n| n.to_string())
            .unwrap_or_default(),
        // Any key we don't track gets an empty value, which is the
        // DICOM-correct answer for a return key with no source data.
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------
// C-STORE handler (M5)
// ---------------------------------------------------------------------

fn handle_c_store(
    association: &mut ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    scp: &ScpContext,
    command: &InMemDicomObject,
    data: &[u8],
    pc_id: u8,
) -> Result<bool, AppError> {
    let message_id = read_u16(command, tags::MESSAGE_ID)?;
    let sop_class_uid = read_str(command, tags::AFFECTED_SOP_CLASS_UID)?;
    let sop_instance_uid = read_str(command, tags::AFFECTED_SOP_INSTANCE_UID)?;

    ctx.emit_inbound(
        "C-STORE-RQ",
        format!("message id {message_id} sop {sop_instance_uid}"),
    );

    // Anything below this point is a peer-data failure rather than a
    // protocol failure: we want to send back a C-STORE-RSP with a
    // failure status so the SCU sees the problem, then move on.
    let status = match ingest_c_store(scp, &sop_class_uid, &sop_instance_uid, data, pc_id, association) {
        Ok(path) => {
            ctx.emit_lifecycle(
                Status::Success,
                format!("stored {sop_instance_uid} → {}", path.display()),
            );
            STATUS_SUCCESS
        }
        Err(err) => {
            ctx.emit_lifecycle(
                Status::Error,
                format!("C-STORE ingest failed: {err}"),
            );
            // Treat decode / validation failures as "processing
            // failure" and disk/IO failures as "out of resources".
            match err {
                AppError::Io(_) => STATUS_REFUSED_OUT_OF_RESOURCES,
                _ => STATUS_FAILED_UNABLE_TO_PROCESS,
            }
        }
    };

    let rsp = build_c_store_rsp(message_id, &sop_class_uid, &sop_instance_uid, status);
    let bytes = encode_command_set(&rsp)?;
    association
        .send(&Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: pc_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: bytes,
            }],
        })
        .map_err(|e| AppError::Internal(format!("send C-STORE-RSP: {e}")))?;

    let status_label = match status {
        STATUS_SUCCESS => "0x0000 (Success)",
        STATUS_REFUSED_OUT_OF_RESOURCES => "0xA700 (Refused: Out of Resources)",
        STATUS_FAILED_UNABLE_TO_PROCESS => "0xC000 (Failed: Unable to Process)",
        _ => "unknown",
    };
    ctx.emit_outbound(
        "C-STORE-RSP",
        format!("message id {message_id} status {status_label}"),
    );

    Ok(true)
}

/// Inner half of `handle_c_store`: decode the data set, validate the
/// UIDs, write a Part-10 file, refresh the SQLite index. Any error
/// flows back to `handle_c_store` which translates it into the right
/// DIMSE failure status.
fn ingest_c_store(
    scp: &ScpContext,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    data: &[u8],
    pc_id: u8,
    association: &ServerAssociation<TcpStream>,
) -> Result<PathBuf, AppError> {
    let ts_uid = transfer_syntax_uid_for(association, pc_id)?;
    let ts = lookup_ts(&ts_uid)?;

    // Parse the inbound data set with the negotiated transfer syntax.
    let dataset = InMemDicomObject::read_dataset_with_ts(data, ts)
        .map_err(|e| AppError::DicomParse(format!("C-STORE data set decode: {e}")))?;

    // Derive the on-disk layout from the data set itself when present
    // — the data set's UIDs are authoritative — and fall back to the
    // command set's Affected* UIDs otherwise.
    let study_uid = read_str(&dataset, tags::STUDY_INSTANCE_UID)
        .map_err(|e| AppError::DicomParse(format!("missing StudyInstanceUID: {e}")))?;
    let series_uid = read_str(&dataset, tags::SERIES_INSTANCE_UID)
        .map_err(|e| AppError::DicomParse(format!("missing SeriesInstanceUID: {e}")))?;

    if !is_safe_uid(&study_uid) {
        return Err(AppError::DicomParse(format!(
            "StudyInstanceUID is not a syntactically valid UID: {study_uid:?}"
        )));
    }
    if !is_safe_uid(&series_uid) {
        return Err(AppError::DicomParse(format!(
            "SeriesInstanceUID is not a syntactically valid UID: {series_uid:?}"
        )));
    }
    if !is_safe_uid(sop_instance_uid) {
        return Err(AppError::DicomParse(format!(
            "SOPInstanceUID is not a syntactically valid UID: {sop_instance_uid:?}"
        )));
    }

    let target_dir = scp.store_dir.join(&study_uid).join(&series_uid);
    std::fs::create_dir_all(&target_dir)
        .map_err(|e| AppError::Io(format!("create {}: {e}", target_dir.display())))?;
    let target = target_dir.join(format!("{sop_instance_uid}.dcm"));

    // Wrap the in-memory data set with a Part-10 file meta header so
    // the on-disk file is openable by any DICOM tool.
    let file_obj = dataset
        .with_meta(
            FileMetaTableBuilder::new()
                .transfer_syntax(&ts_uid)
                .media_storage_sop_class_uid(sop_class_uid)
                .media_storage_sop_instance_uid(sop_instance_uid),
        )
        .map_err(|e| AppError::DicomParse(format!("build file meta: {e}")))?;
    file_obj
        .write_to_file(&target)
        .map_err(|e| AppError::Io(format!("write {}: {e}", target.display())))?;

    // Refresh the SQLite index from the file we just wrote. Re-parsing
    // here is the cost of consistency: the index reflects exactly what
    // is on disk.
    scp.index.ingest_file(&target)?;

    Ok(target)
}

fn build_c_store_rsp(
    message_id: u16,
    sop_class_uid: &str,
    sop_instance_uid: &str,
    status: u16,
) -> InMemDicomObject {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            sop_class_uid.to_string(),
        ),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_STORE_RSP]),
        ),
        DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            dicom_value!(U16, [message_id]),
        ),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [NO_DATASET]),
        ),
        DataElement::new(tags::STATUS, VR::US, dicom_value!(U16, [status])),
        DataElement::new(
            tags::AFFECTED_SOP_INSTANCE_UID,
            VR::UI,
            sop_instance_uid.to_string(),
        ),
    ])
}

/// Filesystem-safe DICOM UID check: 1-64 ASCII chars, only digits and
/// dots, no leading or trailing dot, no `..` segment. Reject anything
/// else before using a UID as a path component.
fn is_safe_uid(uid: &str) -> bool {
    if uid.is_empty() || uid.len() > 64 {
        return false;
    }
    if uid.starts_with('.') || uid.ends_with('.') {
        return false;
    }
    if uid.contains("..") {
        return false;
    }
    uid.chars().all(|c| c.is_ascii_digit() || c == '.')
}

// ---------------------------------------------------------------------
// C-MOVE handler (M6)
// ---------------------------------------------------------------------

fn handle_c_move(
    association: &mut ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    scp: &ScpContext,
    command: &InMemDicomObject,
    identifier_bytes: &[u8],
    pc_id: u8,
) -> Result<bool, AppError> {
    let message_id = read_u16(command, tags::MESSAGE_ID)?;
    let sop_class_uid = read_str(command, tags::AFFECTED_SOP_CLASS_UID)?;
    let move_destination = read_str(command, tags::MOVE_DESTINATION)?;

    let ts_uid = transfer_syntax_uid_for(association, pc_id)?;
    let ts = lookup_ts(&ts_uid)?;
    let request_identifier = InMemDicomObject::read_dataset_with_ts(identifier_bytes, ts)
        .map_err(|e| AppError::Internal(format!("C-MOVE identifier decode: {e}")))?;
    let level_str = read_str(&request_identifier, tags::QUERY_RETRIEVE_LEVEL)?;
    let level = match parse_qr_level(&level_str) {
        Some(l) => l,
        None => {
            ctx.emit_inbound(
                "C-MOVE-RQ",
                format!("message id {message_id} unknown level {level_str}"),
            );
            send_move_final(
                association,
                &sop_class_uid,
                message_id,
                pc_id,
                STATUS_FAILED_IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
                0,
                0,
                0,
            )?;
            return Ok(true);
        }
    };

    ctx.emit_inbound(
        "C-MOVE-RQ",
        format!("message id {message_id} level {level_str} destination {move_destination}"),
    );

    // Resolve the Move Destination AE Title against the configured
    // peer list.
    let destination = match scp.peers.find_by_ae_title(&move_destination)? {
        Some(p) => p,
        None => {
            ctx.emit_lifecycle(
                Status::Error,
                format!("unknown Move Destination AE {move_destination}"),
            );
            send_move_final(
                association,
                &sop_class_uid,
                message_id,
                pc_id,
                STATUS_MOVE_DESTINATION_UNKNOWN,
                0,
                0,
                0,
            )?;
            ctx.emit_outbound(
                "C-MOVE-RSP",
                format!("final status 0xA801 (Move Destination unknown: {move_destination})"),
            );
            return Ok(true);
        }
    };

    // Resolve the matched SOP Instances.
    let query = build_find_query(&request_identifier, level);
    let instances = scp.index.resolve_for_retrieve(&query)?;
    let total = instances.len() as u16;

    if total == 0 {
        send_move_final(
            association,
            &sop_class_uid,
            message_id,
            pc_id,
            STATUS_SUCCESS,
            0,
            0,
            0,
        )?;
        ctx.emit_outbound(
            "C-MOVE-RSP",
            "final status 0x0000 (Success) — zero matches".to_string(),
        );
        return Ok(true);
    }

    // Open an SCU association to the destination negotiating every
    // SOP Class we know we'll need (and a few extras — over-offering
    // is cheap and saves a reconnect if there's a mix of modalities).
    let mut scu = match open_storage_scu(&scp.local_ae_title, &destination.ae_title, &destination.host, destination.port, &instances) {
        Ok(s) => s,
        Err(err) => {
            ctx.emit_lifecycle(
                Status::Error,
                format!("could not open SCU association to {}: {err}", destination.ae_title),
            );
            send_move_final(
                association,
                &sop_class_uid,
                message_id,
                pc_id,
                STATUS_FAILED_UNABLE_TO_PROCESS,
                0,
                total,
                0,
            )?;
            return Ok(true);
        }
    };

    let mut completed: u16 = 0;
    let mut failed: u16 = 0;

    for instance in &instances {
        let result = forward_via_c_store(
            &mut scu,
            instance,
            Some((scp.local_ae_title.as_str(), message_id)),
        );
        match result {
            Ok(_) => {
                completed += 1;
                ctx.emit_outbound(
                    "C-STORE-RQ",
                    format!("→ {} {}/{}", destination.ae_title, completed, total),
                );
            }
            Err(err) => {
                failed += 1;
                ctx.emit_lifecycle(
                    Status::Warning,
                    format!(
                        "C-STORE sub-op to {} failed for {}: {err}",
                        destination.ae_title, instance.sop_instance_uid
                    ),
                );
            }
        }
        let remaining = total - completed - failed;
        send_move_pending(
            association,
            &sop_class_uid,
            message_id,
            pc_id,
            completed,
            remaining,
            failed,
        )?;
    }

    let _ = scu.release();

    let final_status = if failed == 0 {
        STATUS_SUCCESS
    } else if completed > 0 {
        STATUS_WARNING_SUBOPS_COMPLETE_WITH_FAILURES
    } else {
        STATUS_FAILED_UNABLE_TO_PROCESS
    };

    send_move_final(
        association,
        &sop_class_uid,
        message_id,
        pc_id,
        final_status,
        completed,
        0,
        failed,
    )?;

    ctx.emit_outbound(
        "C-MOVE-RSP",
        format!(
            "final status 0x{:04X} — completed {completed} failed {failed} (of {total})",
            final_status
        ),
    );

    Ok(true)
}

// ---------------------------------------------------------------------
// C-GET handler (M6)
// ---------------------------------------------------------------------

fn handle_c_get(
    association: &mut ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    scp: &ScpContext,
    command: &InMemDicomObject,
    identifier_bytes: &[u8],
    pc_id: u8,
) -> Result<bool, AppError> {
    let message_id = read_u16(command, tags::MESSAGE_ID)?;
    let sop_class_uid = read_str(command, tags::AFFECTED_SOP_CLASS_UID)?;

    let ts_uid = transfer_syntax_uid_for(association, pc_id)?;
    let ts = lookup_ts(&ts_uid)?;
    let request_identifier = InMemDicomObject::read_dataset_with_ts(identifier_bytes, ts)
        .map_err(|e| AppError::Internal(format!("C-GET identifier decode: {e}")))?;
    let level_str = read_str(&request_identifier, tags::QUERY_RETRIEVE_LEVEL)?;
    let level = match parse_qr_level(&level_str) {
        Some(l) => l,
        None => {
            send_get_final(
                association,
                &sop_class_uid,
                message_id,
                pc_id,
                STATUS_FAILED_IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
                0,
                0,
                0,
            )?;
            return Ok(true);
        }
    };

    ctx.emit_inbound(
        "C-GET-RQ",
        format!("message id {message_id} level {level_str}"),
    );

    let query = build_find_query(&request_identifier, level);
    let instances = scp.index.resolve_for_retrieve(&query)?;
    let total = instances.len() as u16;

    let mut completed: u16 = 0;
    let mut failed: u16 = 0;

    // C-GET sub-operations go back over the SAME association the
    // C-GET-RQ came in on. The peer must have negotiated SCP-role
    // presentation contexts for the Storage SOP Classes; if they did
    // not, we'll get "no matching presentation context" failures per
    // instance and report them as sub-operation failures.
    for instance in &instances {
        match send_c_store_on_existing_assoc(association, instance) {
            Ok(_) => completed += 1,
            Err(err) => {
                failed += 1;
                ctx.emit_lifecycle(
                    Status::Warning,
                    format!(
                        "C-STORE sub-op on requester association failed for {}: {err}",
                        instance.sop_instance_uid
                    ),
                );
            }
        }
        let remaining = total - completed - failed;
        send_get_pending(
            association,
            &sop_class_uid,
            message_id,
            pc_id,
            completed,
            remaining,
            failed,
        )?;
    }

    let final_status = if failed == 0 {
        STATUS_SUCCESS
    } else if completed > 0 {
        STATUS_WARNING_SUBOPS_COMPLETE_WITH_FAILURES
    } else {
        STATUS_FAILED_UNABLE_TO_PROCESS
    };

    send_get_final(
        association,
        &sop_class_uid,
        message_id,
        pc_id,
        final_status,
        completed,
        0,
        failed,
    )?;

    ctx.emit_outbound(
        "C-GET-RSP",
        format!(
            "final status 0x{:04X} — completed {completed} failed {failed} (of {total})",
            final_status
        ),
    );

    Ok(true)
}

// ---------------------------------------------------------------------
// SCU C-STORE forwarding (M6, shared by C-MOVE outbound and the
// future M8 SCU page)
// ---------------------------------------------------------------------

/// Opens an outbound association to `peer` and negotiates every SOP
/// Class actually present in `instances` (plus the universal pair of
/// Implicit / Explicit VR LE transfer syntaxes for each).
fn open_storage_scu(
    local_ae: &str,
    called_ae: &str,
    host: &str,
    port: u16,
    instances: &[RetrieveInstance],
) -> Result<ClientAssociation<TcpStream>, AppError> {
    let mut distinct_sop_classes: Vec<&str> = instances
        .iter()
        .map(|i| i.sop_class_uid.as_str())
        .collect();
    distinct_sop_classes.sort();
    distinct_sop_classes.dedup();

    let mut options = ClientAssociationOptions::new()
        .calling_ae_title(local_ae.to_string())
        .called_ae_title(called_ae.to_string())
        .max_pdu_length(MAX_PDU_LENGTH);
    for sop in distinct_sop_classes {
        options = options.with_presentation_context(
            sop.to_string(),
            vec![
                EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                IMPLICIT_VR_LITTLE_ENDIAN.to_string(),
            ],
        );
    }
    options
        .establish(format!("{host}:{port}"))
        .map_err(|e| AppError::Internal(format!("SCU establish to {host}:{port}: {e}")))
}

/// Sends one SOP Instance from disk over the outbound SCU
/// association, optionally tagging it with the Move Originator AE +
/// MessageID (set for C-MOVE sub-ops, None for plain SCU usage).
fn forward_via_c_store(
    scu: &mut ClientAssociation<TcpStream>,
    instance: &RetrieveInstance,
    move_originator: Option<(&str, u16)>,
) -> Result<(), AppError> {
    let pc = scu
        .presentation_contexts()
        .iter()
        .find(|p| {
            p.reason == PresentationContextResultReason::Acceptance
                && p.abstract_syntax == instance.sop_class_uid
        })
        .ok_or_else(|| {
            AppError::Internal(format!(
                "destination did not accept presentation context for SOP class {}",
                instance.sop_class_uid
            ))
        })?;
    let pc_id = pc.id;
    let negotiated_ts_uid = pc.transfer_syntax.clone();
    let negotiated_ts = lookup_ts(&negotiated_ts_uid)?;

    let file_obj = open_file(&instance.file_path)
        .map_err(|e| AppError::DicomParse(format!("open {}: {e}", instance.file_path)))?;

    // Encode the data set in the negotiated transfer syntax. For
    // Implicit VR LE ↔ Explicit VR LE this is a re-encoding of the
    // header bytes only — the pixel data flows through unchanged. For
    // a JPEG-to-uncompressed transcode we'd need pixel decoding (not
    // implemented at M6; the destination should accept the same TS
    // we have on disk in the common case).
    let mut data_bytes: Vec<u8> = Vec::new();
    file_obj
        .write_dataset_with_ts(&mut data_bytes, negotiated_ts)
        .map_err(|e| AppError::DicomParse(format!("re-encode for SCU: {e}")))?;

    let message_id = next_scu_message_id();

    let mut command_elements = vec![
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            instance.sop_class_uid.clone(),
        ),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_STORE_RQ]),
        ),
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        // Priority = Medium (PS3.7 §9.3.5).
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [0u16])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [DATASET_PRESENT]),
        ),
        DataElement::new(
            tags::AFFECTED_SOP_INSTANCE_UID,
            VR::UI,
            instance.sop_instance_uid.clone(),
        ),
    ];
    if let Some((ae, mid)) = move_originator {
        command_elements.push(DataElement::new(
            tags::MOVE_ORIGINATOR_APPLICATION_ENTITY_TITLE,
            VR::AE,
            ae.to_string(),
        ));
        command_elements.push(DataElement::new(
            tags::MOVE_ORIGINATOR_MESSAGE_ID,
            VR::US,
            dicom_value!(U16, [mid]),
        ));
    }
    let command_obj = InMemDicomObject::command_from_element_iter(command_elements);
    let cmd_bytes = encode_command_set(&command_obj)?;

    // Send the command set as a single Command PDV (always small —
    // fits in any negotiated PDU size).
    scu.send(&Pdu::PData {
        data: vec![PDataValue {
            presentation_context_id: pc_id,
            value_type: PDataValueType::Command,
            is_last: true,
            data: cmd_bytes,
        }],
    })
    .map_err(|e| AppError::Internal(format!("SCU send C-STORE-RQ command: {e}")))?;

    // Stream the data set via PDataWriter, which chunks across as
    // many Data PDVs as the negotiated max PDU length requires. The
    // typical destination caps PDUs at ~16 KB while a single MR slice
    // is ~500 KB.
    {
        let mut writer = scu.send_pdata(pc_id);
        writer
            .write_all(&data_bytes)
            .map_err(|e| AppError::Internal(format!("SCU send C-STORE-RQ data: {e}")))?;
        writer
            .finish()
            .map_err(|e| AppError::Internal(format!("SCU send C-STORE-RQ flush: {e}")))?;
    }

    // Drain command + (optional) data PDVs of the response. Expect a
    // C-STORE-RSP with status 0x0000 — anything else is a failure.
    let mut response_command: Option<InMemDicomObject> = None;
    loop {
        let pdu = scu
            .receive()
            .map_err(|e| AppError::Internal(format!("SCU receive C-STORE-RSP: {e}")))?;
        match pdu {
            Pdu::PData { data } => {
                for pdv in data {
                    if pdv.value_type == PDataValueType::Command {
                        let cmd = parse_command_set(&pdv.data)?;
                        response_command = Some(cmd);
                    }
                }
                if response_command.is_some() {
                    break;
                }
            }
            Pdu::AbortRQ { source } => {
                return Err(AppError::Internal(format!(
                    "destination aborted association: {:?}",
                    source
                )));
            }
            other => {
                return Err(AppError::Internal(format!(
                    "unexpected PDU while waiting for C-STORE-RSP: {}",
                    other.short_description()
                )));
            }
        }
    }
    let cmd = response_command.expect("checked above");
    let status = read_u16(&cmd, tags::STATUS)?;
    if status != STATUS_SUCCESS {
        return Err(AppError::Internal(format!(
            "destination returned C-STORE-RSP status 0x{:04X}",
            status
        )));
    }
    Ok(())
}

/// SCU-style send of one SOP Instance over an *already-established*
/// `ServerAssociation` — used by C-GET to forward sub-operations back
/// over the requester's own association. The C-GET requester must
/// have negotiated SCP-role presentation contexts for the Storage
/// SOP Classes; if not, this returns an error and C-GET counts a
/// failure for that instance.
fn send_c_store_on_existing_assoc(
    association: &mut ServerAssociation<TcpStream>,
    instance: &RetrieveInstance,
) -> Result<(), AppError> {
    let pc = association
        .presentation_contexts()
        .iter()
        .find(|p| p.abstract_syntax == instance.sop_class_uid)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "no presentation context for SOP class {} on requester association",
                instance.sop_class_uid
            ))
        })?;
    let pc_id = pc.id;
    let negotiated_ts_uid = pc.transfer_syntax.clone();
    let negotiated_ts = lookup_ts(&negotiated_ts_uid)?;

    let file_obj = open_file(&instance.file_path)
        .map_err(|e| AppError::DicomParse(format!("open {}: {e}", instance.file_path)))?;

    let mut data_bytes: Vec<u8> = Vec::new();
    file_obj
        .write_dataset_with_ts(&mut data_bytes, negotiated_ts)
        .map_err(|e| AppError::DicomParse(format!("re-encode for C-GET sub-op: {e}")))?;

    let message_id = next_scu_message_id();

    let command_obj = InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            instance.sop_class_uid.clone(),
        ),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_STORE_RQ]),
        ),
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [0u16])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [DATASET_PRESENT]),
        ),
        DataElement::new(
            tags::AFFECTED_SOP_INSTANCE_UID,
            VR::UI,
            instance.sop_instance_uid.clone(),
        ),
    ]);
    let cmd_bytes = encode_command_set(&command_obj)?;

    association
        .send(&Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: pc_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: cmd_bytes,
            }],
        })
        .map_err(|e| AppError::Internal(format!("C-GET sub-op send command: {e}")))?;

    // Stream the data set via PDataWriter — same chunking concern as
    // the outbound SCU forward.
    {
        let mut writer = association.send_pdata(pc_id);
        writer
            .write_all(&data_bytes)
            .map_err(|e| AppError::Internal(format!("C-GET sub-op write data: {e}")))?;
        writer
            .finish()
            .map_err(|e| AppError::Internal(format!("C-GET sub-op flush data: {e}")))?;
    }
    // We do NOT receive a C-STORE-RSP here — the C-GET requester does
    // not respond to sub-operations the way a normal Storage SCP
    // would. Per PS3.7 §9.1.3, sub-operations are unacknowledged at
    // the DIMSE level: the requester counts received instances and we
    // count attempts.

    Ok(())
}

/// Process-wide monotonic message id for SCU operations. We start at
/// 1 because some legacy receivers treat 0 as a sentinel.
fn next_scu_message_id() -> u16 {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Wrap around the u16 range so we never overflow. Realistic
    // session lengths stay well below 65535 sub-operations.
    ((n % u16::MAX as u64) as u16).max(1)
}

fn parse_qr_level(s: &str) -> Option<FindLevel> {
    match s {
        "PATIENT" => Some(FindLevel::Patient),
        "STUDY" => Some(FindLevel::Study),
        "SERIES" => Some(FindLevel::Series),
        "IMAGE" => Some(FindLevel::Image),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// C-MOVE / C-GET response builders
// ---------------------------------------------------------------------

fn build_c_move_rsp(
    message_id: u16,
    sop_class_uid: &str,
    status: u16,
    completed: u16,
    remaining: u16,
    failed: u16,
) -> InMemDicomObject {
    build_subop_rsp(
        message_id,
        sop_class_uid,
        cmd::C_MOVE_RSP,
        status,
        completed,
        remaining,
        failed,
    )
}

fn build_c_get_rsp(
    message_id: u16,
    sop_class_uid: &str,
    status: u16,
    completed: u16,
    remaining: u16,
    failed: u16,
) -> InMemDicomObject {
    build_subop_rsp(
        message_id,
        sop_class_uid,
        cmd::C_GET_RSP,
        status,
        completed,
        remaining,
        failed,
    )
}

fn build_subop_rsp(
    message_id: u16,
    sop_class_uid: &str,
    command_field: u16,
    status: u16,
    completed: u16,
    remaining: u16,
    failed: u16,
) -> InMemDicomObject {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            sop_class_uid.to_string(),
        ),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [command_field]),
        ),
        DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            dicom_value!(U16, [message_id]),
        ),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [NO_DATASET]),
        ),
        DataElement::new(tags::STATUS, VR::US, dicom_value!(U16, [status])),
        DataElement::new(
            tags::NUMBER_OF_REMAINING_SUBOPERATIONS,
            VR::US,
            dicom_value!(U16, [remaining]),
        ),
        DataElement::new(
            tags::NUMBER_OF_COMPLETED_SUBOPERATIONS,
            VR::US,
            dicom_value!(U16, [completed]),
        ),
        DataElement::new(
            tags::NUMBER_OF_FAILED_SUBOPERATIONS,
            VR::US,
            dicom_value!(U16, [failed]),
        ),
        DataElement::new(
            tags::NUMBER_OF_WARNING_SUBOPERATIONS,
            VR::US,
            dicom_value!(U16, [0u16]),
        ),
    ])
}

fn send_move_pending(
    association: &mut ServerAssociation<TcpStream>,
    sop_class_uid: &str,
    message_id: u16,
    pc_id: u8,
    completed: u16,
    remaining: u16,
    failed: u16,
) -> Result<(), AppError> {
    let rsp = build_c_move_rsp(
        message_id,
        sop_class_uid,
        STATUS_PENDING,
        completed,
        remaining,
        failed,
    );
    send_command_only(association, pc_id, &rsp)
}

fn send_move_final(
    association: &mut ServerAssociation<TcpStream>,
    sop_class_uid: &str,
    message_id: u16,
    pc_id: u8,
    status: u16,
    completed: u16,
    remaining: u16,
    failed: u16,
) -> Result<(), AppError> {
    let rsp = build_c_move_rsp(
        message_id,
        sop_class_uid,
        status,
        completed,
        remaining,
        failed,
    );
    send_command_only(association, pc_id, &rsp)
}

fn send_get_pending(
    association: &mut ServerAssociation<TcpStream>,
    sop_class_uid: &str,
    message_id: u16,
    pc_id: u8,
    completed: u16,
    remaining: u16,
    failed: u16,
) -> Result<(), AppError> {
    let rsp = build_c_get_rsp(
        message_id,
        sop_class_uid,
        STATUS_PENDING,
        completed,
        remaining,
        failed,
    );
    send_command_only(association, pc_id, &rsp)
}

fn send_get_final(
    association: &mut ServerAssociation<TcpStream>,
    sop_class_uid: &str,
    message_id: u16,
    pc_id: u8,
    status: u16,
    completed: u16,
    remaining: u16,
    failed: u16,
) -> Result<(), AppError> {
    let rsp = build_c_get_rsp(
        message_id,
        sop_class_uid,
        status,
        completed,
        remaining,
        failed,
    );
    send_command_only(association, pc_id, &rsp)
}

fn send_command_only(
    association: &mut ServerAssociation<TcpStream>,
    pc_id: u8,
    command: &InMemDicomObject,
) -> Result<(), AppError> {
    let bytes = encode_command_set(command)?;
    association
        .send(&Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: pc_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: bytes,
            }],
        })
        .map_err(|e| AppError::Internal(format!("send subop RSP: {e}")))
}

// ---------------------------------------------------------------------
// Encoding / decoding helpers (M4)
// ---------------------------------------------------------------------

fn transfer_syntax_uid_for(
    association: &ServerAssociation<TcpStream>,
    pc_id: u8,
) -> Result<String, AppError> {
    association
        .presentation_contexts()
        .iter()
        .find(|p| p.id == pc_id)
        .map(|p| p.transfer_syntax.clone())
        .ok_or_else(|| {
            AppError::Internal(format!("no negotiated presentation context for id {pc_id}"))
        })
}

fn lookup_ts(uid: &str) -> Result<&'static TransferSyntax, AppError> {
    TransferSyntaxRegistry
        .get(uid)
        .ok_or_else(|| AppError::Internal(format!("transfer syntax not in registry: {uid}")))
}

fn encode_identifier(
    obj: &InMemDicomObject,
    ts: &TransferSyntax,
) -> Result<Vec<u8>, AppError> {
    let mut bytes: Vec<u8> = Vec::with_capacity(1024);
    obj.write_dataset_with_ts(&mut bytes, ts)
        .map_err(|e| AppError::Internal(format!("identifier encode: {e}")))?;
    let _ = bytes.flush();
    Ok(bytes)
}

fn read_u16(obj: &InMemDicomObject, tag: Tag) -> Result<u16, AppError> {
    obj.element(tag)
        .map_err(|e| AppError::Internal(format!("missing {tag}: {e}")))?
        .uint16()
        .map_err(|e| AppError::Internal(format!("{tag} not US: {e}")))
}

fn read_str(obj: &InMemDicomObject, tag: Tag) -> Result<String, AppError> {
    obj.element(tag)
        .map_err(|e| AppError::Internal(format!("missing {tag}: {e}")))?
        .to_str()
        .map(|c| c.trim().to_string())
        .map_err(|e| AppError::Internal(format!("decode {tag}: {e}")))
}

fn build_c_echo_rsp(message_id: u16) -> InMemDicomObject {
    InMemDicomObject::command_from_element_iter([
        DataElement::new(tags::AFFECTED_SOP_CLASS_UID, VR::UI, VERIFICATION),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_ECHO_RSP]),
        ),
        DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            dicom_value!(U16, [message_id]),
        ),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [NO_DATASET]),
        ),
        DataElement::new(tags::STATUS, VR::US, dicom_value!(U16, [STATUS_SUCCESS])),
    ])
}

fn encode_command_set(obj: &InMemDicomObject) -> Result<Vec<u8>, AppError> {
    let mut bytes: Vec<u8> = Vec::with_capacity(256);
    let ts = TransferSyntaxRegistry
        .get(IMPLICIT_VR_LITTLE_ENDIAN)
        .ok_or_else(|| AppError::Internal("Implicit VR LE not in registry".to_string()))?;
    obj.write_dataset_with_ts(&mut bytes, ts)
        .map_err(|e| AppError::Internal(format!("command set encode: {e}")))?;
    let _ = bytes.flush();
    Ok(bytes)
}

// =====================================================================
// SCU operations (M8) — Phantom as the *initiator* of DIMSE messages.
// The Tauri commands in lib.rs are thin wrappers around these.
// =====================================================================

/// Which Q/R Information Model the SCU should use. Patient Root for
/// queries that start from a patient identifier; Study Root for queries
/// that start from a study/series/image identifier. PS3.4 Annex C.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QrRoot {
    Patient,
    Study,
}

impl QrRoot {
    fn find_uid(self) -> &'static str {
        match self {
            QrRoot::Patient => PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
            QrRoot::Study => STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
        }
    }
    fn move_uid(self) -> &'static str {
        match self {
            QrRoot::Patient => PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE,
            QrRoot::Study => STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_MOVE,
        }
    }
}

/// Inputs the UI hands to the SCU side for a C-FIND / C-MOVE.
///
/// Every field is optional. Empty fields become Universal Matching
/// (return key only). Wildcards (`*`, `?`) are allowed in PatientName
/// and PatientID; backslash-separated UIDs become a list match.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScuQueryKeys {
    pub patient_id: Option<String>,
    pub patient_name: Option<String>,
    pub study_instance_uid: Option<String>,
    pub study_date: Option<String>,
    pub modality: Option<String>,
    pub series_instance_uid: Option<String>,
    pub sop_instance_uid: Option<String>,
    /// Additional empty-valued return keys requested by the UI by
    /// well-known name (e.g. "StudyDescription").
    pub return_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScuEchoResult {
    pub success: bool,
    pub status: u16,
    pub elapsed_ms: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScuFindMatch {
    /// One entry per non-empty key in the response identifier, keyed
    /// by human-readable tag name (PatientID, StudyInstanceUID, …).
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScuFindResult {
    pub matches: Vec<ScuFindMatch>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScuMoveResult {
    pub completed: u16,
    pub failed: u16,
    pub status: u16,
    pub status_label: String,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScuStoreOutcome {
    pub file: String,
    pub success: bool,
    pub sop_instance_uid: Option<String>,
    pub message: String,
}

/// Sends a C-ECHO-RQ to `peer` and reports whether the round-trip
/// completed successfully.
pub fn scu_echo(local_ae: &str, peer: &Peer) -> Result<ScuEchoResult, AppError> {
    let start = Instant::now();

    let mut association = ClientAssociationOptions::new()
        .calling_ae_title(local_ae.to_string())
        .called_ae_title(peer.ae_title.clone())
        .max_pdu_length(MAX_PDU_LENGTH)
        .with_abstract_syntax(VERIFICATION)
        .establish(format!("{}:{}", peer.host, peer.port))
        .map_err(|e| AppError::Internal(format!("SCU echo establish: {e}")))?;

    let pc = association
        .presentation_contexts()
        .iter()
        .find(|p| {
            p.reason == PresentationContextResultReason::Acceptance
                && p.abstract_syntax == VERIFICATION
        })
        .ok_or_else(|| {
            AppError::Internal("peer did not accept Verification presentation context".to_string())
        })?;
    let pc_id = pc.id;

    let message_id = next_scu_message_id();
    let cmd = InMemDicomObject::command_from_element_iter([
        DataElement::new(tags::AFFECTED_SOP_CLASS_UID, VR::UI, VERIFICATION),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_ECHO_RQ]),
        ),
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [NO_DATASET]),
        ),
    ]);
    let cmd_bytes = encode_command_set(&cmd)?;

    association
        .send(&Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: pc_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: cmd_bytes,
            }],
        })
        .map_err(|e| AppError::Internal(format!("SCU echo send: {e}")))?;

    let response_command = receive_response_command(&mut association)?;
    let status = read_u16(&response_command, tags::STATUS)?;
    let _ = association.release();

    let elapsed_ms = start.elapsed().as_millis() as u64;
    Ok(ScuEchoResult {
        success: status == STATUS_SUCCESS,
        status,
        elapsed_ms,
        message: format!("C-ECHO-RSP status 0x{:04X} in {elapsed_ms} ms", status),
    })
}

/// Sends a C-FIND-RQ to `peer` and collects the Pending response
/// identifiers as `ScuFindMatch` rows.
pub fn scu_find(
    local_ae: &str,
    peer: &Peer,
    root: QrRoot,
    level: FindLevel,
    keys: ScuQueryKeys,
) -> Result<ScuFindResult, AppError> {
    let start = Instant::now();
    let sop_class = root.find_uid();

    let mut association = ClientAssociationOptions::new()
        .calling_ae_title(local_ae.to_string())
        .called_ae_title(peer.ae_title.clone())
        .max_pdu_length(MAX_PDU_LENGTH)
        .with_abstract_syntax(sop_class)
        .establish(format!("{}:{}", peer.host, peer.port))
        .map_err(|e| AppError::Internal(format!("SCU find establish: {e}")))?;

    let (pc_id, ts_uid) = accepted_pc(&association, sop_class)?;
    let ts = lookup_ts(&ts_uid)?;

    let identifier = build_scu_identifier(level, &keys);
    let identifier_bytes = encode_identifier(&identifier, ts)?;

    let message_id = next_scu_message_id();
    let cmd = InMemDicomObject::command_from_element_iter([
        DataElement::new(tags::AFFECTED_SOP_CLASS_UID, VR::UI, sop_class.to_string()),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_FIND_RQ]),
        ),
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [0u16])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [DATASET_PRESENT]),
        ),
    ]);
    let cmd_bytes = encode_command_set(&cmd)?;

    association
        .send(&Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: pc_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: cmd_bytes,
            }],
        })
        .map_err(|e| AppError::Internal(format!("SCU find send command: {e}")))?;
    {
        let mut writer = association.send_pdata(pc_id);
        writer
            .write_all(&identifier_bytes)
            .map_err(|e| AppError::Internal(format!("SCU find send identifier: {e}")))?;
        writer
            .finish()
            .map_err(|e| AppError::Internal(format!("SCU find flush identifier: {e}")))?;
    }

    // Drain response stream until we see a non-Pending command.
    let mut matches: Vec<ScuFindMatch> = Vec::new();
    let mut current_command: Option<InMemDicomObject> = None;
    let mut accumulated_data: Vec<u8> = Vec::new();

    loop {
        let pdu = association
            .receive()
            .map_err(|e| AppError::Internal(format!("SCU find receive: {e}")))?;
        match pdu {
            Pdu::PData { data } => {
                for pdv in data {
                    match pdv.value_type {
                        PDataValueType::Command => {
                            let cmd_obj = parse_command_set(&pdv.data)?;
                            let status = read_u16(&cmd_obj, tags::STATUS)?;
                            if status == STATUS_PENDING {
                                current_command = Some(cmd_obj);
                                accumulated_data.clear();
                            } else if status == STATUS_SUCCESS {
                                let _ = association.release();
                                let elapsed_ms = start.elapsed().as_millis() as u64;
                                return Ok(ScuFindResult { matches, elapsed_ms });
                            } else {
                                let _ = association.release();
                                return Err(AppError::Internal(format!(
                                    "C-FIND-RSP final status 0x{:04X}",
                                    status
                                )));
                            }
                        }
                        PDataValueType::Data => {
                            if current_command.is_none() {
                                // Data with no preceding Pending command — peer protocol error.
                                continue;
                            }
                            accumulated_data.extend_from_slice(&pdv.data);
                            if pdv.is_last {
                                let id_obj = InMemDicomObject::read_dataset_with_ts(
                                    accumulated_data.as_slice(),
                                    ts,
                                )
                                .map_err(|e| {
                                    AppError::Internal(format!("C-FIND identifier decode: {e}"))
                                })?;
                                matches.push(extract_match_fields(&id_obj));
                                accumulated_data.clear();
                                current_command = None;
                            }
                        }
                    }
                }
            }
            Pdu::AbortRQ { source } => {
                return Err(AppError::Internal(format!(
                    "C-FIND peer aborted: {:?}",
                    source
                )));
            }
            other => {
                return Err(AppError::Internal(format!(
                    "unexpected pdu during C-FIND: {}",
                    other.short_description()
                )));
            }
        }
    }
}

/// Sends a C-MOVE-RQ to `peer` asking it to send matched instances to
/// `destination_ae`. Returns the final completed/failed counts.
pub fn scu_move(
    local_ae: &str,
    peer: &Peer,
    root: QrRoot,
    level: FindLevel,
    keys: ScuQueryKeys,
    destination_ae: &str,
) -> Result<ScuMoveResult, AppError> {
    let start = Instant::now();
    let sop_class = root.move_uid();

    let mut association = ClientAssociationOptions::new()
        .calling_ae_title(local_ae.to_string())
        .called_ae_title(peer.ae_title.clone())
        .max_pdu_length(MAX_PDU_LENGTH)
        .with_abstract_syntax(sop_class)
        .establish(format!("{}:{}", peer.host, peer.port))
        .map_err(|e| AppError::Internal(format!("SCU move establish: {e}")))?;

    let (pc_id, ts_uid) = accepted_pc(&association, sop_class)?;
    let ts = lookup_ts(&ts_uid)?;

    let identifier = build_scu_identifier(level, &keys);
    let identifier_bytes = encode_identifier(&identifier, ts)?;

    let message_id = next_scu_message_id();
    let cmd = InMemDicomObject::command_from_element_iter([
        DataElement::new(tags::AFFECTED_SOP_CLASS_UID, VR::UI, sop_class.to_string()),
        DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            dicom_value!(U16, [cmd::C_MOVE_RQ]),
        ),
        DataElement::new(tags::MESSAGE_ID, VR::US, dicom_value!(U16, [message_id])),
        DataElement::new(tags::PRIORITY, VR::US, dicom_value!(U16, [0u16])),
        DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            dicom_value!(U16, [DATASET_PRESENT]),
        ),
        DataElement::new(
            tags::MOVE_DESTINATION,
            VR::AE,
            destination_ae.to_string(),
        ),
    ]);
    let cmd_bytes = encode_command_set(&cmd)?;

    association
        .send(&Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: pc_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: cmd_bytes,
            }],
        })
        .map_err(|e| AppError::Internal(format!("SCU move send command: {e}")))?;
    {
        let mut writer = association.send_pdata(pc_id);
        writer
            .write_all(&identifier_bytes)
            .map_err(|e| AppError::Internal(format!("SCU move send identifier: {e}")))?;
        writer
            .finish()
            .map_err(|e| AppError::Internal(format!("SCU move flush identifier: {e}")))?;
    }

    // Drain response stream — track latest counts; stop on non-Pending.
    let mut completed: u16 = 0;
    let mut failed: u16 = 0;
    let final_status;

    loop {
        let pdu = association
            .receive()
            .map_err(|e| AppError::Internal(format!("SCU move receive: {e}")))?;
        match pdu {
            Pdu::PData { data } => {
                let mut got_final = false;
                let mut last_status = STATUS_PENDING;
                for pdv in data {
                    if pdv.value_type == PDataValueType::Command {
                        let cmd_obj = parse_command_set(&pdv.data)?;
                        last_status = read_u16(&cmd_obj, tags::STATUS)?;
                        completed = read_u16(&cmd_obj, tags::NUMBER_OF_COMPLETED_SUBOPERATIONS)
                            .unwrap_or(completed);
                        failed = read_u16(&cmd_obj, tags::NUMBER_OF_FAILED_SUBOPERATIONS)
                            .unwrap_or(failed);
                        if last_status != STATUS_PENDING {
                            got_final = true;
                        }
                    }
                    // Data PDVs on C-MOVE-RSP carry an identifier with
                    // Failed SOP Instance UID List — we ignore for now.
                }
                if got_final {
                    final_status = last_status;
                    break;
                }
            }
            Pdu::AbortRQ { source } => {
                return Err(AppError::Internal(format!(
                    "C-MOVE peer aborted: {:?}",
                    source
                )));
            }
            other => {
                return Err(AppError::Internal(format!(
                    "unexpected pdu during C-MOVE: {}",
                    other.short_description()
                )));
            }
        }
    }
    let _ = association.release();

    let label = match final_status {
        STATUS_SUCCESS => "Success".to_string(),
        STATUS_WARNING_SUBOPS_COMPLETE_WITH_FAILURES => "Warning (sub-op failures)".to_string(),
        STATUS_MOVE_DESTINATION_UNKNOWN => "Move Destination unknown".to_string(),
        STATUS_FAILED_UNABLE_TO_PROCESS => "Failed: Unable to Process".to_string(),
        other => format!("status 0x{:04X}", other),
    };

    Ok(ScuMoveResult {
        completed,
        failed,
        status: final_status,
        status_label: label,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

/// Sends each of `files` to `peer` via C-STORE. Each file is parsed
/// once on the SCU side to discover its SOP Class UID and SOP Instance
/// UID; failures (file unreadable, peer refuses the SOP class, etc.)
/// are recorded per-file rather than aborting the whole batch.
pub fn scu_store(
    local_ae: &str,
    peer: &Peer,
    files: &[PathBuf],
) -> Result<Vec<ScuStoreOutcome>, AppError> {
    // Pre-parse each file to learn its SOP class so we can negotiate
    // only the contexts we need. Any per-file failures here become
    // outcomes the UI can render.
    let mut instances: Vec<(usize, RetrieveInstance)> = Vec::with_capacity(files.len());
    let mut outcomes: Vec<ScuStoreOutcome> = files
        .iter()
        .map(|p| ScuStoreOutcome {
            file: p.to_string_lossy().to_string(),
            success: false,
            sop_instance_uid: None,
            message: String::new(),
        })
        .collect();

    for (idx, path) in files.iter().enumerate() {
        match scan_store_file(path) {
            Ok(inst) => {
                outcomes[idx].sop_instance_uid = Some(inst.sop_instance_uid.clone());
                instances.push((idx, inst));
            }
            Err(err) => {
                outcomes[idx].message = format!("pre-parse failed: {err}");
            }
        }
    }

    if instances.is_empty() {
        return Ok(outcomes);
    }

    // Open one association, negotiating every SOP class present.
    let just_instances: Vec<RetrieveInstance> =
        instances.iter().map(|(_, i)| i.clone()).collect();
    let mut association =
        match open_storage_scu(local_ae, &peer.ae_title, &peer.host, peer.port, &just_instances) {
            Ok(a) => a,
            Err(err) => {
                // Whole batch fails if we can't even open.
                for (idx, _) in &instances {
                    outcomes[*idx].message = format!("could not open SCU association: {err}");
                }
                return Ok(outcomes);
            }
        };

    for (idx, inst) in &instances {
        match forward_via_c_store(&mut association, inst, None) {
            Ok(_) => {
                outcomes[*idx].success = true;
                outcomes[*idx].message = "C-STORE-RSP 0x0000 (Success)".to_string();
            }
            Err(err) => {
                outcomes[*idx].message = format!("{err}");
            }
        }
    }
    let _ = association.release();
    Ok(outcomes)
}

// ---- SCU helpers ----------------------------------------------------

fn accepted_pc(
    association: &ClientAssociation<TcpStream>,
    abstract_syntax: &str,
) -> Result<(u8, String), AppError> {
    let pc = association
        .presentation_contexts()
        .iter()
        .find(|p| {
            p.reason == PresentationContextResultReason::Acceptance
                && p.abstract_syntax == abstract_syntax
        })
        .ok_or_else(|| {
            AppError::Internal(format!(
                "peer did not accept presentation context for {abstract_syntax}"
            ))
        })?;
    Ok((pc.id, pc.transfer_syntax.clone()))
}

fn receive_response_command(
    association: &mut ClientAssociation<TcpStream>,
) -> Result<InMemDicomObject, AppError> {
    loop {
        let pdu = association
            .receive()
            .map_err(|e| AppError::Internal(format!("SCU receive: {e}")))?;
        match pdu {
            Pdu::PData { data } => {
                for pdv in data {
                    if pdv.value_type == PDataValueType::Command {
                        return parse_command_set(&pdv.data);
                    }
                }
            }
            Pdu::AbortRQ { source } => {
                return Err(AppError::Internal(format!("peer aborted: {:?}", source)));
            }
            other => {
                return Err(AppError::Internal(format!(
                    "unexpected pdu while awaiting response: {}",
                    other.short_description()
                )));
            }
        }
    }
}

/// Builds the C-FIND / C-MOVE request identifier from a set of keys
/// supplied by the UI. Empty / missing keys are returned as universal
/// matches (key present with empty value).
fn build_scu_identifier(level: FindLevel, keys: &ScuQueryKeys) -> InMemDicomObject {
    let mut obj = InMemDicomObject::new_empty();

    let level_str = match level {
        FindLevel::Patient => "PATIENT",
        FindLevel::Study => "STUDY",
        FindLevel::Series => "SERIES",
        FindLevel::Image => "IMAGE",
    };
    obj.put_element(DataElement::new(
        tags::QUERY_RETRIEVE_LEVEL,
        VR::CS,
        level_str.to_string(),
    ));

    put_query_value(&mut obj, tags::PATIENT_ID, VR::LO, keys.patient_id.as_deref());
    put_query_value(&mut obj, tags::PATIENT_NAME, VR::PN, keys.patient_name.as_deref());
    put_query_value(
        &mut obj,
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        keys.study_instance_uid.as_deref(),
    );
    put_query_value(&mut obj, tags::STUDY_DATE, VR::DA, keys.study_date.as_deref());
    put_query_value(&mut obj, tags::MODALITY, VR::CS, keys.modality.as_deref());
    put_query_value(
        &mut obj,
        tags::SERIES_INSTANCE_UID,
        VR::UI,
        keys.series_instance_uid.as_deref(),
    );
    put_query_value(
        &mut obj,
        tags::SOP_INSTANCE_UID,
        VR::UI,
        keys.sop_instance_uid.as_deref(),
    );

    // Extra empty-valued return keys.
    for name in &keys.return_keys {
        if let Some((tag, vr)) = lookup_named_tag(name) {
            // Skip if we already inserted this one above.
            if obj.element(tag).is_err() {
                obj.put_element(DataElement::new(tag, vr, String::new()));
            }
        }
    }
    obj
}

fn put_query_value(obj: &mut InMemDicomObject, tag: Tag, vr: VR, value: Option<&str>) {
    let v = value.unwrap_or("").to_string();
    obj.put_element(DataElement::new(tag, vr, v));
}

/// Maps a common DICOM tag name to its `Tag` + VR. Used so the SCU UI
/// can send back-tick a return key by name. Add to the list when new
/// well-known tags become useful.
fn lookup_named_tag(name: &str) -> Option<(Tag, VR)> {
    match name {
        "PatientID" => Some((tags::PATIENT_ID, VR::LO)),
        "PatientName" => Some((tags::PATIENT_NAME, VR::PN)),
        "StudyInstanceUID" => Some((tags::STUDY_INSTANCE_UID, VR::UI)),
        "StudyDate" => Some((tags::STUDY_DATE, VR::DA)),
        "StudyDescription" => Some((tags::STUDY_DESCRIPTION, VR::LO)),
        "ModalitiesInStudy" => Some((tags::MODALITIES_IN_STUDY, VR::CS)),
        "Modality" => Some((tags::MODALITY, VR::CS)),
        "SeriesInstanceUID" => Some((tags::SERIES_INSTANCE_UID, VR::UI)),
        "SeriesDescription" => Some((tags::SERIES_DESCRIPTION, VR::LO)),
        "NumberOfStudyRelatedSeries" => Some((tags::NUMBER_OF_STUDY_RELATED_SERIES, VR::IS)),
        "NumberOfStudyRelatedInstances" => Some((tags::NUMBER_OF_STUDY_RELATED_INSTANCES, VR::IS)),
        "NumberOfSeriesRelatedInstances" => {
            Some((tags::NUMBER_OF_SERIES_RELATED_INSTANCES, VR::IS))
        }
        "SOPInstanceUID" => Some((tags::SOP_INSTANCE_UID, VR::UI)),
        "SOPClassUID" => Some((tags::SOP_CLASS_UID, VR::UI)),
        _ => None,
    }
}

/// Walks every element in a C-FIND response identifier and produces a
/// `{tag_name: value}` map for the UI. Empty values are skipped.
fn extract_match_fields(identifier: &InMemDicomObject) -> ScuFindMatch {
    let mut fields: HashMap<String, String> = HashMap::new();
    for elem in identifier.iter() {
        let tag = elem.header().tag;
        let name = tag_to_friendly_name(tag);
        if let Ok(value) = elem.to_str() {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() {
                fields.insert(name, trimmed);
            }
        }
    }
    ScuFindMatch { fields }
}

/// Reverse-map a `Tag` to its short DICOM name when we know one. For
/// tags we don't have a name for, fall back to the hex form so the UI
/// can still render them.
fn tag_to_friendly_name(tag: Tag) -> String {
    match tag {
        tags::QUERY_RETRIEVE_LEVEL => "QueryRetrieveLevel".to_string(),
        tags::PATIENT_ID => "PatientID".to_string(),
        tags::PATIENT_NAME => "PatientName".to_string(),
        tags::STUDY_INSTANCE_UID => "StudyInstanceUID".to_string(),
        tags::STUDY_DATE => "StudyDate".to_string(),
        tags::STUDY_DESCRIPTION => "StudyDescription".to_string(),
        tags::MODALITIES_IN_STUDY => "ModalitiesInStudy".to_string(),
        tags::MODALITY => "Modality".to_string(),
        tags::SERIES_INSTANCE_UID => "SeriesInstanceUID".to_string(),
        tags::SERIES_DESCRIPTION => "SeriesDescription".to_string(),
        tags::NUMBER_OF_STUDY_RELATED_SERIES => "NumberOfStudyRelatedSeries".to_string(),
        tags::NUMBER_OF_STUDY_RELATED_INSTANCES => "NumberOfStudyRelatedInstances".to_string(),
        tags::NUMBER_OF_SERIES_RELATED_INSTANCES => "NumberOfSeriesRelatedInstances".to_string(),
        tags::SOP_INSTANCE_UID => "SOPInstanceUID".to_string(),
        tags::SOP_CLASS_UID => "SOPClassUID".to_string(),
        tags::ACCESSION_NUMBER => "AccessionNumber".to_string(),
        other => format!("({:04X},{:04X})", other.group(), other.element()),
    }
}

/// Parses one local file enough to feed it into `forward_via_c_store`:
/// SOP Instance UID, SOP Class UID, transfer syntax. Used by
/// `scu_store` so the UI can drive C-STORE without going through the
/// SCP-side ingest path.
fn scan_store_file(path: &Path) -> Result<RetrieveInstance, AppError> {
    let obj = open_file(path)
        .map_err(|e| AppError::DicomParse(format!("open {}: {e}", path.display())))?;
    let transfer_syntax_uid = obj.meta().transfer_syntax.clone();
    let sop_instance_uid = obj
        .element(tags::SOP_INSTANCE_UID)
        .map_err(|e| AppError::DicomParse(format!("missing SOPInstanceUID: {e}")))?
        .to_str()
        .map_err(|e| AppError::DicomParse(format!("decode SOPInstanceUID: {e}")))?
        .trim()
        .to_string();
    let sop_class_uid = obj
        .element(tags::SOP_CLASS_UID)
        .map_err(|e| AppError::DicomParse(format!("missing SOPClassUID: {e}")))?
        .to_str()
        .map_err(|e| AppError::DicomParse(format!("decode SOPClassUID: {e}")))?
        .trim()
        .to_string();
    Ok(RetrieveInstance {
        sop_instance_uid,
        sop_class_uid,
        transfer_syntax_uid,
        file_path: path.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_echo_rsp_round_trips_through_implicit_vr_le() {
        let original = build_c_echo_rsp(42);
        let encoded = encode_command_set(&original).expect("encode");
        let decoded = parse_command_set(&encoded).expect("decode");

        let cmd_field: u16 = decoded
            .element(tags::COMMAND_FIELD)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(cmd_field, cmd::C_ECHO_RSP);

        let mid: u16 = decoded
            .element(tags::MESSAGE_ID_BEING_RESPONDED_TO)
            .unwrap()
            .uint16()
            .unwrap();
        assert_eq!(mid, 42);

        let status: u16 = decoded.element(tags::STATUS).unwrap().uint16().unwrap();
        assert_eq!(status, STATUS_SUCCESS);
    }
}
