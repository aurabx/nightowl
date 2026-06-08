//! `nightowl-cli store ...` — mirrors `rescan_store`.

use clap::Subcommand;

use crate::core::error::AppError;

use crate::cli::context::Context;
use crate::cli::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Walk the configured store directory and re-ingest every file.
    Rescan,
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::Rescan => rescan(ctx, format),
    }
}

fn rescan(ctx: &Context, format: OutputFormat) -> Result<(), AppError> {
    let report = ctx.index.rescan_dir(&ctx.config.store_dir)?;
    match format {
        OutputFormat::Json => emit_json(&report),
        OutputFormat::Human => {
            let text = format!(
                "scanned:  {}\n\
                 inserted: {}\n\
                 updated:  {}\n\
                 skipped:  {}\n\
                 errored:  {}\n\
                 elapsed:  {} ms\n",
                report.files_seen,
                report.files_inserted,
                report.files_updated,
                report.files_skipped,
                report.files_errored,
                report.elapsed_ms,
            );
            emit_text(&text)
        }
    }
}
