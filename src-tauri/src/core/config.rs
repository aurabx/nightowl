//! Persistent application configuration.
//!
//! The configuration is a single JSON file at `<app config dir>/config.json`.
//! It is read once on app boot and held in Tauri-managed state so other
//! modules (SCP listener, store indexer) can read the current values.
//!
//! This module is deliberately Tauri-free: the load/save functions take
//! `&Path` so they are unit-testable without an `AppHandle`. The Tauri-
//! specific path resolution lives in `lib.rs`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::error::AppError;

/// The persisted application configuration.
///
/// Field naming is snake_case on both sides of the IPC channel for parity
/// with the on-disk JSON file. The frontend's TypeScript interface
/// mirrors these names exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// AE Title this app presents to inbound and outbound DICOM peers.
    /// Constrained by `is_valid_ae_title`.
    pub local_ae_title: String,

    /// TCP port the SCP listener will bind to.
    pub listen_port: u16,

    /// Directory on disk that holds received SOP Instances and is the
    /// source of truth for C-FIND / C-MOVE / C-GET responses.
    pub store_dir: PathBuf,
}

impl AppConfig {
    /// Returns the default configuration anchored at the given home
    /// directory. `store_dir` defaults to `<home>/dicom-store`.
    pub fn default_with_home(home: &Path) -> Self {
        Self {
            local_ae_title: "NIGHTOWL".to_string(),
            listen_port: 11112,
            store_dir: home.join("dicom-store"),
        }
    }
}

/// Returns true if the candidate is a syntactically valid DICOM AE Title
/// under the rules this app enforces.
///
/// Rules: one to sixteen characters, every character in the printable
/// ASCII range (0x20-0x7E), and no leading or trailing whitespace.
///
/// The DICOM standard allows the Default Character Repertoire (G0 set
/// including space). We tighten the rule to printable ASCII because the
/// app's UI cannot meaningfully render the full character repertoire and
/// users typing the title need predictable behaviour.
pub fn is_valid_ae_title(s: &str) -> bool {
    if s.is_empty() || s.len() > 16 {
        return false;
    }
    if s.trim() != s {
        return false;
    }
    s.chars().all(|c| {
        let b = c as u32;
        (0x20..=0x7E).contains(&b)
    })
}

/// Validates an `AppConfig` against the field contracts. Returns the
/// first violation. The caller may show it inline next to the named
/// field.
pub fn validate(cfg: &AppConfig) -> Result<(), AppError> {
    if !is_valid_ae_title(&cfg.local_ae_title) {
        return Err(AppError::validation(
            "local_ae_title",
            "AE Title must be 1-16 printable ASCII characters with no leading or trailing whitespace.",
        ));
    }
    if cfg.listen_port == 0 {
        return Err(AppError::validation(
            "listen_port",
            "Port must be between 1 and 65535.",
        ));
    }
    // Ports below 1024 require root on macOS for `bind`. We refuse rather
    // than silently fail later when the SCP listener tries to bind.
    if cfg.listen_port < 1024 {
        return Err(AppError::validation(
            "listen_port",
            "Ports below 1024 require root privileges on macOS. Choose 1024 or higher (DICOM convention is 11112).",
        ));
    }
    if cfg.store_dir.as_os_str().is_empty() {
        return Err(AppError::validation(
            "store_dir",
            "Store directory path must not be empty.",
        ));
    }
    // Reject relative paths — the SCP listener runs in an unpredictable
    // working directory and must read/write from an absolute location.
    if !cfg.store_dir.is_absolute() {
        return Err(AppError::validation(
            "store_dir",
            "Store directory must be an absolute path.",
        ));
    }
    Ok(())
}

/// Reads the configuration from `config_path`. If the file does not
/// exist, returns `default`. Any other failure (malformed JSON,
/// permission error) propagates as `AppError` — we do NOT silently fall
/// back to the default in that case, because that would mask user data.
pub fn load_or_default(config_path: &Path, default: AppConfig) -> Result<AppConfig, AppError> {
    if !config_path.exists() {
        return Ok(default);
    }
    let bytes = std::fs::read(config_path)?;
    let cfg: AppConfig = serde_json::from_slice(&bytes)?;
    Ok(cfg)
}

/// Validates `cfg`, ensures the config parent directory and the configured
/// `store_dir` both exist, then writes the JSON atomically (write-to-temp
/// + rename) to `config_path`.
pub fn save(config_path: &Path, cfg: &AppConfig) -> Result<(), AppError> {
    validate(cfg)?;

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&cfg.store_dir)?;

    // Atomic write: temp file in the same directory, then rename.
    let tmp = config_path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(cfg)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, config_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_ae_titles() {
        assert!(is_valid_ae_title("NIGHTOWL"));
        assert!(is_valid_ae_title("A"));
        assert!(is_valid_ae_title("CT_SCANNER_01"));
        assert!(is_valid_ae_title("1234567890123456")); // exactly 16
        assert!(is_valid_ae_title("AE WITH SPACE")); // internal spaces ok
    }

    #[test]
    fn invalid_ae_titles() {
        assert!(!is_valid_ae_title("")); // empty
        assert!(!is_valid_ae_title("12345678901234567")); // 17 chars
        assert!(!is_valid_ae_title(" PADDED")); // leading whitespace
        assert!(!is_valid_ae_title("TRAILING ")); // trailing whitespace
        assert!(!is_valid_ae_title("NEW\nLINE")); // control char
        assert!(!is_valid_ae_title("café")); // non-ASCII
        assert!(!is_valid_ae_title("\tTAB")); // leading whitespace (tab)
    }

    #[test]
    fn validate_rejects_zero_port() {
        let mut cfg = AppConfig::default_with_home(Path::new("/tmp"));
        cfg.listen_port = 0;
        assert!(matches!(validate(&cfg), Err(AppError::Validation(_))));
    }

    #[test]
    fn validate_rejects_privileged_port() {
        let mut cfg = AppConfig::default_with_home(Path::new("/tmp"));
        cfg.listen_port = 104; // DICOM registered port, requires root
        assert!(matches!(validate(&cfg), Err(AppError::Validation(_))));
    }

    #[test]
    fn validate_rejects_relative_store_dir() {
        let mut cfg = AppConfig::default_with_home(Path::new("/tmp"));
        cfg.store_dir = PathBuf::from("relative/path");
        assert!(matches!(validate(&cfg), Err(AppError::Validation(_))));
    }

    #[test]
    fn load_or_default_returns_default_when_missing() {
        let tmp = std::env::temp_dir().join("phantom-test-missing-config");
        let _ = std::fs::remove_file(&tmp);
        let default = AppConfig::default_with_home(Path::new("/tmp"));
        let cfg = load_or_default(&tmp, default.clone()).expect("ok");
        assert_eq!(cfg, default);
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp_dir = std::env::temp_dir().join("phantom-test-round-trip");
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let cfg_path = tmp_dir.join("config.json");

        let mut cfg = AppConfig::default_with_home(&tmp_dir);
        cfg.local_ae_title = "ROUNDTRIP".to_string();
        cfg.listen_port = 11113;

        save(&cfg_path, &cfg).expect("save ok");

        let loaded = load_or_default(&cfg_path, AppConfig::default_with_home(&tmp_dir))
            .expect("load ok");
        assert_eq!(loaded, cfg);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
