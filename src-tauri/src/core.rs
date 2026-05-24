//! Shared business logic. Tauri commands in `lib.rs` should remain thin
//! wrappers that call into this module.
//!
//! Submodules:
//! - `config`   — persistent application configuration and validators.
//! - `error`    — the single error type returned across the IPC boundary.
//! - `store`    — SQLite-backed SOP Instance index over the local store
//!               directory.
//! - `dimse`    — DIMSE SCP listener (C-ECHO at M3; C-FIND at M4;
//!               C-STORE at M5; C-MOVE / C-GET in M6).
//! - `activity` — persistent activity log (M9) plus the typed
//!               `PersistedActivityEvent` shape the UI lists.
//! - `peers`    — configured remote DICOM peers, persisted as
//!               peers.json. M7 in the plan, but C-MOVE in M6 needs
//!               it to resolve Move Destination AE Titles.
//! - `worklist` — Modality Worklist (DMWL) entries — scheduled
//!               procedure steps, M11. Served by M12's DMWL SCP.

pub mod activity;
pub mod config;
pub mod dimse;
pub mod error;
pub mod peers;
pub mod store;
pub mod worklist;

/// IPC self-check used by the frontend on mount. Returns the fixed string
/// `"pong"`; if the frontend sees anything else, the IPC channel is broken.
pub fn ping() -> &'static str {
    "pong"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(ping(), "pong");
    }
}
