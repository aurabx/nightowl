//! Phantom Tauri library entrypoint.
//!
//! Per the project's `CLAUDE.md`: business logic lives in `core.rs`; the
//! `#[tauri::command]` functions in this file are thin wrappers that call
//! into `core` and return values across the IPC boundary.

mod core;

/// IPC self-check command used by the frontend on mount.
///
/// Returns the literal string `"pong"`. If the frontend sees anything else,
/// the IPC channel is broken.
#[tauri::command]
fn ping() -> &'static str {
    core::ping()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ping])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
