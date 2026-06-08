# Changelog

All notable changes to NightOwl are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] — 2026-06-08

### Added
- `nightowl-cli` binary mirroring the MCP tool surface, so the same
  operations exposed to MCP clients can be driven from the shell.

### Documentation
- User-facing README covering install, run, and the CLI/MCP surface.

### Fixed
- Activity log test isolation: `temp_log` now uses `tempfile::TempDir`
  instead of a nanos-suffixed path. Parallel tests previously collided
  on the same nanosecond and shared a SQLite file, producing flaky row
  counts.

### Changed
- Cleared the backlog of clippy warnings (doc list formatting in
  `core.rs` / `lib.rs`, struct-update style in `dimse::build_worklist_query`,
  auto-deref in `peers::write_atomic` call sites). `make lint` is now
  clean. Two internal DIMSE response helpers and the `scu_move_cmd`
  Tauri command opt out of `too_many_arguments` — refactoring those
  signatures was out of scope for the release.

## [0.2.0] — 2026-06-07

### Fixed
- Capture pre-`setup` panics on every platform. A startup breadcrumb
  (`startup-breadcrumb.log`) is written at the top of `run()` and a
  `std::panic::set_hook` writes panic location + payload to
  `early-panic.log` before `panic = "abort"` fast-fails the process.
  Panics inside Tauri's window / webview construction — invisible to
  `report_setup_failure` — are now recorded. Triggered by a Windows
  user reporting a silent crash with exception code `0xc0000409` and
  no `startup-error.log` produced.
- Surface graceful setup failures via `startup-error.log` (in the
  platform app-log directory) and, on macOS, a native dialog via
  `osascript`. Previously a setup failure produced an unsymbolicated
  `SIGABRT` with nothing on stderr when launched from Finder.

### Added
- MIT License.

## [0.1.0] — 2026-05-27

Initial public release. DICOM SCP listener, SCU operations (C-ECHO,
C-FIND, C-MOVE, C-STORE), local SOP Instance store + index, peer
management, Modality Worklist (M11) with DMWL SCP (M12), persistent
activity log, and an opt-in local MCP server exposing read + SCU
tools (M24).

[0.3.0]: https://github.com/aurabx/nightowl/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/aurabx/nightowl/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/aurabx/nightowl/releases/tag/v0.1.0
