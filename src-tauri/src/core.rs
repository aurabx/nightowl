//! Shared business logic. Tauri commands in `lib.rs` should remain thin
//! wrappers that call into this module.
//!
//! At M0 this only exposes a smoke-test `ping`. Modules for config, store,
//! peers, activity, and dicom will be added by later milestones.

/// Returns a fixed greeting used by the frontend IPC self-check.
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
