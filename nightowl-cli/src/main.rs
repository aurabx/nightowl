//! `nightowl-cli` — standalone console front-end for the NightOwl CLI.
//!
//! The command surface itself lives in the library crate
//! (`nightowl_lib::cli`) so that the desktop binary and this shim share
//! one implementation. The desktop binary dispatches into the same
//! `cli::run` when it is invoked under the `nightowl-cli` symlink the
//! Settings → Command Line page installs; this crate exists so that:
//!
//! - Windows has a true console-subsystem binary (the desktop binary is
//!   a windows-subsystem app and cannot write to the invoking shell), and
//! - `cargo run -p nightowl-cli ...` and the `tests/cli.rs` end-to-end
//!   tests have a binary to drive.
//!
//! Keeping this `main` to a single forwarding call is deliberate: there
//! is no behaviour here to drift from the desktop binary's CLI path.

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    std::process::exit(nightowl_lib::cli::run(argv));
}
