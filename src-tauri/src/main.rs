// Prevent additional console window on Windows in release. macOS only build
// for this iteration, but we keep the attribute for parity with the Tauri
// template.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // The desktop binary doubles as the CLI. There are two ways an
    // invocation asks for CLI behaviour:
    //
    //  1. The binary was invoked under the `nightowl-cli` name. The
    //     Settings → Command Line page installs a symlink
    //     (`/usr/local/bin/nightowl-cli` → this binary), so `argv[0]`
    //     carries that name even though `current_exe()` would resolve it
    //     back to the real binary inside the `.app`. Keying off `argv[0]`
    //     is what lets the symlink name decide, which also makes
    //     `nightowl-cli --help` and a bare `nightowl-cli` behave as a CLI
    //     (clap prints help) rather than launching a window.
    //  2. The binary was invoked under its own name but the first
    //     argument is a known CLI verb (e.g. `nightowl scu echo ...`).
    //
    // Anything else — including a bare double-click launch — starts the
    // Tauri desktop app.
    //
    // On Windows the desktop binary is a windows-subsystem app (no
    // attached console), so Windows CLI usage goes through the separate
    // `nightowl-cli.exe` console shim rather than this symlink path; the
    // dispatch below still works but stdout would not reach the parent
    // shell. That is why the Command Line install page reports Windows as
    // not-yet-supported.
    let invoked_as_cli = Path::new(&args[0])
        .file_name()
        .map(|name| name.to_string_lossy().starts_with("nightowl-cli"))
        .unwrap_or(false);

    if invoked_as_cli
        || args
            .get(1)
            .is_some_and(|verb| nightowl_lib::cli::is_cli_verb(verb))
    {
        std::process::exit(nightowl_lib::cli::run(args));
    }

    nightowl_lib::run();
}
