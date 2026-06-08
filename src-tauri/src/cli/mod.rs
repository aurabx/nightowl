//! Command-line surface, mirroring the MCP tool set.
//!
//! This module lives in the library crate so that **both** entrypoints can
//! drive it from one source of truth:
//!
//! - the desktop binary (`src-tauri/src/main.rs`), which dispatches here
//!   when it is invoked as a CLI (e.g. via the `nightowl-cli` symlink the
//!   Settings → Command Line page installs), and
//! - the standalone `nightowl-cli` shim crate, which exists for the
//!   Windows console-subsystem case and for `cargo run -p nightowl-cli`.
//!
//! Keeping the surface here means the symlinked desktop binary and the
//! shim binary expose byte-identical behaviour — there is no second
//! implementation to drift.
//!
//! The CLI opens the same SQLite stores and `config.json` the desktop app
//! uses (SQLite is in WAL mode so concurrent access is safe) and invokes
//! the same `core` functions the MCP server does. There is no IPC step.
//!
//! Global flags:
//!
//! - `--data-dir <path>` overrides the platform-default app config
//!   directory. Useful for tests and for running against an alternate
//!   data set.
//! - `--json` emits each command's result as pretty-printed JSON, the
//!   same shape the MCP server returns.
//! - `-v` / `--verbose` raises the tracing filter from the default
//!   (errors only) to info; `-vv` to debug.
//!
//! Exit codes:
//!
//! - `0` success
//! - `1` runtime failure (IO, database, DICOM, etc.)
//! - `2` validation failure (bad arguments, unknown peer id, etc.)

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::core::error::AppError;

mod commands;
mod context;
mod output;

use context::Context;
use output::OutputFormat;

#[derive(Parser, Debug)]
#[command(
    name = "nightowl-cli",
    version,
    about = "Command-line driver for NightOwl, mirroring the MCP tool surface."
)]
struct Cli {
    /// Override the data directory. Defaults to the platform's app
    /// config directory for the NightOwl bundle (the same path the
    /// desktop app uses).
    #[arg(long, global = true, value_name = "PATH")]
    data_dir: Option<PathBuf>,

    /// Emit results as pretty-printed JSON instead of human text.
    #[arg(long, global = true)]
    json: bool,

    /// Increase log verbosity. `-v` for info, `-vv` for debug.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: TopLevel,
}

#[derive(Subcommand, Debug)]
enum TopLevel {
    /// Read the effective NightOwl configuration.
    Config {
        #[command(subcommand)]
        action: commands::config::Action,
    },
    /// List configured remote DICOM peers.
    Peers {
        #[command(subcommand)]
        action: commands::peers::Action,
    },
    /// Browse the local SOP Instance index.
    Studies {
        #[command(subcommand)]
        action: commands::studies::Action,
    },
    /// Browse series contents.
    Series {
        #[command(subcommand)]
        action: commands::series::Action,
    },
    /// Operations on SOP Instances.
    Instances {
        #[command(subcommand)]
        action: commands::instances::Action,
    },
    /// Local store maintenance.
    Store {
        #[command(subcommand)]
        action: commands::store::Action,
    },
    /// List Modality Worklist (DMWL) scheduled procedure steps.
    Worklist {
        #[command(subcommand)]
        action: commands::worklist::Action,
    },
    /// Inspect the persistent activity log.
    Activity {
        #[command(subcommand)]
        action: commands::activity::Action,
    },
    /// Send a DIMSE message to a remote peer (echo / find / move / store).
    Scu {
        #[command(subcommand)]
        action: commands::scu::Action,
    },
    /// Inspect a DICOM file's properties without touching the store.
    Inspect {
        #[command(subcommand)]
        action: commands::inspect::Action,
    },
}

/// Top-level verbs the desktop binary recognises as "run me as a CLI".
///
/// The desktop binary (`main.rs`) calls this to decide whether an
/// invocation is a CLI request or a request to launch the GUI. The
/// `nightowl-cli` shim does not need it — that binary is CLI-only and
/// always dispatches here.
pub fn is_cli_verb(arg: &str) -> bool {
    matches!(
        arg,
        "config"
            | "peers"
            | "studies"
            | "series"
            | "instances"
            | "store"
            | "worklist"
            | "activity"
            | "scu"
            | "inspect"
    )
}

/// Parse `argv` (including the program name at index 0) and run the
/// requested command. Returns the process exit code so the caller — the
/// desktop binary or the shim — can `std::process::exit` with it.
///
/// `clap` errors (including `--help` / `--version`, which clap models as
/// errors with exit code 0) are printed here and surfaced via their own
/// exit code rather than panicking.
pub fn run(argv: Vec<String>) -> i32 {
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(err) => {
            // Prints help/version to stdout and usage errors to stderr,
            // matching clap's own `Error::exit` behaviour.
            let _ = err.print();
            return err.exit_code();
        }
    };

    init_tracing(cli.verbose);

    let format = if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Human
    };

    match dispatch(cli, format) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            i32::from(exit_code_for(&err))
        }
    }
}

fn dispatch(cli: Cli, format: OutputFormat) -> Result<(), AppError> {
    // `inspect` reads an arbitrary file path and never touches the
    // persistent stores, so it is dispatched before `Context::open`
    // to avoid creating a data directory the command does not need.
    if let TopLevel::Inspect { action } = cli.command {
        return commands::inspect::run(format, action);
    }

    let ctx = Context::open(cli.data_dir.as_deref())?;
    match cli.command {
        TopLevel::Config { action } => commands::config::run(&ctx, format, action),
        TopLevel::Peers { action } => commands::peers::run(&ctx, format, action),
        TopLevel::Studies { action } => commands::studies::run(&ctx, format, action),
        TopLevel::Series { action } => commands::series::run(&ctx, format, action),
        TopLevel::Instances { action } => commands::instances::run(&ctx, format, action),
        TopLevel::Store { action } => commands::store::run(&ctx, format, action),
        TopLevel::Worklist { action } => commands::worklist::run(&ctx, format, action),
        TopLevel::Activity { action } => commands::activity::run(&ctx, format, action),
        TopLevel::Scu { action } => commands::scu::run(&ctx, format, action),
        // Routed before `Context::open` above.
        TopLevel::Inspect { .. } => unreachable!("inspect is dispatched before Context::open"),
    }
}

fn init_tracing(verbosity: u8) {
    let filter = match verbosity {
        0 => "error",
        1 => "info",
        _ => "debug",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| filter.parse().unwrap()),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

fn exit_code_for(err: &AppError) -> u8 {
    match err {
        AppError::Validation(_) => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn clap_definition_is_valid() {
        // `Cli::command().debug_assert()` runs clap's internal sanity
        // checks (no clashing flag names, every required field has a
        // type clap can parse, etc.). Catches structural mistakes at
        // build time rather than at first `--help` invocation.
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_config_show_with_data_dir() {
        let cli = Cli::try_parse_from(["nightowl-cli", "--data-dir", "/tmp/x", "config", "show"])
            .expect("parse");
        assert_eq!(
            cli.data_dir.as_deref(),
            Some(std::path::Path::new("/tmp/x"))
        );
        assert!(!cli.json);
        assert!(matches!(cli.command, TopLevel::Config { .. }));
    }

    #[test]
    fn parses_json_flag_in_any_position() {
        // `--json` is a global flag, so it should parse whether it
        // appears before or after the subcommand.
        let before =
            Cli::try_parse_from(["nightowl-cli", "--json", "instances", "count"]).expect("before");
        let after =
            Cli::try_parse_from(["nightowl-cli", "instances", "count", "--json"]).expect("after");
        assert!(before.json);
        assert!(after.json);
    }

    #[test]
    fn missing_required_subcommand_is_rejected() {
        // Bare invocation should fail rather than do nothing — the
        // user always gets a help dump. Clap reports this as a
        // help-display rather than a hard missing-arg error.
        let err = Cli::try_parse_from(["nightowl-cli"]).expect_err("must require a subcommand");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
    }

    #[test]
    fn exit_code_validation_is_two() {
        let err = AppError::validation("peer_id", "unknown");
        assert_eq!(exit_code_for(&err), 2);
    }

    #[test]
    fn exit_code_non_validation_is_one() {
        assert_eq!(exit_code_for(&AppError::Io("disk full".into())), 1);
        assert_eq!(exit_code_for(&AppError::Internal("oops".into())), 1);
        assert_eq!(exit_code_for(&AppError::Database("locked".into())), 1);
    }

    #[test]
    fn is_cli_verb_matches_known_top_level_commands() {
        for verb in [
            "config",
            "peers",
            "studies",
            "series",
            "instances",
            "store",
            "worklist",
            "activity",
            "scu",
            "inspect",
        ] {
            assert!(is_cli_verb(verb), "{verb} should be a CLI verb");
        }
        for non_verb in ["--help", "--version", "gui", "", "Config"] {
            assert!(
                !is_cli_verb(non_verb),
                "{non_verb:?} should not be a CLI verb"
            );
        }
    }
}
