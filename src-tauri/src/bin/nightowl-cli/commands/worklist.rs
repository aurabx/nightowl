//! `nightowl-cli worklist ...` — mirrors `list_worklist`.

use clap::Subcommand;

use nightowl_lib::core::error::AppError;

use crate::context::Context;
use crate::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// List every Modality Worklist scheduled procedure step entry.
    List,
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::List => list(ctx, format),
    }
}

fn list(ctx: &Context, format: OutputFormat) -> Result<(), AppError> {
    let entries = ctx.worklist.list()?;
    match format {
        OutputFormat::Json => emit_json(&entries),
        OutputFormat::Human => {
            if entries.is_empty() {
                return emit_text("(no worklist entries)");
            }
            // Worklist row shape is large and field-heavy; pretty JSON
            // is more readable than a forced 8-column table.
            emit_json(&entries)
        }
    }
}
