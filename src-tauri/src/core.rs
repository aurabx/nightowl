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
//! - `mcp`      — local Model Context Protocol server (M24) exposing
//!               read + active SCU tools over Streamable HTTP on
//!               127.0.0.1, disabled by default.

pub mod activity;
pub mod config;
pub mod dimse;
pub mod error;
pub mod mcp;
pub mod peers;
pub mod store;
pub mod worklist;

use crate::core::error::AppError;

/// IPC self-check used by the frontend on mount. Returns the fixed string
/// `"pong"`; if the frontend sees anything else, the IPC channel is broken.
pub fn ping() -> &'static str {
    "pong"
}

/// Opens an http(s) URL in the user's default browser via the OS shell.
///
/// Rejects anything that is not a plain `http://` or `https://` URL so a
/// compromised webview cannot use this command to launch local apps via
/// custom URL schemes (e.g. `file://`, `mailto:`, `vscode://`).
pub fn open_url(url: &str) -> Result<(), AppError> {
    if !is_safe_web_url(url) {
        return Err(AppError::validation(
            "url",
            "only http(s) URLs may be opened",
        ));
    }

    #[cfg(target_os = "macos")]
    let (program, args): (&str, Vec<&str>) = ("open", vec![url]);

    #[cfg(target_os = "linux")]
    let (program, args): (&str, Vec<&str>) = ("xdg-open", vec![url]);

    #[cfg(target_os = "windows")]
    let (program, args): (&str, Vec<&str>) = ("cmd", vec!["/C", "start", "", url]);

    std::process::Command::new(program)
        .args(&args)
        .spawn()
        .map_err(|e| AppError::Internal(format!("failed to launch {program}: {e}")))?;

    Ok(())
}

fn is_safe_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    !url.chars().any(|c| c.is_control() || c == ' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(ping(), "pong");
    }

    #[test]
    fn open_url_accepts_https() {
        assert!(is_safe_web_url("https://aurabox.cloud/nightowl"));
        assert!(is_safe_web_url("http://example.com/path?q=1"));
    }

    #[test]
    fn open_url_rejects_other_schemes() {
        assert!(!is_safe_web_url("file:///etc/passwd"));
        assert!(!is_safe_web_url("javascript:alert(1)"));
        assert!(!is_safe_web_url("mailto:a@b"));
        assert!(!is_safe_web_url(""));
    }

    #[test]
    fn open_url_rejects_whitespace_and_control() {
        assert!(!is_safe_web_url("https://example.com/ has space"));
        assert!(!is_safe_web_url("https://example.com/\nfoo"));
    }
}
