//! `nightowl-cli instances ...` — mirrors `count_instances`.

use clap::Subcommand;

use crate::core::error::AppError;

use crate::cli::context::Context;
use crate::cli::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Return the total SOP Instance count in the local store index.
    Count,
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::Count => count(ctx, format),
    }
}

fn count(ctx: &Context, format: OutputFormat) -> Result<(), AppError> {
    let n = ctx.index.total_instance_count()?;
    match format {
        OutputFormat::Json => emit_json(&n),
        OutputFormat::Human => emit_text(&n.to_string()),
    }
}
