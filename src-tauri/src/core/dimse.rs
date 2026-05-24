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

use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dicom_core::{dicom_value, DataElement, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_dictionary_std::uids::{
    COMPUTED_RADIOGRAPHY_IMAGE_STORAGE, CT_IMAGE_STORAGE,
    DIGITAL_X_RAY_IMAGE_STORAGE_FOR_PRESENTATION, ENCAPSULATED_PDF_STORAGE,
    EXPLICIT_VR_LITTLE_ENDIAN, IMPLICIT_VR_LITTLE_ENDIAN, JPEG_BASELINE8_BIT,
    MR_IMAGE_STORAGE, PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
    SECONDARY_CAPTURE_IMAGE_STORAGE, STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND,
    ULTRASOUND_IMAGE_STORAGE, VERIFICATION,
};
use dicom_object::FileMetaTableBuilder;
use dicom_encoding::transfer_syntax::{TransferSyntax, TransferSyntaxIndex};
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::{ServerAssociation, ServerAssociationOptions};
use dicom_ul::association::Association;
use dicom_ul::pdu::{PDataValue, PDataValueType, Pdu};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use super::activity::{ActivityLog, PersistedActivityEvent};
use super::error::AppError;
use super::store::{FindLevel, FindQuery, FindRow, Index, KeyMatch};

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

// DIMSE status codes (PS3.7 Annex C).
const STATUS_SUCCESS: u16 = 0x0000;
const STATUS_PENDING: u16 = 0xFF00;
#[allow(dead_code)]
const STATUS_REFUSED_OUT_OF_RESOURCES: u16 = 0xA700;
#[allow(dead_code)]
const STATUS_FAILED_IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS: u16 = 0xA900;
#[allow(dead_code)]
const STATUS_FAILED_UNABLE_TO_PROCESS: u16 = 0xC000;

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

/// Shared context the per-association threads need: the SOP Instance
/// index for query + ingestion, and the on-disk store directory where
/// inbound C-STORE files land. Cloning the `Arc` is cheap.
pub struct ScpContext {
    pub index: Arc<Index>,
    pub store_dir: PathBuf,
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
        .with_abstract_syntax(VERIFICATION)
        .with_abstract_syntax(PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND)
        .with_abstract_syntax(STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND)
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
        other => {
            ctx.emit_lifecycle(
                Status::Warning,
                format!("unsupported DIMSE command 0x{:04X}", other),
            );
            // Not a fatal protocol error — we just don't implement it
            // yet. The peer will likely time out or release.
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
