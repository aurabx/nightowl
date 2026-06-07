//! Filesystem paths for NightOwl's persistent state.
//!
//! Two callers need to resolve the same files: the desktop binary
//! (through Tauri's `AppHandle::path()`) and the CLI binary (with no
//! Tauri runtime available). Centralising the filename joins and the
//! platform-default directory lookup here keeps them in lockstep.

use std::path::{Path, PathBuf};

use directories::BaseDirs;

use super::error::AppError;

/// Bundle identifier from `tauri.conf.json`. Repeated as a const so the
/// CLI binary and the early-startup breadcrumb code in `lib.rs` can
/// resolve the data directory without going through Tauri's path APIs.
/// The unit test below guards against drift between this value and the
/// one in `tauri.conf.json`.
pub const BUNDLE_ID: &str = "cloud.aurabox.nightowl";

/// Absolute paths to every persistent file the app reads or writes
/// inside the platform's app config directory.
#[derive(Debug, Clone)]
pub struct DataPaths {
    pub config: PathBuf,
    pub index: PathBuf,
    pub peers: PathBuf,
    pub worklist: PathBuf,
}

/// Joins the four well-known filenames onto `dir`. Pure — does no IO.
///
/// `dir` is the platform's app config directory for this bundle (the
/// same path Tauri's `AppHandle::path().app_config_dir()` returns).
pub fn data_paths_from(dir: &Path) -> DataPaths {
    DataPaths {
        config: dir.join("config.json"),
        index: dir.join("store.sqlite"),
        peers: dir.join("peers.json"),
        worklist: dir.join("worklist.sqlite"),
    }
}

/// Resolves the platform-specific app config directory for this bundle.
///
/// Mirrors Tauri's `AppHandle::path().app_config_dir()` so the CLI
/// binary and the desktop app see the same files on disk:
///
/// - macOS: `~/Library/Application Support/cloud.aurabox.nightowl/`
/// - Linux: `$XDG_CONFIG_HOME/cloud.aurabox.nightowl/`
///   (typically `~/.config/cloud.aurabox.nightowl/`)
/// - Windows: `%APPDATA%\cloud.aurabox.nightowl\`
pub fn default_data_dir() -> Result<PathBuf, AppError> {
    let base = BaseDirs::new()
        .ok_or_else(|| AppError::Io("no home directory available".to_string()))?;
    Ok(base.config_dir().join(BUNDLE_ID))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bundle id appears both here and in `tauri.conf.json`. If they
    /// drift, the CLI would read a different data directory from the
    /// desktop app on the same machine. This test fails the build when
    /// only one side is changed.
    #[test]
    fn bundle_id_matches_tauri_conf() {
        let conf = include_str!("../../tauri.conf.json");
        let parsed: serde_json::Value =
            serde_json::from_str(conf).expect("parse tauri.conf.json");
        let identifier = parsed["identifier"]
            .as_str()
            .expect("tauri.conf.json missing string `identifier`");
        assert_eq!(BUNDLE_ID, identifier);
    }

    #[test]
    fn data_paths_appends_known_filenames() {
        let dir = Path::new("/tmp/nightowl-test");
        let p = data_paths_from(dir);
        assert_eq!(p.config, dir.join("config.json"));
        assert_eq!(p.index, dir.join("store.sqlite"));
        assert_eq!(p.peers, dir.join("peers.json"));
        assert_eq!(p.worklist, dir.join("worklist.sqlite"));
    }
}
