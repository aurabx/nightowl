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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dicom_core::{dicom_value, DataElement, VR};
use dicom_dictionary_std::tags;
use dicom_dictionary_std::uids::{
    EXPLICIT_VR_LITTLE_ENDIAN, IMPLICIT_VR_LITTLE_ENDIAN, VERIFICATION,
};
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::ServerAssociationOptions;
use dicom_ul::association::Association;
use dicom_ul::pdu::{PDataValue, PDataValueType, Pdu};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use super::activity::{ActivityLog, PersistedActivityEvent};
use super::error::AppError;

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

// DIMSE status codes used at M3.
const STATUS_SUCCESS: u16 = 0x0000;

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

/// Handle to the running SCP listener. Holding this struct alive is
/// what keeps the listener bound; dropping the inner `shutdown` flag
/// causes the accept loop to exit on its next iteration.
pub struct ListenerHandle {
    pub bind_addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
}

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

    std::thread::Builder::new()
        .name("phantom-scp-accept".to_string())
        .spawn(move || run_accept_loop(listener, ae_for_thread, app_for_thread, shutdown_for_thread))
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
        if let Err(err) = std::thread::Builder::new()
            .name(format!("phantom-scp-{}", local_seq))
            .spawn(move || {
                handle_association(stream, peer, ae_clone, app_clone, association_id);
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
    association_id: String,
) {
    let peer_host = peer.map(|p| p.to_string());

    if let Err(err) = stream.set_read_timeout(Some(Duration::from_secs(30))) {
        tracing::warn!(error = %err, "set_read_timeout failed");
    }

    // Negotiate. We accept Verification on Implicit VR LE and Explicit
    // VR LE — the minimum for echoscu. Future milestones bolt on more
    // SOP classes and transfer syntaxes by calling the same builder.
    let options = ServerAssociationOptions::new()
        .accept_any()
        .with_abstract_syntax(VERIFICATION)
        .with_transfer_syntax(IMPLICIT_VR_LITTLE_ENDIAN)
        .with_transfer_syntax(EXPLICIT_VR_LITTLE_ENDIAN)
        .ae_title(local_ae_title.clone());

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
                for value in data {
                    if value.value_type == PDataValueType::Command {
                        match dispatch_command(&mut association, &ctx, &value) {
                            Ok(true) => {} // continue
                            Ok(false) => {
                                // Command dispatcher signalled "abort cleanly".
                                return;
                            }
                            Err(err) => {
                                ctx.emit_lifecycle(
                                    Status::Error,
                                    format!("command dispatch failed: {err}"),
                                );
                                let _ = association
                                    .inner_stream()
                                    .shutdown(Shutdown::Both);
                                return;
                            }
                        }
                    }
                    // Data PDVs are ignored at M3 (C-ECHO has no data
                    // set). M4/M5 will route them to query/storage
                    // handlers.
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

/// Returns `Ok(true)` to continue the receive loop, `Ok(false)` to
/// stop after a clean abort. `Err` indicates a protocol-level failure
/// the caller should treat as fatal for this association.
fn dispatch_command(
    association: &mut dicom_ul::association::server::ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    pdv: &PDataValue,
) -> Result<bool, AppError> {
    let command = parse_command_set(&pdv.data)?;

    let command_field: u16 = command
        .element(tags::COMMAND_FIELD)
        .map_err(|e| AppError::Internal(format!("missing CommandField: {e}")))?
        .uint16()
        .map_err(|e| AppError::Internal(format!("CommandField not US: {e}")))?;

    match command_field {
        cmd::C_ECHO_RQ => handle_c_echo(association, ctx, &command, pdv.presentation_context_id),
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
    association: &mut dicom_ul::association::server::ServerAssociation<TcpStream>,
    ctx: &AssociationCtx,
    command: &InMemDicomObject,
    presentation_context_id: u8,
) -> Result<bool, AppError> {
    let message_id: u16 = command
        .element(tags::MESSAGE_ID)
        .map_err(|e| AppError::Internal(format!("missing MessageID: {e}")))?
        .uint16()
        .map_err(|e| AppError::Internal(format!("MessageID not US: {e}")))?;

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
