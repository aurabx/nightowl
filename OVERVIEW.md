# NightOwl

NightOwl is a desktop developer tool for exercising the DICOM network
protocol in both directions. It runs as a single-window application
that listens for inbound DICOM associations and lets the operator
send DIMSE requests to remote peers from a GUI. It also exposes its
read and active surface as Model Context Protocol (MCP) tools, so
external LLM clients can drive it programmatically.

NightOwl is a developer and integration-testing tool. It is not a
clinical product, makes no compliance claims, and ships with no
transport security or authentication in the current iteration.

## What it does

The application combines five capabilities behind a sidebar UI:

1. A **DICOM SCP** that accepts inbound associations on a configurable
   TCP port and Application Entity (AE) Title.
2. A **DICOM SCU** UI for sending C-ECHO, C-FIND, C-MOVE, and C-STORE
   requests to configured remote peers.
3. A **local SOP Instance store** — a directory on disk indexed in
   SQLite — that behaves like a tiny PACS: received instances are
   written there, and inbound C-FIND, C-MOVE, and C-GET requests are
   answered from the same index.
4. A **Modality Worklist (DMWL) provider** with CRUD over scheduled
   procedure steps and a C-FIND responder on the DMWL information
   model.
5. A **persistent activity log** that records every association and
   every DIMSE message (inbound and outbound) and streams new events
   live to the UI.

An optional **local MCP server** exposes a curated subset of these
capabilities as MCP tools over HTTP on the loopback interface.

## Architecture

NightOwl is built on **Tauri 2**: a Rust backend hosting a webview
that renders a React + TypeScript frontend.

- **Desktop shell**: Tauri 2.11.
- **Frontend**: React 19, TypeScript 6, Vite 7, Tailwind CSS v4,
  lucide-react icons.
- **Backend**: Rust (stable, edition 2021, MSRV 1.77).
- **DICOM stack**: the `dicom-rs` family of crates (`dicom-object`,
  `dicom-core`, `dicom-ul`, `dicom-transfer-syntax-registry`,
  `dicom-dictionary-std`, `dicom-encoding`) for parsing, association
  negotiation, and the DIMSE PDU codec.
- **Persistence**: SQLite (via `rusqlite`, bundled) for the SOP
  Instance index, the activity log, and the worklist; JSON files for
  application config and the peer list.
- **MCP server**: the `rmcp` Rust SDK, hosted inside an `axum` HTTP
  service nested at `/mcp`.

Business logic lives in `src-tauri/src/core/` (split across `config`,
`dimse`, `store`, `peers`, `worklist`, `activity`, `mcp`, and
`error` modules). The `#[tauri::command]` functions in `lib.rs` are
thin wrappers that resolve Tauri state and call into `core`.

The bundled macOS build has a minimum system version of macOS 12.0.

## DICOM surface

### Supported as SCP (inbound)

The listener binds `0.0.0.0:<configured port>` (default `11112`) and
negotiates the following abstract syntaxes:

- Verification (C-ECHO).
- Patient Root and Study Root Query/Retrieve — FIND, MOVE, GET.
- Modality Worklist Information Model — FIND.
- Storage SOP Classes:
  - CT Image Storage
  - MR Image Storage
  - Secondary Capture Image Storage
  - Ultrasound Image Storage
  - Computed Radiography Image Storage
  - Digital X-Ray Image Storage — For Presentation
  - Encapsulated PDF Storage

Transfer syntaxes offered on every association: Implicit VR Little
Endian, Explicit VR Little Endian, and JPEG Baseline 8-bit.

### Supported as SCU (outbound)

Initiated from the SCU page or the MCP `scu_*` tools:

- **C-ECHO** — Verification SOP Class ping. Returns success, status
  code, and elapsed milliseconds.
- **C-FIND** — Patient Root or Study Root, at PATIENT / STUDY /
  SERIES / IMAGE level. Matching keys support single value, wildcard
  (`*`, `?`) on `PatientID` and `PatientName`, UID list, and date
  range.
- **C-MOVE** — asks a remote peer to forward matching SOP Instances
  to a named Move Destination AE Title.
- **C-STORE** — sends one or more local DICOM Part-10 files to a
  remote peer. Per-file outcome (success / failure / extracted SOP
  Instance UID / message) is returned.

C-GET as SCU is not implemented in the current iteration. C-GET as
SCP is implemented.

## UI pages

The sidebar exposes eight pages:

- **Peers** — CRUD for the list of remote DICOM nodes (Name, AE
  Title, Host, Port). Persisted as `peers.json`.
- **SCU** — Pick a peer, pick an operation (Echo / Find / Move /
  Store), fill in the operation-specific form, run, and view the
  per-operation result panel.
- **Activity** — Live, paginated, filterable view of every DIMSE
  event. Filters cover direction (inbound / outbound / info), status,
  peer AE Title, command, association id, free-text search, and
  since-timestamp.
- **Store** — Browser for the local SOP Instance index. Shows the
  Patient → Study → Series → SOP Instance hierarchy, refreshes
  automatically when a background scan completes, and offers a
  manual rescan button.
- **Worklist** — CRUD for Modality Worklist scheduled procedure step
  entries.
- **MCP** — Controls for the local MCP server (enable/disable, port,
  live runtime status badge, and a copy-paste configuration snippet
  plus a `claude mcp add` one-liner for Claude Code).
- **Settings** — Local AE Title, listen port, store directory.
- **About** — App version and a link out to the product page.

## Configuration

All persistent state lives in the platform-specific Tauri app config
directory (on macOS: `~/Library/Application Support/cloud.aurabox.nightowl/`).

| File              | Purpose                                                  |
|-------------------|----------------------------------------------------------|
| `config.json`     | AE Title, listen port, store directory, MCP block.       |
| `peers.json`      | List of remote DICOM peers.                              |
| `store.sqlite`    | SOP Instance index and activity log.                     |
| `worklist.sqlite` | Modality Worklist scheduled procedure step entries.      |

The store directory itself defaults to `~/dicom-store` and is the
directory NightOwl scans, writes to, and answers Q/R requests from.

### Defaults

- Local AE Title: `NIGHTOWL`
- Listen port: `11112`
- Store directory: `~/dicom-store`
- MCP server: disabled; default port `7300` when enabled.

### Validation

- AE Title: 1–16 printable ASCII characters, no leading or trailing
  whitespace.
- Ports: rejected below 1024 (requires root on macOS).
- Store directory: must be an absolute path.
- MCP port: must differ from the DICOM listen port when MCP is
  enabled.

### Hot reload

Saving from the Settings or MCP pages applies changes without a
restart. The DICOM SCP is rebound when AE Title, listen port, or
store directory change. The MCP server is restarted when its
`enabled` flag or port changes. A port that is already in use is
detected by a test bind before the old listener is torn down, so a
bad port change surfaces as an error rather than leaving the app
without a listener.

## Local MCP server

When enabled, NightOwl runs an HTTP MCP server on
`http://127.0.0.1:<port>/mcp` using the streamable HTTP transport.
It binds only to the loopback interface and has no authentication —
the posture matches the rest of the app (developer tool, trusted
local environment).

### Tools exposed

Read tools (11):

- `get_config` — local AE Title, listen port, store directory.
- `list_peers`
- `list_studies`
- `list_series_for_study`
- `list_instances_for_series`
- `count_instances`
- `rescan_store`
- `read_dicom_file`
- `list_worklist`
- `list_activity` (filterable, paginated)
- `count_activity`

Peer management tools (3):

- `create_peer`
- `update_peer`
- `delete_peer`

Active SCU tools (4):

- `scu_echo`
- `scu_find`
- `scu_move`
- `scu_store`

Each tool input is declared as a JSON Schema (via `schemars`) so any
spec-compliant MCP client gets a typed surface for free.

## Activity log

Every association event and every DIMSE message that flows through
the SCP or SCU paths is recorded in the activity log:

- Each row has timestamp, direction (inbound / outbound / info),
  peer AE Title and host, command, status, message, and a
  per-association UUID.
- Stored in SQLite, capped at 50,000 rows, with in-line trimming
  every 500 inserts.
- New events are broadcast as Tauri events so the Activity page
  updates in real time without polling.

## Build and run

| Command         | Effect                                              |
|-----------------|-----------------------------------------------------|
| `make dev`      | Full dev mode with hot reload (Tauri shell + Vite). |
| `make web`      | Frontend dev server only, no Tauri shell.           |
| `make build`    | Release bundle (desktop app).                       |
| `make check`    | Rust + TypeScript compile checks.                   |
| `make test`     | `cargo test` (unit + doc tests).                    |
| `make lint`     | `cargo clippy` with warnings as errors.             |
| `make fmt`      | `cargo fmt`.                                        |
| `make icons`    | Regenerate the Tauri icon set from a source PNG.    |
| `make kill-dev` | Force-kill dev processes and free dev ports.        |

## Limitations and scope

- No TLS and no authentication on the DICOM listener or the MCP
  server. The SCP listens on every interface; use only on a trusted
  local network or behind a firewall.
- Not a clinical product. NightOwl makes no regulatory or compliance
  claims.
- C-GET as SCU is not implemented; C-GET as SCP is.
- Storage SOP Classes accepted are limited to the set listed above.
  Additional SOP Classes are added by amending the negotiation
  table in `core::dimse`.
- The bundled platform target is macOS in the current build; Tauri
  itself supports the other desktop platforms.

## Related

NightOwl is published by Aurabox.
Product page: <https://aurabox.cloud/nightowl>.
