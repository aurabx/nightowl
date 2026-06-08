//! `nightowl-cli config ...` — mirrors the MCP `get_config` tool.

use clap::Subcommand;

use crate::core::error::AppError;

use crate::cli::context::Context;
use crate::cli::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Print the loaded NightOwl configuration.
    Show,
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::Show => show(ctx, format),
    }
}

fn show(ctx: &Context, format: OutputFormat) -> Result<(), AppError> {
    match format {
        OutputFormat::Json => emit_json(&ctx.config),
        OutputFormat::Human => {
            let cfg = &ctx.config;
            let text = format!(
                "data_dir:       {}\n\
                 config_path:    {}\n\
                 local_ae_title: {}\n\
                 listen_port:    {}\n\
                 store_dir:      {}\n\
                 mcp.enabled:    {}\n\
                 mcp.port:       {}\n",
                ctx.data_dir.display(),
                ctx.paths.config.display(),
                cfg.local_ae_title,
                cfg.listen_port,
                cfg.store_dir.display(),
                cfg.mcp.enabled,
                cfg.mcp.port,
            );
            emit_text(&text)
        }
    }
}
