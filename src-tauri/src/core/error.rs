//! Single error type that crosses the Tauri IPC boundary.
//!
//! `AppError` is `Serialize` so that any `#[tauri::command] -> Result<T,
//! AppError>` rejects with a structured object the frontend can pattern-
//! match on (`kind` + `message`). It is also `thiserror::Error` so it
//! interoperates with `?` and `Box<dyn Error>` in the rest of the backend.

use serde::Serialize;
use thiserror::Error;

/// Validation failure payload exposed to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationDetails {
    /// Logical field name (matches the form input key).
    pub field: String,
    /// Human-readable reason; rendered inline by the frontend.
    pub reason: String,
}

/// All recoverable failures that can be reported to the UI.
///
/// Variants are tagged externally with `kind`, and the variant body lives
/// under `message`. The frontend sees, for example,
/// `{"kind": "Validation", "message": {"field": "local_ae_title", "reason": "..."}}`.
#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    /// Filesystem failure (read, write, mkdir).
    #[error("io error: {0}")]
    Io(String),

    /// JSON encode/decode failure.
    #[error("json error: {0}")]
    Json(String),

    /// A user-facing input did not satisfy the contract for a typed field.
    #[error("validation error: {0:?}")]
    Validation(ValidationDetails),

    /// Failure originating in Tauri itself (path resolution, plugin, etc.).
    #[error("tauri error: {0}")]
    Tauri(String),

    /// Catch-all for unexpected errors that do not fit the above. Use
    /// sparingly — prefer adding a specific variant when a new failure
    /// class appears more than once.
    #[error("internal error: {0}")]
    Internal(String),
}

impl AppError {
    /// Convenience constructor for `Validation` variant.
    pub fn validation(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Validation(ValidationDetails {
            field: field.into(),
            reason: reason.into(),
        })
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        Self::Tauri(e.to_string())
    }
}
