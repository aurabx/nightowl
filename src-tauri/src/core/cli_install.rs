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
//! - **Windows**: copy the bundled `nightowl-cli.exe` console binary into
//!   `%LOCALAPPDATA%\Programs\NightOwl\bin\` and prepend that directory to
//!   `HKCU\Environment\Path`. The desktop binary is a windows-subsystem
//!   app and cannot write to the invoking shell, so Windows needs the
//!   separate console binary; it ships next to the desktop binary via the
//!   `externalBin` entry in `tauri.windows.conf.json`. No admin rights and
//!   no symlinks (which need developer mode on Windows) are required.
//!
//! All operations are idempotent: installing twice is a no-op; uninstalling
//! a missing entry is a no-op.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::core::error::AppError;

#[cfg(unix)]
const UNIX_LINK_NAME: &str = "nightowl-cli";

// Per-user install directory name under `%LOCALAPPDATA%\Programs\`, the
// installed command name, and the bundled source binary's name. Source
// and installed names match — the `externalBin` sidecar lands next to the
// desktop binary as `nightowl-cli.exe` (Tauri strips the target-triple
// suffix), and that is exactly the command we want on `$PATH`.
#[cfg(windows)]
const WINDOWS_BIN_DIR_NAME: &str = "NightOwl";
#[cfg(windows)]
const WINDOWS_CLI_EXE_NAME: &str = "nightowl-cli.exe";
#[cfg(windows)]
const WINDOWS_SOURCE_CLI_NAME: &str = "nightowl-cli.exe";

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
    #[cfg(windows)]
    {
        windows::status_impl()
    }
    #[cfg(not(any(unix, windows)))]
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
    #[cfg(windows)]
    {
        windows::install_impl()
    }
    #[cfg(not(any(unix, windows)))]
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
    #[cfg(windows)]
    {
        windows::uninstall_impl()
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(UNSUPPORTED_MESSAGE.to_string())
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

#[cfg(not(any(unix, windows)))]
const UNSUPPORTED_MESSAGE: &str =
    "Installing the nightowl-cli command from the app is not supported on this platform. \
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

#[cfg(not(any(unix, windows)))]
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

// ── Windows: copy the bundled CLI binary + per-user PATH update ──────────────

#[cfg(windows)]
mod windows {
    use super::*;

    pub(super) fn status_impl() -> Result<CliInstallStatus, String> {
        let binary_path = current_binary_path()?;
        let bin_dir = bin_dir()?;
        let install_path = bin_dir.join(WINDOWS_CLI_EXE_NAME);

        let file_present = install_path.is_file();
        let on_path = user_path_contains(&bin_dir).unwrap_or(false);
        let status = match (file_present, on_path) {
            (true, true) => "installed",
            (false, false) => "not_installed",
            // Half-installed (file but no PATH entry, or vice versa).
            // Surfacing it lets the user reinstall to repair without us
            // silently mutating either side.
            _ => "stale",
        };

        let path_hint = if status == "installed" {
            Some(
                "Open a new terminal to pick up the updated PATH — shells that were \
                 already running use the value they started with."
                    .to_string(),
            )
        } else {
            None
        };

        Ok(CliInstallStatus {
            platform: platform_name().to_string(),
            binary_path: binary_path.display().to_string(),
            install_path: Some(install_path.display().to_string()),
            status: status.to_string(),
            path_hint,
        })
    }

    pub(super) fn install_impl() -> Result<String, String> {
        let bin_dir = bin_dir()?;
        let install_path = bin_dir.join(WINDOWS_CLI_EXE_NAME);
        let source = source_cli_binary()?;

        std::fs::create_dir_all(&bin_dir)
            .map_err(|e| format!("Failed to create {}: {e}", bin_dir.display()))?;

        // Overwrite is fine — the source is the binary that shipped with
        // this install of NightOwl, so reinstalling refreshes it.
        std::fs::copy(&source, &install_path).map_err(|e| {
            format!(
                "Failed to copy {} → {}: {e}",
                source.display(),
                install_path.display()
            )
        })?;

        add_to_user_path(&bin_dir)?;
        Ok(install_path.display().to_string())
    }

    pub(super) fn uninstall_impl() -> Result<String, String> {
        let bin_dir = bin_dir()?;
        let install_path = bin_dir.join(WINDOWS_CLI_EXE_NAME);

        let mut steps: Vec<String> = Vec::new();
        if install_path.exists() {
            std::fs::remove_file(&install_path)
                .map_err(|e| format!("Failed to remove {}: {e}", install_path.display()))?;
            steps.push(format!("Removed {}", install_path.display()));
        }

        if user_path_contains(&bin_dir).unwrap_or(false) {
            remove_from_user_path(&bin_dir)?;
            steps.push(format!("Removed {} from PATH", bin_dir.display()));
        }

        // Best-effort: drop the now-empty bin dir. Ignore failure (e.g. it
        // still holds something the user put there).
        let _ = std::fs::remove_dir(&bin_dir);

        if steps.is_empty() {
            Ok(format!("{} was not installed", install_path.display()))
        } else {
            Ok(steps.join("; "))
        }
    }

    /// Per-user install root: `%LOCALAPPDATA%\Programs\NightOwl\bin\`.
    ///
    /// `%LOCALAPPDATA%` is writable by the current user without admin and
    /// is the standard location for per-user app installs on Windows.
    fn bin_dir() -> Result<PathBuf, String> {
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "%LOCALAPPDATA% is not set".to_string())?;
        Ok(local
            .join("Programs")
            .join(WINDOWS_BIN_DIR_NAME)
            .join("bin"))
    }

    /// Locate the bundled `nightowl-cli.exe`. The `externalBin` entry in
    /// `tauri.windows.conf.json` makes Tauri place it next to the desktop
    /// binary, so the source is `<dir-of-current_exe>\nightowl-cli.exe`.
    /// During `cargo run` of the desktop binary the workspace build emits
    /// both binaries side by side in `target\<profile>\`, so the same
    /// lookup works without any bundle wiring.
    fn source_cli_binary() -> Result<PathBuf, String> {
        let exe = current_binary_path()?;
        let dir = exe
            .parent()
            .ok_or_else(|| "Failed to resolve the install directory".to_string())?;
        let cli = dir.join(WINDOWS_SOURCE_CLI_NAME);
        if !cli.exists() {
            return Err(format!(
                "CLI binary not found at {} — was NightOwl installed with the CLI bundle?",
                cli.display()
            ));
        }
        Ok(cli)
    }

    /// Read `HKCU\Environment\Path` and return true if `dir` is present in
    /// any of the `;`-separated entries (case-insensitive, canonicalised).
    fn user_path_contains(dir: &Path) -> Result<bool, String> {
        let target = canonical_lossy(dir);
        Ok(read_user_path_entries()?
            .into_iter()
            .any(|entry| canonical_lossy(&PathBuf::from(entry)) == target))
    }

    fn add_to_user_path(dir: &Path) -> Result<(), String> {
        let mut entries = read_user_path_entries()?;
        let target = canonical_lossy(dir);
        if entries
            .iter()
            .any(|entry| canonical_lossy(&PathBuf::from(entry)) == target)
        {
            return Ok(());
        }
        entries.insert(0, dir.display().to_string());
        write_user_path_entries(&entries)?;
        broadcast_environment_change();
        Ok(())
    }

    fn remove_from_user_path(dir: &Path) -> Result<(), String> {
        let target = canonical_lossy(dir);
        let mut changed = false;
        let kept: Vec<String> = read_user_path_entries()?
            .into_iter()
            .filter(|entry| {
                let drop = canonical_lossy(&PathBuf::from(entry)) == target;
                changed |= drop;
                !drop
            })
            .collect();
        if !changed {
            return Ok(());
        }
        write_user_path_entries(&kept)?;
        broadcast_environment_change();
        Ok(())
    }

    fn read_user_path_entries() -> Result<Vec<String>, String> {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
        use winreg::RegKey;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let env = hkcu
            .open_subkey_with_flags("Environment", KEY_READ)
            .map_err(|e| format!("Failed to open HKCU\\Environment: {e}"))?;
        // A user with no `Path` value yet is normal, not an error.
        let raw: String = env.get_value("Path").unwrap_or_default();
        Ok(raw
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    fn write_user_path_entries(entries: &[String]) -> Result<(), String> {
        use winreg::RegKey;
        let hkcu = RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let (env, _) = hkcu
            .create_subkey("Environment")
            .map_err(|e| format!("Failed to open HKCU\\Environment: {e}"))?;
        env.set_value("Path", &entries.join(";"))
            .map_err(|e| format!("Failed to write HKCU\\Environment\\Path: {e}"))
    }

    /// Broadcast `WM_SETTINGCHANGE("Environment")` so already-running
    /// shells and Explorer pick up the new PATH without a logout. Failures
    /// are silent — the registry write is the source of truth; the
    /// broadcast is only a courtesy.
    fn broadcast_environment_change() {
        use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        };
        let param: Vec<u16> = "Environment\0".encode_utf16().collect();
        let mut result: usize = 0;
        // Safety: `param` is a valid null-terminated UTF-16 buffer that
        // outlives the call; `result` is a valid out-pointer.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0 as WPARAM,
                param.as_ptr() as LPARAM,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result as *mut usize,
            );
        }
    }

    /// Lower-cased, trailing-separator-trimmed form used for the
    /// case-insensitive PATH comparison. Windows paths are
    /// case-insensitive, but the registry preserves the user's casing, so
    /// we normalise for comparison without rewriting the stored entry.
    fn canonical_lossy(path: &Path) -> String {
        path.to_string_lossy().trim_end_matches('\\').to_lowercase()
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
