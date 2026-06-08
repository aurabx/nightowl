//! `nightowl-cli studies ...` — mirrors `list_studies` and
//! `list_series_for_study`.

use clap::Subcommand;

use nightowl_lib::core::error::AppError;

use crate::context::Context;
use crate::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// List every study in the local SOP Instance index.
    List,
    /// List the series under a given Study Instance UID.
    Series {
        /// Study Instance UID to expand.
        study_uid: String,
    },
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::List => list(ctx, format),
        Action::Series { study_uid } => series(ctx, format, &study_uid),
    }
}

fn list(ctx: &Context, format: OutputFormat) -> Result<(), AppError> {
    let studies = ctx.index.list_studies()?;
    match format {
        OutputFormat::Json => emit_json(&studies),
        OutputFormat::Human => {
            if studies.is_empty() {
                return emit_text("(no studies in store)");
            }
            let mut out = String::new();
            out.push_str("STUDY INSTANCE UID                          DATE       MOD    SER  INST  PATIENT\n");
            for s in &studies {
                out.push_str(&format!(
                    "{:<43} {:<10} {:<6} {:>3}  {:>4}  {} ({})\n",
                    truncate(&s.study_instance_uid, 43),
                    s.study_date.as_deref().unwrap_or("-"),
                    s.modalities.as_deref().unwrap_or("-"),
                    s.series_count,
                    s.instance_count,
                    s.patient_name.as_deref().unwrap_or("-"),
                    s.patient_id,
                ));
            }
            emit_text(&out)
        }
    }
}

fn series(ctx: &Context, format: OutputFormat, study_uid: &str) -> Result<(), AppError> {
    let series = ctx.index.list_series_for_study(study_uid)?;
    match format {
        OutputFormat::Json => emit_json(&series),
        OutputFormat::Human => {
            if series.is_empty() {
                return emit_text("(no series for that study)");
            }
            let mut out = String::new();
            out.push_str("SERIES INSTANCE UID                         MOD   INST  DESCRIPTION\n");
            for s in &series {
                out.push_str(&format!(
                    "{:<43} {:<5} {:>4}  {}\n",
                    truncate(&s.series_instance_uid, 43),
                    s.modality.as_deref().unwrap_or("-"),
                    s.instance_count,
                    s.series_description.as_deref().unwrap_or(""),
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
