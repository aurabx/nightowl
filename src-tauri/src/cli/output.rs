//! Output formatting helpers.
//!
//! Every command body produces a typed result, then hands it here to
//! be rendered as either pretty JSON (mirrors what the MCP server
//! returns) or as a short human-readable summary.

use std::io::{self, Write};

use crate::core::error::AppError;
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    Human,
    Json,
}

/// Writes `value` to stdout as pretty JSON. Same encoder the MCP
/// server uses so identical inputs produce byte-identical output.
pub fn emit_json<T: Serialize>(value: &T) -> Result<(), AppError> {
    let text = serde_json::to_string_pretty(value)?;
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.write_all(b"\n")?;
    Ok(())
}

/// Writes `text` to stdout verbatim, appending a trailing newline if
/// the body does not already end with one.
pub fn emit_text(text: &str) -> Result<(), AppError> {
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())?;
    if !text.ends_with('\n') {
        out.write_all(b"\n")?;
    }
    Ok(())
}
