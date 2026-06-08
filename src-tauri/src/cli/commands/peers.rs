//! `nightowl-cli peers ...` — mirrors the MCP `list_peers` tool.

use clap::Subcommand;

use crate::core::error::AppError;

use crate::cli::context::Context;
use crate::cli::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// List every configured remote DICOM peer.
    List,
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::List => list(ctx, format),
    }
}

fn list(ctx: &Context, format: OutputFormat) -> Result<(), AppError> {
    let peers = ctx.peers.list()?;
    match format {
        OutputFormat::Json => emit_json(&peers),
        OutputFormat::Human => {
            if peers.is_empty() {
                return emit_text("(no peers configured)");
            }
            let mut out = String::new();
            out.push_str("ID                                    NAME                  AE TITLE         HOST:PORT\n");
            for p in &peers {
                out.push_str(&format!(
                    "{:<37} {:<21} {:<16} {}:{}\n",
                    p.id, p.name, p.ae_title, p.host, p.port
                ));
            }
            emit_text(&out)
        }
    }
}
