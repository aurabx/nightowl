# Phantom — Phase 2: closing the worklist loop, operational polish, hardening

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

There is no `PLANS.md` in this repo; the canonical rules for this format live in `.claude/skills/codex-plans/SKILL.md`. Keep this plan consistent with that guidance. The prior plan covering milestones M0 through M12 lives at `PLAN.md` in the same directory and is the source of truth for the existing codebase. Read `PLAN.md` first if you have not seen this codebase before — it describes the application architecture, the file layout under `src/` and `src-tauri/`, the DICOM vocabulary (SCP, SCU, DIMSE, SOP Class, Transfer Syntax, association, PDU), and the design decisions that shaped every existing module.

This plan picks up where `PLAN.md` ended. Phantom today is a working macOS desktop DICOM service tester: SCP for C-ECHO, C-FIND (Patient Root + Study Root + Modality Worklist), C-STORE, C-MOVE, C-GET; SCU for C-ECHO, C-FIND, C-MOVE, C-STORE; persistent SOP index, peers list, activity log, worklist; six-page React UI. Thirty-two backend unit tests pass; every DIMSE command has been verified against DCMTK's `echoscu` / `findscu` / `storescu` / `movescu` / `getscu`.

## Purpose / Big Picture

After this plan, Phantom becomes a serious development tool that can stand in for a real PACS in more realistic clinical workflow tests: it speaks the full worklist round-trip (a modality can not only pull its scheduled studies but also report status back); it survives Settings changes without restarting; it can carry traffic over TLS the way every clinical-grade peer does; and it can host multiple AE Titles in one process (the same way a real PACS routes by Called AE). Three small SCU-side UX improvements complete the loop so an operator can do an end-to-end test without leaving the app.

There are eleven new milestones grouped into four phases. Each milestone is independently verifiable; you can stop and ship after any one of them and the app remains in a known-good state. Phases are an organisational hint, not a hard dependency: M13 → M14 (worklist round-trip) reads more naturally together, M18 (TLS) is a cross-cutting feature touching every association site, and M19 (multi-AE) genuinely depends on M18 only because real multi-AE deployments expect TLS — feel free to do M19 before M18 if that ordering helps.

Vocabulary added in this plan (every term is defined inline the first time it's used, but the table is here for quick reference):

- **MPPS** — Modality Performed Procedure Step. A normalised DICOM SOP class (`1.2.840.10008.3.1.2.3.3`) that lets a modality report exam status (in progress, completed, discontinued) back to the worklist provider. The modality issues an N-CREATE-RQ to start the step and an N-SET-RQ to finalise it.
- **N-service** — Normalised DIMSE service. Unlike C-services (C-ECHO, C-FIND, C-STORE, C-MOVE, C-GET) that act on composite objects (images), N-services act on instance attributes of a normalised SOP class. The seven N-services are N-EVENT-REPORT, N-GET, N-SET, N-ACTION, N-CREATE, N-DELETE.
- **Storage Commitment** — A DICOM service (SOP class `1.2.840.10008.1.20.1`) where an SCU asks an SCP "did you really store these instances?" The exchange is asynchronous: N-ACTION-RQ from the SCU, then N-EVENT-REPORT-RQ from the SCP some time later with the success/failure list.
- **AccessControl** in `dicom-ul` — a trait the server-side association code uses to decide whether to accept or reject an A-ASSOCIATE-RQ based on the Calling and Called AE Titles. Today we use `AcceptAny`; multi-AE hosting requires a custom implementation.
- **N-EVENT-REPORT** — the N-service used to notify the peer of an event. The Storage Commitment SCP sends this to the original SCU to report commit outcomes.

## Progress

Use a list with checkboxes to summarize granular steps. Every stopping point must be documented here. This section must always reflect the actual current state of the work.

- [ ] M13: MPPS SCP (N-CREATE + N-SET handler over the existing dispatch loop). A modality can post a Performed Procedure Step and update its status.
- [ ] M14: Storage Commitment SCP (N-ACTION request handler + N-EVENT-REPORT outbound). A peer can ask Phantom to confirm storage and gets a structured event back.
- [x] (2026-05-25) M15: Settings hot-reload — changing AE Title / port / store dir / MCP enabled / MCP port applies on save without an app restart. `save_config` orchestrates SCP and MCP rebinds; SCP failure is fatal to the save (port pre-validated on port change), MCP failure transitions the runtime state to `Failed` and surfaces in the Settings status badge. The SQLite index path itself stays fixed (per the existing M15 decision).
- [ ] M16: Filesystem watcher on the store directory. New `.dcm` files dropped into the directory get indexed within seconds without the user clicking "Rescan now".
- [ ] M17: Native folder picker for the Settings store directory and the SCU C-STORE file list. Uses `tauri-plugin-dialog`.
- [ ] M18: TLS associations. Phantom can serve and consume DICOM over TLS using `rustls`; per-peer TLS toggles and an optional CA file path in Settings.
- [ ] M19: Multi-AE hosting. One Phantom process serves multiple AE Titles on the same port, each with its own store directory and worklist.
- [ ] M20: User identity negotiation. Optional username/password (DICOM user-identity sub-item, PS3.7 §D.3.3.7) accepted by the SCP and offered by the SCU.
- [ ] M21: C-GET SCU with explicit SCP-role presentation contexts. The SCU page gets a C-GET button that actually works.
- [ ] M22: Live C-MOVE progress in the SCU page. Sub-operation counts stream in as the operation runs rather than landing all at once on completion.
- [ ] M23: Drag-and-drop file picker for the SCU C-STORE form. Replaces the textarea with a real drop zone.
- [x] (2026-05-25) M24: Local MCP server. NightOwl binds an rmcp Streamable-HTTP server on 127.0.0.1:&lt;mcp.port&gt; (default 7300, disabled by default) and exposes 18 tools covering read, peer-management and active SCU surface for LLM clients (Claude Code, etc.).

Use timestamps in completed entries to measure rates of progress, like:

    - [x] (2026-06-01 14:00Z) M13: MPPS SCP committed; full sequence verified with DCMTK `dcmpsmk` + `mppsscu` style traffic.

## Surprises & Discoveries

Document unexpected behaviors, bugs, optimizations, or insights discovered during implementation. Provide concise evidence. Empty at plan-authoring time.

## Decision Log

Record every decision made while working on the plan in the format below.

- Decision: Plan covers all four directions (worklist round-trip, operational polish, production hardening, SCU/UX) rather than picking one.
  Rationale: The user explicitly asked for "Other: all" from a multi-choice question that offered the categories individually. They want a roadmap they can sequence themselves, not a forced ordering. Each milestone is independent so they can be reordered without rework.
  Date/Author: 2026-05-23 / plan author.

- Decision: MPPS lands as M13, Storage Commitment as M14, both as N-service handlers added to the existing dispatch loop in `src-tauri/src/core/dimse.rs`.
  Rationale: The wire-level mechanism is identical to C-services (command + data PDVs over an association), so we extend `cmd::` with the N-service command fields and add new match arms in `dispatch`. The data model (a new `mpps_events` table) is straightforward SQL. Both close the worklist loop without architectural changes.
  Date/Author: 2026-05-23 / plan author.

- Decision: Settings hot-reload (M15) does NOT support changing the SQLite index path or the activity log path at runtime — only the store directory.
  Rationale: Re-opening the SQLite database while threads hold an `Arc<Index>` requires either an inner `RwLock<Index>` or a swap-the-Arc dance with `arc-swap`. The store directory change is the realistic operational case (someone reorganising their dev data); the database path is a static implementation detail. We document the gap rather than over-engineering.
  Date/Author: 2026-05-23 / plan author.

- Decision: Multi-AE hosting (M19) shares one port and dispatches by Called AE Title via a custom `AccessControl` implementation rather than spawning multiple listeners.
  Rationale: PS3.7 §A.1 lets one network endpoint serve multiple AE Titles via the Called AE Title field in A-ASSOCIATE-RQ. Spawning one listener per AE wastes a port per AE and complicates configuration. The custom AccessControl reads the Called AE, looks up the matching AE config, and routes the rest of the association through that AE's `ScpContext`.
  Date/Author: 2026-05-23 / plan author.

- Decision: TLS (M18) uses `rustls` via `dicom-ul`'s `sync-tls` feature flag rather than a separate TLS proxy.
  Rationale: `dicom-ul` 0.9.1 already supports rustls via `tls_config(rustls::ServerConfig)`. No third-party proxy needed; certificates load from disk paths in config. The frontend gets a "Use TLS" toggle per peer plus a Settings section for the SCP cert.
  Date/Author: 2026-05-23 / plan author.

## Outcomes & Retrospective

Summarize outcomes, gaps, and lessons learned at major milestones or at completion. Empty at plan-authoring time.

## Context and Orientation

The reader is assumed to be able to read PLAN.md to understand the existing codebase. The brief summary follows.

The repo root is `/Users/xtfer/working/aurabx/_experiments/phantom`. The frontend is React 19 + TypeScript 6 + Vite 7 + Tailwind v4 + lucide-react, under `src/`. The backend is Tauri 2.11 + Rust 1.77+ under `src-tauri/src/`. The relevant Rust modules are:

- `src-tauri/src/core/config.rs` — `AppConfig { local_ae_title, listen_port, store_dir }`, persisted as `config.json` in the platform app-config directory (macOS: `~/Library/Application Support/cloud.aurabox.phantom/`). Validation via `is_valid_ae_title` and `validate`.
- `src-tauri/src/core/store.rs` — `Index` wrapping a SQLite `Connection` in a `Mutex`, schema `sop_instances`, methods for scanning a directory of `.dcm` files (`rescan_dir`), querying for C-FIND (`find`, `resolve_for_retrieve`), and the supporting types `FindQuery`, `FindRow`, `RetrieveInstance`, `KeyMatch`.
- `src-tauri/src/core/peers.rs` — `PeerStore` persisting `Peer { id, name, ae_title, host, port }` to `peers.json`. CRUD methods plus `find_by_ae_title` (used by C-MOVE's destination resolution).
- `src-tauri/src/core/activity.rs` — `ActivityLog` persisting `ActivityEvent` rows to a `activity_events` table in `store.sqlite`, capped at 50,000 rows. Every DIMSE event flows through here.
- `src-tauri/src/core/worklist.rs` — `WorklistStore` persisting `WorklistEntry` (scheduled procedure step) to its own `worklist.sqlite` file. CRUD plus `find(query: WorklistQuery)` used by the M12 DMWL SCP.
- `src-tauri/src/core/dimse.rs` — the DIMSE module. Contains the SCP listener (`start_listener`, `run_accept_loop`, `handle_association`), the receive loop with multi-PDV command accumulation (`handle_pdv`, `InFlightCommand`), the dispatch (`dispatch` matches on the DIMSE command field), every C-service handler (`handle_c_echo`, `handle_c_find`, `handle_dmwl_find`, `handle_c_store`, `handle_c_move`, `handle_c_get`), and the SCU primitives (`scu_echo`, `scu_find`, `scu_move`, `scu_store`, `open_storage_scu`, `forward_via_c_store`, `send_c_store_on_existing_assoc`). The DIMSE command field table is the `cmd` module at the top of the file.
- `src-tauri/src/core/error.rs` — `AppError` enum that crosses the IPC boundary as a tagged JSON union `{kind, message}`. Variants: `Io`, `Json`, `Validation { field, reason }`, `Tauri`, `Database`, `DicomParse`, `Internal`.
- `src-tauri/src/lib.rs` — the Tauri entrypoint. Holds `AppState`, opens every store in `setup()`, defines every `#[tauri::command]` shim around a core function, manages `Arc<Index>`, `Arc<ActivityLog>`, `Arc<PeerStore>`, `Arc<WorklistStore>`, and the SCP `ListenerHandle`. Bind failure at setup is fatal.

The DICOM library is `dicom-rs` 0.9 (`dicom-object`, `dicom-core`, `dicom-encoding`, `dicom-transfer-syntax-registry`, `dicom-dictionary-std`, `dicom-ul`). The relevant API surfaces are documented inline in `core/dimse.rs` already; the most important pieces are `ServerAssociationOptions`, `ClientAssociationOptions`, `Pdu::PData`, `PDataValue`, `PDataValueType::{Command, Data}`, `InMemDicomObject`, `DataSetSequence`, `FileMetaTableBuilder`, and `association.send_pdata(pc_id)` which returns a `PDataWriter` for chunked data set transfer.

The frontend has six pages under `src/pages/`: Peers, Scu, Activity, Store, Worklist, Settings. Each one talks to the backend through typed wrappers in `src/lib/api.ts` that call `invoke(...)`. Shared UI components live under `src/components/`: `Field`, `Select`, `Modal`, `Pagination`, `Sidebar`.

The Makefile at the repo root has every common dev task. `make help` lists them. The most relevant ones for this plan are `make dev` (run the app), `make test-rust`, `make build-web`, `make kill-dev` (force-clean stale dev processes — this saved a lot of time in M5/M6), and the DCMTK smoke targets `make echoscu / findscu / storescu`.

DICOM command field values (for the cmd:: module). Existing values are in `src-tauri/src/core/dimse.rs::cmd`; new ones for this plan are listed here. The hex values come from PS3.7 Table 7.1-1:

- N_EVENT_REPORT_RQ = 0x0100, N_EVENT_REPORT_RSP = 0x8100
- N_GET_RQ          = 0x0110, N_GET_RSP          = 0x8110
- N_SET_RQ          = 0x0120, N_SET_RSP          = 0x8120
- N_ACTION_RQ       = 0x0130, N_ACTION_RSP       = 0x8130
- N_CREATE_RQ       = 0x0140, N_CREATE_RSP       = 0x8140
- N_DELETE_RQ       = 0x0150, N_DELETE_RSP       = 0x8150

Useful constants from `dicom_dictionary_std::uids`:

- MODALITY_PERFORMED_PROCEDURE_STEP_SOP_CLASS = "1.2.840.10008.3.1.2.3.3"
- STORAGE_COMMITMENT_PUSH_MODEL = "1.2.840.10008.1.20.1"
- STORAGE_COMMITMENT_PUSH_MODEL_INSTANCE = "1.2.840.10008.1.20.1.1" (the well-known SOP Instance UID for push-model)

Useful tag constants from `dicom_dictionary_std::tags`:

- AFFECTED_SOP_CLASS_UID = (0000,0002)
- AFFECTED_SOP_INSTANCE_UID = (0000,1000)
- REQUESTED_SOP_CLASS_UID = (0000,0003)
- REQUESTED_SOP_INSTANCE_UID = (0000,1001)
- EVENT_TYPE_ID = (0000,1002)
- ACTION_TYPE_ID = (0000,1008)
- PERFORMED_PROCEDURE_STEP_STATUS = (0040,0252)
- PERFORMED_STATION_AE_TITLE = (0040,0241)
- PERFORMED_PROCEDURE_STEP_START_DATE = (0040,0244)
- PERFORMED_PROCEDURE_STEP_START_TIME = (0040,0245)
- PERFORMED_PROCEDURE_STEP_END_DATE = (0040,0250)
- PERFORMED_PROCEDURE_STEP_END_TIME = (0040,0251)
- REFERENCED_SOP_SEQUENCE = (0008,1199)
- TRANSACTION_UID = (0008,1195)

## Plan of Work

The work is broken into eleven milestones. Each milestone has a goal paragraph, the concrete files to edit, the verification, and the acceptance behaviour. Do not start a later milestone until the earlier one passes its acceptance test.

### Phase A — Worklist round-trip

This phase makes Phantom a full worklist participant: the modality can pull its scheduled steps via DMWL (already done in M11/M12), report exam status back via MPPS (M13), and confirm storage via Storage Commitment (M14).

#### M13 — MPPS SCP

Goal: a modality can post a Modality Performed Procedure Step to Phantom — first an N-CREATE-RQ to register the start of a procedure (status `IN PROGRESS`), then later an N-SET-RQ to mark it `COMPLETED` or `DISCONTINUED`. Phantom persists each event, exposes the history on a new MPPS tab inside the Worklist page, and the existing Activity log lights up with each N-message.

Files to edit:

- New file: `src-tauri/src/core/mpps.rs` — `MppsStore` persisting `MppsEvent` to a new `mpps_events` table in `store.sqlite`. Fields: `id` (= the SOP Instance UID the modality assigned in N-CREATE), `affected_sop_class_uid`, `status` (IN_PROGRESS / COMPLETED / DISCONTINUED), `scheduled_step_id` (cross-reference back to a `WorklistEntry`), `performed_station_ae_title`, `performed_start_date_time`, `performed_end_date_time` (nullable until N-SET), `created_at`, `updated_at`. Plus a `raw_dataset_json` column holding the full N-CREATE dataset as JSON for forensic visibility — modalities send a LOT of fields and we don't want to map them all into columns.
- `src-tauri/src/core/dimse.rs`:
  - Extend `cmd::` with the six N-service command fields listed in Context.
  - Add `MODALITY_PERFORMED_PROCEDURE_STEP_SOP_CLASS` to the negotiation builder.
  - Add `handle_n_create` and `handle_n_set` handlers, dispatched from `dispatch()`. Each reads the SOP Instance UID from the command, parses the data set with the negotiated TS, persists via `MppsStore`, responds with N-CREATE-RSP (status `0x0000`) or N-SET-RSP (status `0x0000` if the referenced instance exists, `0x0112` "No such SOP Instance" otherwise).
  - Emit standard activity events: `inbound N-CREATE-RQ`, `outbound N-CREATE-RSP`, etc.
- `src-tauri/src/lib.rs`:
  - Open `MppsStore`, manage it, pass into `ScpContext`.
  - Tauri commands: `list_mpps_events`, `get_mpps_event(id)`. No create/update — modalities own MPPS data, the UI is read-only.
- Frontend:
  - `src/pages/Worklist.tsx` grows a tab switcher: "Scheduled" (existing UI) vs "Performed" (new MPPS list).
  - Performed tab shows a table: PerformedStation, Status (badge), Start, End, scheduled step cross-reference, and a "View raw" button that opens a modal with the full JSON dataset.

Verification: there is no widely-available DCMTK MPPS SCU CLI tool, so the verification path uses Phantom's own SCP plus a small Rust integration test that sends a hand-built MPPS message. Optionally, the open-source `dcmqi` MPPS tool or `orthanc-mpps` can be used externally.

Acceptance: after seeding a worklist entry and running the integration test that posts an N-CREATE then an N-SET, the Performed tab shows one row with Status `COMPLETED` and the Activity log shows four events (inbound N-CREATE-RQ, outbound N-CREATE-RSP, inbound N-SET-RQ, outbound N-SET-RSP).

#### M14 — Storage Commitment SCP

Goal: after a peer C-STORE's a batch of instances to Phantom, the peer can ask "did you really store these?" via Storage Commitment Push Model. The SCU sends an N-ACTION-RQ on SOP Class `1.2.840.10008.1.20.1`, SOP Instance `1.2.840.10008.1.20.1.1` with a Referenced SOP Sequence listing what they want commit confirmation for and a Transaction UID. Phantom checks each Referenced SOP Instance UID against the index, then either synchronously or asynchronously sends an N-EVENT-REPORT-RQ back over the same association (or a fresh outbound association) with the success/failure list.

Files to edit:

- `src-tauri/src/core/dimse.rs`:
  - Add `STORAGE_COMMITMENT_PUSH_MODEL` to negotiation.
  - Add `handle_n_action` dispatched from `dispatch()`. Decodes the request, parses Referenced SOP Sequence + Transaction UID, queries the index for each Referenced SOP Instance UID, builds the success/failure response data set, sends the N-ACTION-RSP (status `0x0000`), then immediately (synchronously for M14 simplicity) sends an N-EVENT-REPORT-RQ over the same association with EventTypeID 1 (commit success) or 2 (commit failure, mixed).
  - Persist commit transactions to a new `commitment_transactions` table in `store.sqlite` so the UI can show history.

The whole exchange is one association: A-ASSOCIATE-RQ, N-ACTION-RQ (SCU→SCP), N-ACTION-RSP (SCP→SCU), N-EVENT-REPORT-RQ (SCP→SCU), N-EVENT-REPORT-RSP (SCU→SCP), A-RELEASE.

Files to edit (continued):

- `src-tauri/src/core/commitment.rs` (new) — `CommitmentStore` persisting `CommitmentTransaction { transaction_uid, requester_ae_title, requested_instances, succeeded, failed, completed_at }`. Read-only for the UI.
- `src-tauri/src/lib.rs`: `list_commitment_transactions` Tauri command.
- Frontend: a new "Commitment" tab on the Activity page (or a small panel on the Store page) showing recent transactions.

Verification: DCMTK has `storescu --commit-on-success` which performs C-STORE followed by a Storage Commitment N-ACTION. Use it after seeding the store.

Acceptance: after `storescu --commit-on-success` of three files to Phantom, the DCMTK output shows `Storage Commitment Request Success`, the Activity log shows the full six-PDU exchange in order, and the Commitment panel lists one transaction with `succeeded: 3, failed: 0`.

### Phase B — Operational polish

These three milestones are small, high-frequency-of-use improvements. Each one removes a "restart the app" or "click Rescan Now" friction that exists today.

#### M15 — Settings hot-reload

Goal: after editing the AE Title, listen port, or store directory on the Settings page and clicking Save, the change takes effect immediately without restarting the app. The SCP listener rebinds to the new port (or stays on the old one if unchanged); the SOP index re-opens against the new store directory; the next inbound association uses the new AE Title.

Why this matters: today, editing Settings updates the JSON file but the running SCP keeps the old values. The user has to quit the dock icon and relaunch. Three changes in a single dev session is three restarts.

Files to edit:

- `src-tauri/src/core/dimse.rs`:
  - `ListenerHandle::shutdown()` already exists from M3 but is unused. Make it actually wait for the accept loop to exit (via a `JoinHandle` plus a shutdown atomic check inside the accept iterator).
  - Add a `rebind` function or rebind logic that drops the current listener, opens a new TCP listener on the new port, and resumes accepting.
- `src-tauri/src/lib.rs`:
  - `save_config` becomes the rebind orchestrator: if `local_ae_title` or `listen_port` changed, call `listener.shutdown()` then `start_listener` again with the new values, replacing the `AppState.listener` field. If `store_dir` changed, open a new `Index` against the new path and swap the `Arc<Index>` (via `RwLock<Arc<Index>>` or `arc-swap::ArcSwap`).
  - Hold `AppState.listener` and `AppState.index` behind types that allow swap. Recommended: `tokio::sync::RwLock<ListenerHandle>` for the listener (rebind takes a write lock briefly), and `arc_swap::ArcSwap<Index>` for the index (lock-free reads).
- Frontend:
  - `src/pages/Settings.tsx` shows a transient toast or inline "Restarted SCP on port NNNN" confirmation after save.

Verification: with the dev console open, change the listen port from 11112 to 11113, click Save. Run `make echoscu PORT=11113` and see the same successful echo response without restarting the dev process.

Acceptance: three Settings changes in one session, each one followed by a successful `echoscu` against the new value, with no dev process restart.

#### M16 — Filesystem watcher on the store directory

Goal: drop a `.dcm` file into the configured store directory and within ~1 second the Store page reflects it; remove a file and within ~1 second it disappears from the index.

Why this matters: today the user has to click "Rescan now" after every external change. In a workflow where Phantom acts as a PACS and a separate tool pushes files via cron or rsync, that's manual toil.

Files to edit:

- `src-tauri/Cargo.toml`: `notify` is already in deps (added at M2 but unused).
- `src-tauri/src/core/store.rs`: add `Index::ingest_file_or_skip(path)` that's a no-op for non-DICOM files, and `Index::remove_by_file_path(path)` that deletes the corresponding row. Both already exist in spirit; expose them properly.
- New file `src-tauri/src/core/watcher.rs`: spawns a `notify::recommended_watcher` on the store directory, batches events with a 500 ms debounce, and applies them via `Index::ingest_file_or_skip` and `Index::remove_by_file_path`. Re-watches if the store directory changes (M15 dependency, but graceful if M15 not yet done).
- `src-tauri/src/lib.rs::setup` spawns the watcher after the initial scan.
- Hook into the existing `store/scan-completed` event or add `store/changed` so the Store page refreshes.

Verification: with the app running, `cp ~/data-sets/stow-test/IM000001.dcm ~/dicom-store/`, watch the Store page populate within a second. `rm ~/dicom-store/.../IM000001.dcm`, watch it disappear.

Acceptance: file addition and removal both propagate to the Store page within 2 seconds of the filesystem change, without any user interaction.

#### M17 — Native folder picker for store directory and SCU C-STORE files

Goal: replace the typed text input on the Settings page (for `store_dir`) and the textarea on the SCU page (for C-STORE file list) with native macOS folder/file pickers.

Why this matters: typing absolute paths is friction. Drag-from-Finder works for the SCU textarea today but only with a multi-line paste; a real picker matches the OS UX users expect.

Files to edit:

- `package.json`: add `@tauri-apps/plugin-dialog`.
- `src-tauri/Cargo.toml`: add `tauri-plugin-dialog = "2"`.
- `src-tauri/src/lib.rs`: register the plugin.
- `src-tauri/capabilities/default.json`: add `dialog:default` to permissions.
- `src/pages/Settings.tsx`: "Browse…" button next to the store-dir input that opens `open({ directory: true })`.
- `src/pages/Scu.tsx`: replace the textarea on the C-STORE form with a "Choose files…" button (`open({ multiple: true, filters: [{ name: 'DICOM', extensions: ['dcm'] }] })`) plus a chip-style list of selected files with per-file remove.

Verification: click "Browse…" on Settings, pick a directory in Finder, verify the path populates. Click "Choose files…" on the SCU page, multi-select `.dcm` files, verify they appear as chips and `Send Store` works.

Acceptance: a user can configure the store directory and send a C-STORE without ever typing a file path.

### Phase C — Production hardening

These three milestones move Phantom from "developer tool on my laptop" toward "could deploy in a multi-AE environment with TLS". Each one is independently useful even outside that arc.

#### M18 — TLS associations

Goal: Phantom can both serve and consume DICOM-over-TLS associations using `rustls`. Settings gets a "TLS" section with paths to the SCP server certificate and private key; the Peers form gets a "Use TLS" checkbox plus an optional CA certificate path; the SCU side passes through.

Why this matters: every clinical-grade DICOM deployment uses TLS. Without it Phantom can't participate in real test environments.

Files to edit:

- `src-tauri/Cargo.toml`: enable the `sync-tls` feature on `dicom-ul` (`dicom-ul = { version = "0.9", features = ["sync-tls"] }`). Add `rustls = "0.23"` and `rustls-pemfile = "2"` for cert loading.
- `src-tauri/src/core/config.rs`: extend `AppConfig` with `tls: Option<TlsConfig>` where `TlsConfig { cert_path, key_path }`. Validate that both files exist and parse.
- `src-tauri/src/core/peers.rs`: extend `Peer` with `tls: bool` and `ca_cert_path: Option<String>`.
- `src-tauri/src/core/dimse.rs`:
  - `start_listener`: if `cfg.tls` is Some, build a `rustls::ServerConfig`, call `options.tls_config(...)`, and use `.establish_tls(stream)` instead of `.establish(stream)`.
  - SCU paths: when `peer.tls`, build a `rustls::ClientConfig` (with `ca_cert_path` as the root if set, otherwise system roots), call `options.tls_config(...)`, use `.establish_tls(addr)`.
- Frontend:
  - Settings page: new "TLS" section with file pickers for cert and key.
  - Peers modal: TLS checkbox; CA cert file picker when checked.

Verification: generate a self-signed cert and key with `openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes -subj "/CN=phantom"`. Configure Phantom's TLS section to point at them. Configure a peer to use TLS with the same cert as CA. Use the DCMTK `echoscu` with `--tls` (or `storescu --tls`) to verify.

Acceptance: `make echoscu` succeeds with `--tls` against Phantom; `make storescu` over TLS lands files; Activity log shows associations with a `(TLS)` tag in the lifecycle message.

#### M19 — Multi-AE hosting

Goal: one Phantom process serves multiple AE Titles on the same port. A new "AE Identities" page lists configured AEs, each with its own AE Title, store directory, and worklist. Inbound associations are routed by the Called AE Title in the A-ASSOCIATE-RQ.

Why this matters: a real PACS often hosts multiple AEs (one per department, one per modality vendor) on one network endpoint. Phantom needs this to stand in.

Files to edit:

- `src-tauri/src/core/identities.rs` (new): `Identity { id, ae_title, store_dir, worklist_db_path }` and `IdentityStore` persisting to `identities.json`. The original `AppConfig.local_ae_title` and `store_dir` become the "default" identity for backwards compatibility.
- `src-tauri/src/core/dimse.rs`:
  - Replace `AcceptAny` with a custom `AccessControl` impl that consults `IdentityStore` and rejects unknown Called AE Titles.
  - `handle_association` looks up the matching `Identity` after `establish()` and uses that identity's `ScpContext` (its own `Index`, `WorklistStore`, etc.) for subsequent dispatch.
- `src-tauri/src/lib.rs`: open an `Index` and `WorklistStore` per identity; key them by identity id in a `HashMap<String, ScpContext>`.
- Frontend: new "Identities" sidebar entry; CRUD page similar to Peers.

Verification: configure two identities `RADIOLOGY@./radiology-store` and `CARDIOLOGY@./cardiology-store`. From two terminals, send echoscu to each Called AE on the same port. Each gets its own activity stream.

Acceptance: simultaneous `echoscu -aec RADIOLOGY` and `echoscu -aec CARDIOLOGY` both succeed; `echoscu -aec UNKNOWN` is rejected with an association abort.

#### M20 — User identity negotiation

Goal: optional username/password authentication via DICOM's User Identity sub-item (PS3.7 §D.3.3.7). Settings gets a "Require authentication" toggle with a username/password list; SCU page lets the user enter credentials for a Peer.

Why this matters: many clinical environments require some form of authentication beyond network ACLs. The DICOM user-identity negotiation is the standard way.

Files to edit:

- `src-tauri/src/core/config.rs`: extend `AppConfig` with `auth: Option<AuthConfig>` where `AuthConfig { users: HashMap<String, String> }` (username → bcrypt password hash).
- `src-tauri/src/core/peers.rs`: optional `username` + `password` on each `Peer` (stored encrypted using the OS keychain via the `keyring` crate per `CLAUDE.md`'s guidance).
- `src-tauri/src/core/dimse.rs`:
  - Server side: the user-identity sub-item arrives in the A-ASSOCIATE-RQ's user variables. dicom-ul exposes them via `association.user_variables()` after establish. Check credentials; if invalid, abort the association with a user-identity-rejected reason code.
  - Client side: `ClientAssociationOptions::username` and `.password` are already available in dicom-ul; thread the peer credentials through.

Verification: configure Phantom with a single user. `echoscu --user phantom --password secret` succeeds; `echoscu --user wrong --password wrong` fails with the right rejection reason.

Acceptance: SCU echoscu with correct credentials passes, with wrong credentials fails with `Authentication Failed`; the Activity log records both cases.

### Phase D — SCU + UX polish

Three small follow-ons flagged in `PLAN.md`'s M8 retrospective.

#### M21 — C-GET SCU with role-selection negotiation

Goal: the SCU page's hidden C-GET button works. Phantom acts as a C-GET requester: opens an association offering both the Q/R Get SOP class as SCU-role AND the relevant Storage SOP classes as SCP-role (so the responder can send instances back over the same association); sends C-GET-RQ; receives inbound C-STORE-RQ sub-operations; stores each into the local store directory; counts; reports.

Why this matters: this is the one DIMSE operation a developer can't currently initiate from Phantom, even though every other side of the protocol is supported.

Files to edit:

- `src-tauri/src/core/dimse.rs`:
  - dicom-ul 0.9.1 does not (as of the prior plan's investigation) expose SCP/SCU role-selection negotiation directly. Check the latest version (`cargo update` then re-investigate `ClientAssociationOptions`); if still unsupported, hand-build the A-ASSOCIATE-RQ by composing PDUs at the byte level, using the existing `dicom_ul::pdu::writer::write_pdu` plus a custom `UserVariableItem::SCP_SCU_RoleSelection`. The role-selection sub-item is well-defined (PS3.7 §D.3.3.4): one per SOP class, with bool SCU role bit and bool SCP role bit.
  - `scu_get` function: build the options, establish, send C-GET-RQ, loop receiving PDUs. On each inbound C-STORE-RQ + data, persist via the existing `ingest_c_store` flow, respond with C-STORE-RSP success. On each Pending C-GET-RSP, update counts. Stop on Success.
- `src-tauri/src/lib.rs`: `scu_get_cmd` Tauri command.
- Frontend: enable the C-GET button on the SCU page (currently disabled / not wired); reuse the FIND form for query keys.

Prototyping note: this milestone has the highest risk of the four follow-on phases because of the role-selection unknown. Spend an hour first prototyping a hand-built A-ASSOCIATE-RQ that successfully negotiates SCP-role contexts against a known peer (DCMTK's `storescp` will do). Only after that prototype succeeds, integrate into Phantom.

Verification: `npm run tauri dev` + DCMTK `storescp` on a separate port with a few `.dcm` files. From the SCU page, choose the peer pointing at that storescp, operation C-GET, level STUDY, query for a specific StudyInstanceUID, click "Send Get". Watch the Store page populate with the retrieved instances.

Acceptance: after a successful C-GET against a peer holding 3 instances, the Store page shows 3 new SOP Instances; the C-GET result panel shows `completed 3 failed 0`.

#### M22 — Live C-MOVE progress in the SCU page

Goal: while a C-MOVE runs, the SCU page shows running sub-operation counts (`12 / 47 complete`) rather than sitting on a spinner until the final RSP arrives.

Why this matters: M6's C-MOVE handler emits a Pending C-MOVE-RSP after every sub-operation with current counts. These hit the SCP-side activity stream (you can see them on the Activity page) but the M8 SCU page only displays the final RSP. For large match sets the UI looks frozen for minutes.

Files to edit:

- `src-tauri/src/core/dimse.rs::scu_move`: instead of returning the final result, take a `tauri::AppHandle` and emit `scu/move-progress` events with `{ completed, remaining, failed }` after each Pending RSP. Return the final result as before.
- `src/pages/Scu.tsx`: subscribe to `scu/move-progress` when the operation is C-MOVE and it's running. Render a progress bar + numeric counts.

Verification: configure a peer pointing at DCMTK storescp; run a C-MOVE against Phantom itself targeting a large study (say 30+ instances); watch the SCU page counts tick up.

Acceptance: the SCU page shows live progress updates at least every 200ms during a multi-instance C-MOVE; the final result panel matches what the Activity page recorded.

#### M23 — Drag-and-drop file picker for SCU C-STORE

Goal: drag files from Finder onto a drop zone on the SCU page and they become the list of files to send. Works alongside (not instead of) the M17 file picker.

Why this matters: drag-and-drop is the natural macOS gesture for "use these files".

Files to edit:

- `src/pages/Scu.tsx`: replace the textarea + chip list with a drop zone component. Use Tauri's `getCurrent().onDragDropEvent` for OS-level drag-drop (not just HTML drag-drop, which only handles files dragged within the webview).
- `src-tauri/capabilities/default.json`: add the drag-drop permission.

Verification: select 5 `.dcm` files in Finder, drag them onto the drop zone, see them appear. Click Send Store.

Acceptance: a developer can complete a C-STORE without typing or clicking through a file picker.

### Phase E — Local MCP server

This phase exposes NightOwl's existing capabilities to LLM clients (Claude Code, etc.) through a Model Context Protocol (MCP) server bound on the loopback interface. The goal is to make NightOwl a drivable fixture for automated DICOM testing: an agent can list studies, inspect peers, and actively send DIMSE messages without going through the GUI.

#### M24 — Local MCP server (delivered 2026-05-25)

Goal: an external MCP client can connect to `http://127.0.0.1:<mcp.port>/mcp` (default 7300) and call 14 tools covering the read + active SCU surface. Disabled by default; enabled from a new "Local MCP server" section in Settings. Loopback bind is the only access control — consistent with the existing "no TLS / no auth" posture documented in `PLAN.md`.

Files touched:

- `src-tauri/Cargo.toml` — added `rmcp = "1.7"` with the `server`, `macros`, `schemars` and `transport-streamable-http-server` features; plus `axum = "0.8"`, `tower = "0.5"`, `schemars = "1.2"`.
- `src-tauri/src/core/config.rs` — added nested `McpConfig { enabled, port }` field on `AppConfig` (default `{ enabled: false, port: 7300 }`), `#[serde(default)]` for backwards compatibility with pre-M24 `config.json` files. Validation rejects ports below 1024, port 0, and collisions with `listen_port`, but only when `enabled` is true.
- `src-tauri/src/core/dimse.rs`, `src-tauri/src/core/store.rs`, `src-tauri/src/core/activity.rs` — added `schemars::JsonSchema` derive to `QrRoot`, `ScuQueryKeys`, `FindLevel`, `ActivityFilter` so the rmcp tool macro can derive their input schemas. `ActivityFilter` also gained `Clone`.
- `src-tauri/src/core/mcp.rs` (new) — `NightowlMcp` handler with 14 `#[tool]`-annotated methods (10 read tools, 4 SCU tools); typed parameter structs for each tool; a `ServerHandle` and `start_server(...)` that binds the axum router and spawns the serve loop on the tokio runtime. Four unit tests cover the error mapping and result rendering helpers.
- `src-tauri/src/lib.rs` — `AppState` gained a `mcp: Option<ServerHandle>` field; `setup()` calls `tauri::async_runtime::block_on(mcp::start_server(...))` when `cfg.mcp.enabled`, treating bind failure as logged-and-continue rather than fatal (unlike the SCP listener, MCP is opt-in ancillary).
- `src/lib/api.ts`, `src/pages/Settings.tsx` — `AppConfig` TypeScript interface gained the `mcp` block; Settings page renders a new "Local MCP server" section with an enable checkbox, a port input that disables when the toggle is off, and a copy-to-clipboard preview of the cross-client `mcpServers` JSON snippet (works in Claude Desktop, Claude Code's `~/.claude.json`, Cursor, etc.). The snippet is built from the SAVED port so a typed-but-unsaved port cannot mislead the consumer.

Verification (commands run, output captured):

    $ cd src-tauri && cargo check
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.28s

    $ cd src-tauri && cargo test --lib
    test result: ok. 44 passed; 0 failed; 0 ignored; 0 measured

    $ npm run build
    ✓ 1767 modules transformed, built in 1.43s

End-to-end verification (still to do by the operator):

1. Launch the app: `npm run tauri dev`.
2. Open Settings → "Local MCP server" → check Enable → set port 7300 → Save. Restart NightOwl.
3. From a terminal: `claude mcp add --transport http nightowl http://127.0.0.1:7300/mcp`.
4. In a Claude Code session: `/mcp` should show `nightowl` connected with 14 tools.
5. Call `list_peers` → returns the configured peers JSON.
6. Configure a peer pointing at a local DCMTK `storescp -d 11113 PHANTOM`; call `scu_echo` with that peer's id → returns `{ "success": true, "status": 0, ... }`. The Activity page shows the inbound/outbound association events.

Follow-ups landed alongside M24 (2026-05-25):

- `core::mcp::tests::tool_router_registers_every_documented_tool` — introspection test that catches accidental drops or additions to the 14-tool surface.
- `core::mcp::tests::scu_find_tool_input_schema_includes_query_fields` — verifies schemars actually derives a real schema (not the fallback `any`) for the structured-input tool.
- New `mcp_status` Tauri command + `McpStatusBadge` component — Settings now shows a live running/disabled/failed pill next to the section header, with the bound address.
- Second copy button for the `claude mcp add` CLI one-liner alongside the JSON snippet, for Claude Code users who prefer the CLI over hand-editing `~/.claude.json`.
- (2026-07-19) `create_peer`, `update_peer`, `delete_peer` MCP tools added (peer management, 3 tools). Total tool count: 18 (11 read + 4 SCU + 3 peer-mutation). See CHANGELOG [Unreleased].

Out of scope (deferred):

- Multi-AE awareness — the MCP server sees the single-AE config only. Will follow M19.
- Authentication and TLS on the MCP endpoint — loopback bind is the only barrier.
- CRUD tools for the worklist — read + SCU only by explicit choice. (Peer CRUD was added 2026-07-19; see Follow-ups above.)
- MCP resources and prompts — v1 exposes tools only.

## Concrete Steps

Each milestone's "Files to edit" section above lists the concrete file paths. Beyond that the iteration loop is the same as Phase 1 (see `PLAN.md` and the Makefile):

For every milestone:

1. Read the relevant existing module(s) named in the milestone description so you understand the patterns to follow.
2. Make the listed edits, working backend-first then frontend.
3. Run `make check-rust` to confirm Rust compiles.
4. Run `make test-rust` to confirm existing tests still pass and new tests pass.
5. Run `make build-web` to confirm TypeScript compiles.
6. Run `make kill-dev` then `npm run tauri dev` and perform the verification described in the milestone.
7. Mark the milestone done in `Progress`, write its entry in `Outcomes & Retrospective`, commit with a message that starts `M13: …` etc.

For new SOP classes (M13, M14, M18, M19), the negotiation builder pattern is established in `src-tauri/src/core/dimse.rs::handle_association`. Append one `.with_abstract_syntax(NEW_SOP_CLASS_UID)` line.

For new DIMSE command handlers (M13, M14), the dispatcher pattern is established in `src-tauri/src/core/dimse.rs::dispatch`. Add one new arm; extract the handler into a `handle_n_…` function alongside the existing `handle_c_…` handlers. Reuse `encode_command_set`, `parse_command_set`, `lookup_ts`, `transfer_syntax_uid_for`, `read_u16`, `read_str` — all of these are already-tested helpers used by every C-service.

For new SQLite tables (M13 mpps_events, M14 commitment_transactions, M19 identities), the schema pattern is established in `src-tauri/src/core/store.rs::SCHEMA` and `src-tauri/src/core/worklist.rs::SCHEMA`. Append a new `CREATE TABLE IF NOT EXISTS` plus indexes; for separate-file stores (worklist's separate `worklist.sqlite`), follow the `WorklistStore::open` pattern from M11.

For new Tauri commands, the shim pattern is established in `src-tauri/src/lib.rs`. Each command is a thin wrapper around a core function, taking `State<'_, Arc<…Store>>` and returning `Result<T, AppError>`. Don't forget to add the command name to the `generate_handler![]` macro at the bottom of `run()`.

For new frontend pages or sub-pages, the patterns are established in `src/pages/Peers.tsx` (CRUD with modal), `src/pages/Activity.tsx` (live list with filters), `src/pages/Worklist.tsx` (CRUD with modal + tab switcher in M13's MPPS extension). Reuse `src/components/Field`, `Select`, `Modal`, `Pagination`.

## Validation and Acceptance

The plan is accepted milestone-by-milestone. Each milestone's "Verification" and "Acceptance" sections above describe the exact behaviour to confirm. There is no "final acceptance" for the whole plan because the milestones are independent — you can stop at any point and ship.

The most useful integration test loop is:

1. `make kill-dev` to reset any stale processes.
2. `make test-rust` to confirm 32+ Rust unit tests pass.
3. `make build-web` to confirm the frontend builds.
4. `npm run tauri dev` to start the app.
5. Use DCMTK CLI tools against `localhost:11112` to exercise each new capability: `echoscu` for connectivity, `findscu` for queries, `storescu` for store, `movescu` for move, `getscu` for get, plus `storescu --commit-on-success` for M14, hand-built MPPS PDUs for M13, and the SCU page itself for M21/M22/M23.

## Idempotence and Recovery

Every backend change is additive on top of the existing schema (new tables, new columns, new modules). Existing rows are unaffected. SQLite migrations are unnecessary at this scale because every store opens with `CREATE TABLE IF NOT EXISTS`.

If a milestone partially lands and you need to back out, `git revert <commit>` is safe — there's no cross-milestone state dependency in the database.

For TLS work (M18), the cert/key files are referenced from `config.json`. If the files become unreadable, the SCP listener will fail to start with a clear error. Worst case: edit `config.json` by hand to drop the `tls` section back to `null` and relaunch.

For multi-AE work (M19), the legacy `local_ae_title` config field stays as the "default identity" so an existing Phantom installation continues to work without configuration changes.

The `make kill-dev` target (added in the chore commit just before this plan) is the canonical way to recover from a stale dev process holding port 11112 or 5173 after a failed test. Use it liberally.

## Artifacts and Notes

Each milestone's verification section above includes the expected DCMTK output and the Activity log fragments to confirm success. Capture short transcripts of those in the milestone's entry under `Outcomes & Retrospective` as you complete each one — the next person to touch the worklist round-trip will thank you.

The most useful reference for DIMSE protocol details when reading dicom-rs source: the `dicom-ul-0.9.1` crate source under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/dicom-ul-0.9.1/`. The `src/association/tests.rs` file in that crate has working examples of command-set construction and PDU exchange that pre-dated some of the higher-level helpers we used in `core/dimse.rs`.

## Interfaces and Dependencies

The new dependencies and the modules they enable:

- `notify = "8.2"` (already in deps from M2) → `core::watcher` for M16.
- `tauri-plugin-dialog = "2"` → file/folder pickers for M17 + M23.
- `dicom-ul = { version = "0.9", features = ["sync-tls"] }` + `rustls = "0.23"` + `rustls-pemfile = "2"` → TLS for M18.
- `keyring = "3"` (mentioned by `CLAUDE.md` but unused so far) → secure password storage for M20.
- `arc-swap = "1"` → lock-free `Arc<Index>` swap for M15.

New public types and module entry points to create:

In `src-tauri/src/core/mpps.rs` (M13):

    pub struct MppsEvent { … fields … }
    pub struct MppsStore { conn: Mutex<Connection> }
    impl MppsStore {
        pub fn open(path: &Path) -> Result<Self, AppError>;
        pub fn record_create(&self, event: MppsEvent) -> Result<MppsEvent, AppError>;
        pub fn record_set(&self, sop_instance_uid: &str, updates: …) -> Result<MppsEvent, AppError>;
        pub fn list(&self) -> Result<Vec<MppsEvent>, AppError>;
        pub fn get(&self, sop_instance_uid: &str) -> Result<Option<MppsEvent>, AppError>;
    }

In `src-tauri/src/core/commitment.rs` (M14):

    pub struct CommitmentTransaction { … fields … }
    pub struct CommitmentStore { conn: Mutex<Connection> }
    impl CommitmentStore {
        pub fn open(path: &Path) -> Result<Self, AppError>;
        pub fn record(&self, txn: CommitmentTransaction) -> Result<(), AppError>;
        pub fn list(&self) -> Result<Vec<CommitmentTransaction>, AppError>;
    }

In `src-tauri/src/core/watcher.rs` (M16):

    pub struct WatcherHandle { /* keeps the Notify watcher alive */ }
    pub fn start_watcher(store_dir: PathBuf, index: Arc<Index>, app: AppHandle) -> Result<WatcherHandle, AppError>;

In `src-tauri/src/core/identities.rs` (M19):

    pub struct Identity { id, ae_title, store_dir, worklist_db_path }
    pub struct IdentityStore { /* persisted to identities.json */ }
    impl IdentityStore {
        pub fn list(&self) -> Result<Vec<Identity>, AppError>;
        pub fn create / update / delete / find_by_ae_title(&self, ae: &str) -> Option<Identity>;
    }

In `src-tauri/src/core/dimse.rs` extensions:

    pub mod cmd {
        pub const N_EVENT_REPORT_RQ: u16 = 0x0100;
        pub const N_EVENT_REPORT_RSP: u16 = 0x8100;
        pub const N_GET_RQ: u16 = 0x0110;
        pub const N_GET_RSP: u16 = 0x8110;
        pub const N_SET_RQ: u16 = 0x0120;
        pub const N_SET_RSP: u16 = 0x8120;
        pub const N_ACTION_RQ: u16 = 0x0130;
        pub const N_ACTION_RSP: u16 = 0x8130;
        pub const N_CREATE_RQ: u16 = 0x0140;
        pub const N_CREATE_RSP: u16 = 0x8140;
        pub const N_DELETE_RQ: u16 = 0x0150;
        pub const N_DELETE_RSP: u16 = 0x8150;
    }

    fn handle_n_create(...)        // M13
    fn handle_n_set(...)           // M13
    fn handle_n_action(...)        // M14 (Storage Commitment)
    fn send_n_event_report(...)    // M14 (commit result)

    pub fn scu_get(...)            // M21

All new commands follow the existing shim convention in `lib.rs`:

    #[tauri::command]
    async fn list_mpps_events(state: State<'_, Arc<MppsStore>>) -> Result<Vec<MppsEvent>, AppError> {
        state.list()
    }

…and are registered in the `tauri::generate_handler![…]` macro inside `run()`.

## Revision history

This is the inaugural revision of `PLAN-NEXT.md`. Written 2026-05-23 after the user requested a roadmap covering all four directions discussed at the end of `PLAN.md` (worklist round-trip, operational polish, production hardening, SCU/UX polish). The eleven milestones M13 through M23 are presented in phased order but each is independently shippable; the user may sequence them however they prefer.
