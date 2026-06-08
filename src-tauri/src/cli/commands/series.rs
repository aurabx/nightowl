//! `nightowl-cli series ...` — mirrors `list_instances_for_series`.

use clap::Subcommand;

use crate::core::error::AppError;

use crate::cli::context::Context;
use crate::cli::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// List every SOP Instance under a given Series Instance UID.
    Instances {
        /// Series Instance UID to expand.
        series_uid: String,
    },
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::Instances { series_uid } => instances(ctx, format, &series_uid),
    }
}

fn instances(ctx: &Context, format: OutputFormat, series_uid: &str) -> Result<(), AppError> {
    let rows = ctx.index.list_instances_for_series(series_uid)?;
    match format {
        OutputFormat::Json => emit_json(&rows),
        OutputFormat::Human => {
            if rows.is_empty() {
                return emit_text("(no instances for that series)");
            }
            let mut out = String::new();
            out.push_str("SOP INSTANCE UID                            BYTES        FILE\n");
            for r in &rows {
                out.push_str(&format!(
                    "{:<43} {:>10}  {}\n",
                    truncate(&r.sop_instance_uid, 43),
                    r.size_bytes,
                    r.file_path,
                ));
            }
            emit_text(&out)
        }
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}
