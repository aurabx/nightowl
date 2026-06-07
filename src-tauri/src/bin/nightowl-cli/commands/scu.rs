//! `nightowl-cli scu ...` — mirrors `scu_echo`, `scu_find`, `scu_move`
//! and `scu_store`.

use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use nightowl_lib::core::dimse::{
    scu_echo, scu_find, scu_move, scu_store, QrRoot, ScuQueryKeys,
};
use nightowl_lib::core::error::AppError;
use nightowl_lib::core::peers::{Peer, PeerStore};
use nightowl_lib::core::store::FindLevel;

use crate::context::Context;
use crate::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Send a DICOM C-ECHO (the DICOM "ping") to a configured peer.
    Echo {
        /// Peer id (UUID) from `nightowl-cli peers list`.
        peer_id: String,
    },
    /// Send a DICOM C-FIND query and print the response identifiers.
    Find(FindFlags),
    /// Send a DICOM C-MOVE request, asking a peer to transfer matched
    /// instances to the named destination AE.
    Move(MoveFlags),
    /// Send each given file to a peer via DICOM C-STORE.
    Store(StoreFlags),
}

#[derive(Args, Debug)]
pub struct FindFlags {
    /// Peer id (UUID) from `nightowl-cli peers list`.
    pub peer_id: String,
    /// Query/Retrieve information model root.
    #[arg(long, value_enum, default_value_t = QrRootArg::Study)]
    pub root: QrRootArg,
    /// Query/Retrieve level.
    #[arg(long, value_enum, default_value_t = FindLevelArg::Study)]
    pub level: FindLevelArg,
    #[command(flatten)]
    pub keys: QueryKeyFlags,
}

#[derive(Args, Debug)]
pub struct MoveFlags {
    /// Peer id (UUID) from `nightowl-cli peers list`.
    pub peer_id: String,
    /// Destination AE Title — the SCP the matched instances should be
    /// sent to.
    #[arg(long)]
    pub destination_ae: String,
    /// Query/Retrieve information model root.
    #[arg(long, value_enum, default_value_t = QrRootArg::Study)]
    pub root: QrRootArg,
    /// Query/Retrieve level.
    #[arg(long, value_enum, default_value_t = FindLevelArg::Study)]
    pub level: FindLevelArg,
    #[command(flatten)]
    pub keys: QueryKeyFlags,
}

#[derive(Args, Debug)]
pub struct StoreFlags {
    /// Peer id (UUID) from `nightowl-cli peers list`.
    pub peer_id: String,
    /// One or more Part-10 DICOM file paths to send.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
}

/// Mirrors `ScuQueryKeys`. Every flag is optional. Empty fields become
/// Universal Matching (the key is requested in the response but the
/// query does not filter on it).
#[derive(Args, Debug, Default)]
pub struct QueryKeyFlags {
    #[arg(long)]
    pub patient_id: Option<String>,
    #[arg(long)]
    pub patient_name: Option<String>,
    #[arg(long)]
    pub study_uid: Option<String>,
    #[arg(long)]
    pub study_date: Option<String>,
    #[arg(long)]
    pub modality: Option<String>,
    #[arg(long)]
    pub series_uid: Option<String>,
    #[arg(long)]
    pub sop_uid: Option<String>,
    /// Additional empty-valued return keys requested by tag name (e.g.
    /// "StudyDescription"). Repeatable.
    #[arg(long = "return-key")]
    pub return_keys: Vec<String>,
}

impl From<QueryKeyFlags> for ScuQueryKeys {
    fn from(f: QueryKeyFlags) -> Self {
        Self {
            patient_id: f.patient_id,
            patient_name: f.patient_name,
            study_instance_uid: f.study_uid,
            study_date: f.study_date,
            modality: f.modality,
            series_instance_uid: f.series_uid,
            sop_instance_uid: f.sop_uid,
            return_keys: f.return_keys,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum QrRootArg {
    Patient,
    Study,
}

impl From<QrRootArg> for QrRoot {
    fn from(a: QrRootArg) -> Self {
        match a {
            QrRootArg::Patient => QrRoot::Patient,
            QrRootArg::Study => QrRoot::Study,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum FindLevelArg {
    Patient,
    Study,
    Series,
    Image,
}

impl From<FindLevelArg> for FindLevel {
    fn from(a: FindLevelArg) -> Self {
        match a {
            FindLevelArg::Patient => FindLevel::Patient,
            FindLevelArg::Study => FindLevel::Study,
            FindLevelArg::Series => FindLevel::Series,
            FindLevelArg::Image => FindLevel::Image,
        }
    }
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::Echo { peer_id } => echo(ctx, format, &peer_id),
        Action::Find(flags) => find(ctx, format, flags),
        Action::Move(flags) => mv(ctx, format, flags),
        Action::Store(flags) => store(ctx, format, flags),
    }
}

fn echo(ctx: &Context, format: OutputFormat, peer_id: &str) -> Result<(), AppError> {
    let peer = resolve_peer(&ctx.peers, peer_id)?;
    let emitter = ctx.emitter();
    let result = scu_echo(&emitter, &ctx.config.local_ae_title, &peer)?;
    match format {
        OutputFormat::Json => emit_json(&result),
        OutputFormat::Human => emit_text(&format!(
            "{} status=0x{:04X} elapsed={}ms message={}",
            if result.success { "ok" } else { "FAILED" },
            result.status,
            result.elapsed_ms,
            result.message,
        )),
    }
}

fn find(ctx: &Context, format: OutputFormat, flags: FindFlags) -> Result<(), AppError> {
    let peer = resolve_peer(&ctx.peers, &flags.peer_id)?;
    let emitter = ctx.emitter();
    let result = scu_find(
        &emitter,
        &ctx.config.local_ae_title,
        &peer,
        flags.root.into(),
        flags.level.into(),
        flags.keys.into(),
    )?;
    match format {
        OutputFormat::Json => emit_json(&result),
        OutputFormat::Human => emit_text(&format!(
            "{} match(es) in {}ms",
            result.matches.len(),
            result.elapsed_ms,
        )),
    }
}

fn mv(ctx: &Context, format: OutputFormat, flags: MoveFlags) -> Result<(), AppError> {
    let peer = resolve_peer(&ctx.peers, &flags.peer_id)?;
    let emitter = ctx.emitter();
    let result = scu_move(
        &emitter,
        &ctx.config.local_ae_title,
        &peer,
        flags.root.into(),
        flags.level.into(),
        flags.keys.into(),
        &flags.destination_ae,
    )?;
    match format {
        OutputFormat::Json => emit_json(&result),
        OutputFormat::Human => emit_text(&format!(
            "status=0x{:04X} ({}) completed={} failed={} elapsed={}ms",
            result.status, result.status_label, result.completed, result.failed, result.elapsed_ms,
        )),
    }
}

fn store(ctx: &Context, format: OutputFormat, flags: StoreFlags) -> Result<(), AppError> {
    let peer = resolve_peer(&ctx.peers, &flags.peer_id)?;
    let emitter = ctx.emitter();
    let outcomes = scu_store(&emitter, &ctx.config.local_ae_title, &peer, &flags.files)?;
    match format {
        OutputFormat::Json => emit_json(&outcomes),
        OutputFormat::Human => {
            let mut out = String::new();
            for o in &outcomes {
                out.push_str(&format!(
                    "{} {} {}\n",
                    if o.success { "ok " } else { "ERR" },
                    o.file,
                    o.message,
                ));
            }
            emit_text(&out)
        }
    }
}

/// Mirrors the `resolve_peer` helper in `lib.rs`. Repeated here because
/// the original is private to that crate; lifting it into `core` would
/// be an unrelated change.
fn resolve_peer(peers: &PeerStore, id: &str) -> Result<Peer, AppError> {
    peers
        .list()?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::validation("peer_id", format!("unknown peer id {id}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_key_flags_map_to_scu_keys() {
        let flags = QueryKeyFlags {
            patient_id: Some("PAT*".into()),
            patient_name: Some("DOE^*".into()),
            study_uid: Some("1.2.3".into()),
            study_date: Some("20240101-20241231".into()),
            modality: Some("CT".into()),
            series_uid: Some("1.2.3.4".into()),
            sop_uid: Some("1.2.3.4.5".into()),
            return_keys: vec!["StudyDescription".into(), "ReferringPhysicianName".into()],
        };
        let keys: ScuQueryKeys = flags.into();
        assert_eq!(keys.patient_id.as_deref(), Some("PAT*"));
        assert_eq!(keys.patient_name.as_deref(), Some("DOE^*"));
        assert_eq!(keys.study_instance_uid.as_deref(), Some("1.2.3"));
        assert_eq!(keys.study_date.as_deref(), Some("20240101-20241231"));
        assert_eq!(keys.modality.as_deref(), Some("CT"));
        assert_eq!(keys.series_instance_uid.as_deref(), Some("1.2.3.4"));
        assert_eq!(keys.sop_instance_uid.as_deref(), Some("1.2.3.4.5"));
        assert_eq!(keys.return_keys.len(), 2);
        assert_eq!(keys.return_keys[0], "StudyDescription");
    }

    #[test]
    fn empty_query_key_flags_yield_universal_matching() {
        // Universal Matching = every field None, no return keys.
        let keys: ScuQueryKeys = QueryKeyFlags::default().into();
        assert!(keys.patient_id.is_none());
        assert!(keys.study_instance_uid.is_none());
        assert!(keys.return_keys.is_empty());
    }

    #[test]
    fn qr_root_arg_maps_to_core_enum() {
        assert!(matches!(QrRoot::from(QrRootArg::Patient), QrRoot::Patient));
        assert!(matches!(QrRoot::from(QrRootArg::Study), QrRoot::Study));
    }

    #[test]
    fn find_level_arg_maps_to_core_enum() {
        assert!(matches!(
            FindLevel::from(FindLevelArg::Patient),
            FindLevel::Patient
        ));
        assert!(matches!(
            FindLevel::from(FindLevelArg::Study),
            FindLevel::Study
        ));
        assert!(matches!(
            FindLevel::from(FindLevelArg::Series),
            FindLevel::Series
        ));
        assert!(matches!(
            FindLevel::from(FindLevelArg::Image),
            FindLevel::Image
        ));
    }
}
