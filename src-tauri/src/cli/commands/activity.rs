//! `nightowl-cli activity ...` — mirrors `list_activity` and
//! `count_activity`.

use clap::{Args, Subcommand};

use crate::core::activity::ActivityFilter;
use crate::core::error::AppError;

use crate::cli::context::Context;
use crate::cli::output::{emit_json, emit_text, OutputFormat};

#[derive(Subcommand, Debug)]
pub enum Action {
    /// Return persisted activity log entries, newest first.
    List(ListFlags),
    /// Return the total count of persisted activity log entries.
    Count,
}

#[derive(Args, Debug)]
pub struct ListFlags {
    /// Filter by direction (in / out / info).
    #[arg(long)]
    pub direction: Option<String>,
    /// Filter by status (info / success / warning / error).
    #[arg(long)]
    pub status: Option<String>,
    /// Filter by peer AE Title.
    #[arg(long)]
    pub peer_ae: Option<String>,
    /// Filter by DIMSE command name (e.g. "C-ECHO-RQ").
    #[arg(long)]
    pub command: Option<String>,
    /// Filter by association id.
    #[arg(long)]
    pub association_id: Option<String>,
    /// Free-text search over the message field.
    #[arg(long)]
    pub search: Option<String>,
    /// Only return events with timestamp_ms >= this value.
    #[arg(long)]
    pub since_ms: Option<i64>,
    /// Page size (capped server-side at 5000).
    #[arg(long)]
    pub limit: Option<i64>,
    /// Zero-based offset into the (newest-first) result set.
    #[arg(long)]
    pub offset: Option<i64>,
}

impl From<ListFlags> for ActivityFilter {
    fn from(f: ListFlags) -> Self {
        Self {
            direction: f.direction,
            status: f.status,
            peer_ae_title: f.peer_ae,
            command: f.command,
            association_id: f.association_id,
            search: f.search,
            since_ms: f.since_ms,
            limit: f.limit,
            offset: f.offset,
        }
    }
}

pub fn run(ctx: &Context, format: OutputFormat, action: Action) -> Result<(), AppError> {
    match action {
        Action::List(flags) => list(ctx, format, flags.into()),
        Action::Count => count(ctx, format),
    }
}

fn list(ctx: &Context, format: OutputFormat, filter: ActivityFilter) -> Result<(), AppError> {
    let page = ctx.activity.list(filter)?;
    match format {
        OutputFormat::Json => emit_json(&page),
        OutputFormat::Human => {
            if page.events.is_empty() {
                return emit_text(&format!("(no events; total in store: {})", page.total));
            }
            let mut out = String::new();
            out.push_str("ID       TIMESTAMP_MS    DIR  STATUS   COMMAND          PEER             MESSAGE\n");
            for e in &page.events {
                out.push_str(&format!(
                    "{:>7}  {:>14}  {:<4} {:<8} {:<16} {:<16} {}\n",
                    e.id,
                    e.event.timestamp_ms,
                    direction_label(&e.event.direction),
                    status_label(&e.event.status),
                    e.event.command.as_deref().unwrap_or("-"),
                    e.event.peer_ae_title.as_deref().unwrap_or("-"),
                    e.event.message,
                ));
            }
            out.push_str(&format!(
                "\n({} shown, {} total)\n",
                page.events.len(),
                page.total
            ));
            emit_text(&out)
        }
    }
}

fn count(ctx: &Context, format: OutputFormat) -> Result<(), AppError> {
    let n = ctx.activity.count()?;
    match format {
        OutputFormat::Json => emit_json(&n),
        OutputFormat::Human => emit_text(&n.to_string()),
    }
}

fn direction_label(d: &crate::core::dimse::Direction) -> &'static str {
    use crate::core::dimse::Direction;
    match d {
        Direction::Inbound => "in",
        Direction::Outbound => "out",
        Direction::Info => "info",
    }
}

fn status_label(s: &crate::core::dimse::Status) -> &'static str {
    use crate::core::dimse::Status;
    match s {
        Status::Info => "info",
        Status::Success => "success",
        Status::Warning => "warning",
        Status::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_flags_map_to_activity_filter() {
        let flags = ListFlags {
            direction: Some("in".into()),
            status: Some("success".into()),
            peer_ae: Some("PEER".into()),
            command: Some("C-ECHO-RQ".into()),
            association_id: Some("a-123".into()),
            search: Some("hello".into()),
            since_ms: Some(1_700_000_000_000),
            limit: Some(100),
            offset: Some(50),
        };
        let filter: ActivityFilter = flags.into();
        assert_eq!(filter.direction.as_deref(), Some("in"));
        assert_eq!(filter.status.as_deref(), Some("success"));
        assert_eq!(filter.peer_ae_title.as_deref(), Some("PEER"));
        assert_eq!(filter.command.as_deref(), Some("C-ECHO-RQ"));
        assert_eq!(filter.association_id.as_deref(), Some("a-123"));
        assert_eq!(filter.search.as_deref(), Some("hello"));
        assert_eq!(filter.since_ms, Some(1_700_000_000_000));
        assert_eq!(filter.limit, Some(100));
        assert_eq!(filter.offset, Some(50));
    }

    #[test]
    fn empty_list_flags_yield_empty_filter() {
        let flags = ListFlags {
            direction: None,
            status: None,
            peer_ae: None,
            command: None,
            association_id: None,
            search: None,
            since_ms: None,
            limit: None,
            offset: None,
        };
        let filter: ActivityFilter = flags.into();
        assert!(filter.direction.is_none());
        assert!(filter.peer_ae_title.is_none());
        assert!(filter.limit.is_none());
    }
}
