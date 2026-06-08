//! Install / uninstall the `nightowl-cli` command on the user's `$PATH`.
//!
//! The desktop binary doubles as the CLI: when it is invoked under the
//! name `nightowl-cli` (see `src-tauri/src/main.rs`) it dispatches to
//! `crate::cli::run` instead of opening a window. Installing the CLI is
//! therefore just a matter of putting a `nightowl-cli` entry on `$PATH`
//! that resolves to the running desktop binary — no second binary is
//! copied, and nothing the macOS bundler signs is touched.
//!
//! ## Install path per platform
//!
//! - **macOS / Linux**: symlink `/usr/local/bin/nightowl-cli` → the
//!   desktop binary when that directory is writable without elevation,
//!   otherwise `~/.local/bin/nightowl-cli`. One file, no registry
//!   mutation.
//! - **Windows**: not yet supported. The desktop binary is a
//!   windows-subsystem app and cannot write to the invoking shell, so a
//!   Windows CLI needs the separate `nightowl-cli.exe` console binary to
//!   be bundled into the installer and copied onto `%PATH%` — bundling
//!   work that does not exist yet. Rather than install something that
//!   would not work, `status()` reports `"unsupported"` and `install()`
//!   returns an explanatory error.
//!
//! All operations are idempotent: installing twice is a no-op; uninstalling
//! a missing entry is a no-op.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::core::error::AppError;

#[cfg(unix)]
const UNIX_LINK_NAME: &str = "nightowl-cli";

/// Snapshot of the CLI install state, returned to the Command Line page.
#[derive(Debug, Serialize)]
pub struct CliInstallStatus {
    /// Operating system family: `"macos" | "linux" | "windows" | "other"`.
    pub platform: String,
    /// Absolute path of the currently running desktop binary — the file
    /// the `nightowl-cli` entry resolves to once installed.
    pub binary_path: String,
    /// Path where the CLI entry ends up after install. `None` on
    /// unsupported platforms.
    pub install_path: Option<String>,
    /// `"installed"` — the entry exists and points at this binary.
    /// `"stale"` — an entry exists but points somewhere else (e.g. an
    /// older install, or a different tool of the same name).
    /// `"not_installed"` — nothing to find.
    /// `"unsupported"` — this platform has no automatic install path.
    pub status: String,
    /// Hint shown to the user when the install location may need extra
    /// `$PATH` configuration (e.g. `~/.local/bin` is not always on PATH),
    /// or when the platform is unsupported.
    pub path_hint: Option<String>,
}

/// Inspect the install state without making any changes.
pub fn status() -> Result<CliInstallStatus, String> {
    #[cfg(unix)]
    {
        unix::status_impl()
    }
    #[cfg(not(unix))]
    {
        Ok(unsupported_status())
    }
}

/// Install the CLI. Returns the resolved destination so the caller can
/// show it to the user. Safe to re-run — the call is idempotent.
pub fn install() -> Result<String, String> {
    #[cfg(unix)]
    {
        unix::install_impl()
    }
    #[cfg(not(unix))]
    {
        Err(UNSUPPORTED_MESSAGE.to_string())
    }
}

/// Reverse the install. Only removes the entry this app created — it
/// refuses to remove a `nightowl-cli` on `$PATH` that points at some
/// other binary.
pub fn uninstall() -> Result<String, String> {
    #[cfg(unix)]
    {
        unix::uninstall_impl()
    }
    #[cfg(not(unix))]
    {
        Err(UNSUPPORTED_MESSAGE.to_string())
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

#[cfg(not(unix))]
const UNSUPPORTED_MESSAGE: &str =
    "Installing the nightowl-cli command from the app is not yet supported on this platform. \
     Use the standalone nightowl-cli binary instead.";

fn current_binary_path() -> Result<PathBuf, String> {
    // `error::AppError` already wraps `std::io::Error`; reuse its Display
    // so the message shape matches the rest of the backend.
    std::env::current_exe().map_err(|e| AppError::from(e).to_string())
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}

#[cfg(not(unix))]
fn unsupported_status() -> CliInstallStatus {
    let binary_path = current_binary_path().unwrap_or_default();
    CliInstallStatus {
        platform: platform_name().to_string(),
        binary_path,
        install_path: None,
        status: "unsupported".to_string(),
        path_hint: Some(UNSUPPORTED_MESSAGE.to_string()),
    }
}

// ── Unix: symlink to the desktop binary ─────────────────────────────────────

#[cfg(unix)]
mod unix {
    use super::*;

    /// Compare two paths after canonicalising each, falling back to a
    /// literal comparison if canonicalisation fails (e.g. the target was
    /// removed between reading the symlink and the comparison).
    pub(super) fn paths_equal(a: &Path, b: &Path) -> bool {
        let canon_a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
        let canon_b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
        canon_a == canon_b
    }

    pub(super) fn status_impl() -> Result<CliInstallStatus, String> {
        let binary_path = current_binary_path()?;
        let install_path = preferred_install_path()
            .ok_or_else(|| "Could not resolve a home directory for the CLI install".to_string())?;

        let status = if install_path.symlink_metadata().is_ok() {
            match std::fs::read_link(&install_path) {
                Ok(target) if paths_equal(&target, &binary_path) => "installed",
                _ => "stale",
            }
        } else {
            "not_installed"
        };

        Ok(CliInstallStatus {
            platform: platform_name().to_string(),
            binary_path: binary_path.display().to_string(),
            install_path: Some(install_path.display().to_string()),
            status: status.to_string(),
            path_hint: path_hint_for(&install_path),
        })
    }

    pub(super) fn install_impl() -> Result<String, String> {
        let binary_path = current_binary_path()?;
        let install_path = preferred_install_path()
            .ok_or_else(|| "Could not resolve a home directory for the CLI install".to_string())?;

        if let Some(parent) = install_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
            }
        }

        // Remove any existing entry first so reinstall is idempotent and
        // a stale link is replaced cleanly. `symlink_metadata` catches a
        // dangling symlink that `exists()` would miss.
        if install_path.symlink_metadata().is_ok() {
            std::fs::remove_file(&install_path).map_err(|e| {
                format!(
                    "Failed to remove existing {} before reinstall: {e}",
                    install_path.display()
                )
            })?;
        }

        std::os::unix::fs::symlink(&binary_path, &install_path).map_err(|e| {
            format!(
                "Failed to create symlink {} → {}: {e}",
                install_path.display(),
                binary_path.display()
            )
        })?;

        Ok(install_path.display().to_string())
    }

    pub(super) fn uninstall_impl() -> Result<String, String> {
        let binary_path = current_binary_path()?;
        let install_path = preferred_install_path()
            .ok_or_else(|| "Could not resolve a home directory for the CLI install".to_string())?;

        if install_path.symlink_metadata().is_err() {
            return Ok(format!("{} was not installed", install_path.display()));
        }

        match std::fs::read_link(&install_path) {
            Ok(target) if paths_equal(&target, &binary_path) => {
                std::fs::remove_file(&install_path)
                    .map_err(|e| format!("Failed to remove {}: {e}", install_path.display()))?;
                Ok(format!("Removed {}", install_path.display()))
            }
            Ok(target) => Err(format!(
                "Refusing to remove {} — it points to {} (not the NightOwl binary)",
                install_path.display(),
                target.display()
            )),
            Err(_) => Err(format!(
                "Refusing to remove {} — it is not a symlink this app created",
                install_path.display()
            )),
        }
    }

    /// `/usr/local/bin` is preferred only when it exists *and* is writable
    /// without elevation; otherwise fall back to `~/.local/bin`.
    fn preferred_install_path() -> Option<PathBuf> {
        let usr_local = PathBuf::from("/usr/local/bin");
        if usr_local.exists() && dir_is_writable(&usr_local) {
            return Some(usr_local.join(UNIX_LINK_NAME));
        }
        let home = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())?;
        Some(home.join(".local").join("bin").join(UNIX_LINK_NAME))
    }

    /// True when the current process can create a file in `dir`. Probes
    /// with a short-lived temp file; the result is best-effort and may
    /// flip between calls if permissions change.
    fn dir_is_writable(dir: &Path) -> bool {
        let probe = dir.join(format!(".nightowl-cli-write-probe-{}", std::process::id()));
        match std::fs::File::create(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    }

    fn path_hint_for(install_path: &Path) -> Option<String> {
        let parent = install_path.parent()?;
        if parent.ends_with(".local/bin") {
            Some(
                "If `nightowl-cli` is not found in your shell, add `~/.local/bin` to your PATH \
                 (e.g. `export PATH=\"$HOME/.local/bin:$PATH\"` in `~/.zprofile` or `~/.profile`)."
                    .to_string(),
            )
        } else {
            None
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::unix::paths_equal;
    use tempfile::TempDir;

    #[test]
    fn paths_equal_resolves_symlinks_to_the_same_target() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real-binary");
        std::fs::write(&real, b"binary").unwrap();
        let link = tmp.path().join("nightowl-cli");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(paths_equal(&link, &real));
    }

    #[test]
    fn paths_equal_rejects_distinct_targets() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        assert!(!paths_equal(&a, &b));
    }
}
