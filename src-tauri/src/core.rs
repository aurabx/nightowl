//! Shared business logic. Tauri commands in `lib.rs` should remain thin
//! wrappers that call into this module.
//!
//! Submodules:
//! - `config`   — persistent application configuration and validators.
//! - `error`    — the single error type returned across the IPC boundary.
//! - `store`    — SQLite-backed SOP Instance index over the local store
//!               directory.
//! - `dimse`    — DIMSE SCP listener (C-ECHO at M3; C-FIND / C-STORE /
//!               C-MOVE / C-GET in later milestones).
//! - `activity` — persistent activity log (M9) plus the typed
//!               `PersistedActivityEvent` shape the UI lists.
//!
//! Later milestones will add `peers`.

pub mod activity;
pub mod config;
pub mod dimse;
pub mod error;
pub mod store;

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
