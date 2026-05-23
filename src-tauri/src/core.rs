//! Shared business logic. Tauri commands in `lib.rs` should remain thin
//! wrappers that call into this module.
//!
//! Submodules:
//! - `config`  — persistent application configuration and validators.
//! - `error`   — the single error type returned across the IPC boundary.
//!
//! Later milestones will add `store`, `peers`, `activity`, and `dicom`.

pub mod config;
pub mod error;

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
