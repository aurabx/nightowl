//! `nightowl-cli inspect ...` — mirrors the `read_dicom_file` tool.

use std::path::PathBuf;

use clap::Subcommand;

use crate::core::error::AppError;
use crate::core::inspect::read_dicom_properties;

use crate::cli::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Read a single DICOM file and print its meta header and every
    /// top-level data element.
    File {
        /// Path to the DICOM Part-10 file.
        path: PathBuf,
    },
}

pub fn run(format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::File { path } => file(format, &path),
    }
}

fn file(format: OutputFormat, path: &std::path::Path) -> Result<(), AppError> {
    let props = read_dicom_properties(path)?;
    match format {
        OutputFormat::Json => emit_json(&props),
        OutputFormat::Human => {
            let mut text = String::new();
            text.push_str(&format!("file:             {}\n", props.file_path));
            text.push_str(&format!("size:             {} bytes\n", props.size_bytes));
            text.push_str(&format!(
                "transfer syntax:  {}\n",
                props.transfer_syntax_uid
            ));
            text.push_str(&format!(
                "SOP class UID:    {}\n",
                props.media_storage_sop_class_uid
            ));
            text.push_str(&format!(
                "SOP instance UID: {}\n",
                props.media_storage_sop_instance_uid
            ));
            text.push_str(&format!("elements:         {}\n\n", props.element_count));
            for el in &props.elements {
                text.push_str(&format!(
                    "{} {:<32} {} {}\n",
                    el.tag, el.name, el.vr, el.value
                ));
            }
            emit_text(&text)
        }
    }
}
