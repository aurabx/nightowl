//! Phantom Tauri library entrypoint.
//!
//! Per the project's `CLAUDE.md`: business logic lives in `core`; the
//! `#[tauri::command]` functions in this file are thin wrappers that
//! resolve Tauri-specific values (paths, managed state) and call into
//! `core`.

mod core;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};

use core::config::{AppConfig, load_or_default, save};
use core::error::AppError;

/// Process-wide state shared across Tauri commands.
///
/// At M1 this holds only the configuration. Later milestones will add the
/// SQLite index handle, the SCP listener handle, the peers store, etc.
struct AppState {
    config: Mutex<AppConfig>,
}

/// Returns the path to `config.json` inside the platform-specific app
/// config directory. Tauri resolves `app_config_dir` based on the
/// `identifier` in `tauri.conf.json` (here: `cloud.aurabox.phantom`),
/// which on macOS is `~/Library/Application Support/cloud.aurabox.phantom`.
fn config_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Tauri(e.to_string()))?;
    Ok(dir.join("config.json"))
}

/// Resolves the user's home directory through Tauri so the same logic
/// works on every platform.
fn home_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    app.path()
        .home_dir()
        .map_err(|e| AppError::Tauri(e.to_string()))
}

/// Locks the config mutex and clones the current value. We hold the
/// lock only long enough to copy out, never across an `await`. Mutex
/// poisoning is reported as an `Internal` error rather than panicking.
fn read_config(state: &AppState) -> Result<AppConfig, AppError> {
    state
        .config
        .lock()
        .map(|guard| guard.clone())
        .map_err(|_| AppError::Internal("config mutex poisoned".to_string()))
}

#[tauri::command]
fn ping() -> &'static str {
    core::ping()
}

/// Returns the currently loaded configuration.
#[tauri::command]
fn get_config(state: State<'_, AppState>) -> Result<AppConfig, AppError> {
    read_config(&state)
}

/// Validates `cfg`, writes it to disk, and updates the in-memory copy on
/// success. The frontend should re-render with the new values after this
/// command resolves; we also return the saved config for symmetry with
/// `get_config`.
#[tauri::command]
fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    cfg: AppConfig,
) -> Result<AppConfig, AppError> {
    let path = config_path(&app)?;
    save(&path, &cfg)?;
    let mut guard = state
        .config
        .lock()
        .map_err(|_| AppError::Internal("config mutex poisoned".to_string()))?;
    *guard = cfg.clone();
    Ok(cfg)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle();
            let cfg_path = config_path(handle)?;

            // Ensure the config directory exists so a first-time launch
            // does not blow up writing the default config later.
            if let Some(parent) = cfg_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let home = home_dir(handle)?;
            let default = AppConfig::default_with_home(&home);
            let cfg = load_or_default(&cfg_path, default)?;

            // Ensure the store directory exists before any DICOM module
            // tries to read or write into it.
            std::fs::create_dir_all(&cfg.store_dir)?;

            app.manage(AppState {
                config: Mutex::new(cfg),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![ping, get_config, save_config])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
