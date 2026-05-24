# Phantom: a Tauri desktop app for testing DICOM services

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. There is no `PLANS.md` file checked into this repository; the canonical rules for this format live in `.claude/skills/codex-plans/SKILL.md` — keep this document consistent with that guidance.

## Purpose / Big Picture

Phantom is a single-window macOS desktop application that helps a developer exercise the DICOM network protocol in both directions. After this work the developer can:

1. Launch the app from the dock, see a sidebar with five pages (Peers, SCU, Activity, Store, Settings), and the app immediately begins listening for inbound DICOM associations on a configurable TCP port using a configurable local Application Entity Title.
2. Point the app at a local directory on disk (for example `~/dicom-store`) which the app treats like a tiny PACS: any DICOM files already there are indexed, any files arriving over C-STORE are written there and added to the index, and any C-FIND / C-MOVE / C-GET requests are answered from that index.
3. Use the Peers page to add, edit, and remove remote DICOM nodes by name, AE Title, host, and port.
4. Use the SCU page to send a C-ECHO, C-FIND, C-MOVE, C-GET, or C-STORE request to a configured peer and see the response.
5. Use the Activity page to watch a live, scrolling log of every association that touches the app — inbound and outbound — with timestamps, peer identification, command field, status, and any errors.

"DICOM" here means Digital Imaging and Communications in Medicine — the protocol used by clinical imaging equipment (CT scanners, ultrasound machines, PACS servers) to exchange images and metadata. The network half of that protocol is called DIMSE — DICOM Message Service Element — and it runs over TCP using an association handshake followed by a series of command/data PDUs. The five DIMSE operations this app supports are:

- C-ECHO: the DICOM equivalent of a network ping. One peer asks another "are you alive?" and expects a status reply.
- C-FIND: a query for objects matching an identifier (for example, all studies for patient `MRN12345`). The responder returns zero or more matching identifiers.
- C-MOVE: a request that the responder send specified DICOM objects to a *third* AE Title — typically the requester itself, but identified by AE Title and resolved against the responder's configured peer list.
- C-GET: like C-MOVE but the responder sends the objects back on the *same* association rather than opening a new one to a third party.
- C-STORE: the actual transfer of a DICOM object. Sent by a sender on its own, or sent as the second half of a C-MOVE / C-GET.

The "phantom" name reflects intent: this app stands in for a real PACS or modality during development. It is a developer tool. It is not a clinical product, makes no compliance claims, and has no authentication or transport security in this iteration.

The terminology "instance" is overloaded in DICOM. Throughout this plan we use:

- **Peer**: a remote DICOM node we can talk to. A Peer has a Name (free text), an AE Title (the DICOM identifier, up to sixteen ASCII characters), a Host (DNS name or IP), and a Port.
- **SOP Instance**: a single DICOM object (typically one image). SOP stands for Service-Object Pair; in practice you can read it as "a DICOM file".
- **Local Store** or **Store**: the configured directory on disk that holds the SOP Instances this app serves.

When the user said "managing associated instances" we interpret that as managing the list of Peers. When they said "keep track of instances that are added to it as if it were a PACS" we interpret that as the Local Store of SOP Instances. This interpretation should be confirmed at the end of milestone M1; if wrong, only the terminology in the UI needs to change, not the data model.

## Progress

Use a list with checkboxes to summarize granular steps. Every stopping point must be documented here, even if it requires splitting a partially completed task into two. This section must always reflect the actual current state of the work.

- [x] (2026-05-23) M0: Tauri 2 + React 19 + TS 6 + Tailwind v4 + lucide-react scaffold compiles via `cargo check`, builds via `npm run build`, and launches via `npm run tauri dev`. Sidebar shows Peers / SCU / Activity / Store / Settings. Frontend calls `invoke("ping")` on mount and renders the `pong` reply in the footer.
- [x] (2026-05-23) M1: Settings page persists a config (local AE Title, listen port, store directory) to disk and reloads on launch. `AppConfig` + validators in `src-tauri/src/core/config.rs`, `AppError` discriminated union in `core/error.rs`, `get_config` / `save_config` Tauri commands in `lib.rs`, Settings UI in `src/pages/Settings.tsx`, shared API client + types in `src/lib/api.ts`. Eight backend unit tests passing.
- [x] (2026-05-23) M2: Local Store scanner indexes a directory of DICOM files into a SQLite database and the Store page browses the Patient / Study / Series / SOP Instance hierarchy. `Index` + `parse_dicom` + `rescan_dir` in `src-tauri/src/core/store.rs`, five Tauri commands (`rescan_store`, `list_studies`, `list_series_for_study`, `list_instances_for_series`, `total_instance_count`), Store page tree UI with live-event refresh, initial background scan on boot. Eleven backend tests passing.
- [x] (2026-05-23) M3: SCP listener accepts an association and answers C-ECHO. `core/dimse.rs` binds `0.0.0.0:<port>` on app start, accepts the Verification SOP Class on Implicit/Explicit VR LE, dispatches DIMSE commands, and emits stable `activity` events. `echoscu -aec PHANTOM -aet TESTSCU localhost 11112` returns "Received Echo Response (Success)" with exit 0. Twelve backend tests passing.
- [x] (2026-05-23) M4: SCP C-FIND returns matching identifiers from the local index. Patient Root and Study Root Q/R Find SOP classes negotiated; multi-PDV command + data accumulator in `handle_pdv`; identifier-to-SQL translator supports single value, wildcard, UID list, and date range matching; response identifier is constructed by walking the request keys and populating each from the matched `FindRow`. Verified with `findscu -S -k QueryRetrieveLevel=STUDY` (returns full study row), `-S SERIES` filtered by StudyInstanceUID, and `-P PATIENT` with `PatientName=Doe*` wildcard.
- [x] (2026-05-23) M5: SCP C-STORE accepts inbound objects, writes them to the store directory, and updates the index. Storage SOP Classes (CT, MR, SC, US, CR, DX, Encapsulated PDF) negotiated; JPEG Baseline 8-bit added to the transfer syntax list; `handle_c_store` decodes the dataset, validates the UIDs as path components, wraps with a Part-10 file meta via `FileMetaTableBuilder`, writes to `<store_dir>/<study>/<series>/<sop>.dcm`, refreshes the SQLite index via `ingest_file`, and responds 0x0000 / 0xA700 / 0xC000. Verified with `storescu` against three real MR files: three `Received Store Response (Success)` and three Part-10 files on disk in the right hierarchy.
- [ ] M6: SCP C-MOVE and C-GET return matching SOP Instances. Verified by `movescu` and `getscu` from DCMTK.
- [ ] M7: Peers CRUD UI persists a peer list. Add, edit, delete a peer; the change survives an app restart.
- [ ] M8: SCU page can run C-ECHO, C-FIND, C-MOVE, C-GET, and C-STORE against a configured peer and display the result.
- [x] (2026-05-23) M9: Activity page shows a live event stream of every association (inbound and outbound) with peer, command, status, and timestamp. **Built ahead of M4–M8** to make the next four milestones visually verifiable: every C-FIND / C-STORE / C-MOVE response from M4–M6 will now appear in the live log without instrumentation work. `ActivityLog` in `src-tauri/src/core/activity.rs` persists every `dimse::emit` into `activity_events` (50,000-row cap, trim on every 500 inserts), three Tauri commands (`list_activity`, `clear_activity`, `activity_count`), Activity page with live event subscription, direction / status / search filters, pause-resume toggle, and clear-log button. Seventeen backend tests passing.
- [ ] M10: (Stub only) Modality Worklist SCP placeholder page exists so the worklist work can land later as a single milestone.

Use timestamps when entries are completed, like `- [x] (2026-05-23 11:00Z) Scaffold created.`

## Surprises & Discoveries

Document unexpected behaviors, bugs, optimizations, or insights discovered during implementation. Provide concise evidence.

- Observation (M0): Vite 8 (the current "latest" tag at 8.0.14) ships with Rolldown as its bundler. Rolldown's native binding for `darwin-arm64` is delivered as an optionalDependency that npm refuses to install on Node 20.16 because Vite 8 declares `engines: node ^20.19.0 || >=22.12.0`. The install completes with a warning but the binding is silently absent, so any subsequent `vite build` or `vite dev` crashes with `Cannot find module './rolldown-binding.darwin-universal.node'`.
  Evidence: After `npm install`, `ls node_modules/@rolldown/binding-darwin-arm64/` returned "No such file or directory" even though the optional dep was listed in the rolldown package manifest. `npm run build` then failed with the binding load error.
  Resolution: Pinned Vite to `^7.3.3` and `@vitejs/plugin-react` to `^5.2.0`. Vite 7 uses Rollup (no native binding) and emits only a soft warning on Node 20.16. Future revisit: bumping the user's Node to 22 LTS would unblock Vite 8.

- Observation (M0): Tauri's `generate_context!()` proc macro reads `tauri.conf.json` at compile time and refuses to compile if any path listed under `bundle.icon` is missing. There is no "skip icons in dev" mode.
  Evidence: First `cargo check` failed with `failed to open icon /.../src-tauri/icons/32x32.png: No such file or directory`.
  Resolution: Generated a 1024×1024 solid-colour source PNG and ran `npx tauri icon /tmp/phantom-source.png` to produce the full icon set under `src-tauri/icons/`.

- Observation (M0): The Tauri 2 CLI does not ship default placeholder icons inside `node_modules/@tauri-apps/cli/templates/...` the way the v1 CLI did. The user must supply a source image.
  Evidence: `find node_modules/@tauri-apps/cli -name "*.png"` returned no results after install.
  Resolution: Used Python's stdlib (`struct`, `zlib`) to write a valid 1024×1024 PNG to `/tmp/phantom-source.png` for `tauri icon` to resample. The script is in this plan's Concrete Steps; no third-party imaging tool required.

- Observation (M1): Rustdoc tries to compile indented blocks AND fenced blocks with no language tag inside `///` comments as Rust doctests. A JSON example formatted as an indented block under "the frontend sees, for example:" caused `cargo test` to fail with a syntax error.
  Evidence: `running 1 test ... test src/core/error.rs - core::error::AppError (line 25) ... FAILED ... error: expected one of '.', ';', '?', '}', or an operator, found ':'`.
  Resolution: Reformat the JSON example as an inline backtick span rather than an indented block. For any future multi-line non-Rust example in a doc comment, use a fenced block tagged ` ```text ` or ` ```json ` so rustdoc skips it.

- Observation (M1): On a fresh launch with no prior `config.json`, the Tauri setup callback does create the app config directory and the default `store_dir` (`~/dicom-store`) but does NOT write a config file. The config file is only written when the user clicks Save. This is intentional but a future reader could be surprised that "first launch" leaves the directory empty.
  Evidence: After 75 seconds of `npm run tauri dev` against a clean `~/Library/Application Support/cloud.aurabox.phantom/` and `~/dicom-store/`, both directories existed but `config.json` did not. Documented here so M3 (SCP listener) knows that `cfg.store_dir` is guaranteed to exist by the time the listener starts but `config.json` may not.

- Observation (M2): Killing `npm run tauri dev` with `pkill -TERM` does not always tear down the Vite child process. On the next run, Vite fails fast with `Error: Port 5173 is already in use` and Tauri reports `The "beforeDevCommand" terminated with a non-zero status code` — so the whole dev session fails to start. The Rust scan code never runs, leaving the SQLite database absent and giving the appearance that M2 broke.
  Evidence: First boot attempt produced an empty log apart from the port-in-use error; `lsof -ti :5173 | xargs kill -9` cleared the stale process and the second attempt completed normally.
  Resolution: After any forced kill of tauri dev, run `lsof -ti :5173 | xargs -r kill -9` (or rely on the `pkill -KILL -f vite` step in the test harness) before relaunching. Worth wiring into the Makefile as `make kill-dev` if this becomes a recurring nuisance.

- Observation (M2): The dicom-rs API for accessing tags goes `obj.element(Tag) -> Result<&InMemElement, AccessError>` then `element.to_str() -> Result<Cow<str>, ConvertValueError>`. Two different error types in two lines. The `?` operator can't propagate them to a single user-facing error variant without explicit `.map_err`.
  Evidence: First-pass code with `?` failed to compile until the `req_str` / `opt_str` helpers wrapped both errors into `String` so the caller (`parse_dicom`) can decide whether to skip the file or surface a real error.
  Resolution: The helpers in `core/store.rs` (`req_str`, `opt_str`) are the canonical pattern for any future code that needs to pull tags out of `DefaultDicomObject`. M4 (C-FIND) and M5 (C-STORE) should reuse them rather than reinventing the conversion.

- Observation (M3): `TransferSyntax::erased()` is for the typed, non-erased static (e.g. `dicom_transfer_syntax_registry::entries::IMPLICIT_VR_LITTLE_ENDIAN`). The value returned by `TransferSyntaxRegistry.get(uid)` is *already* erased; calling `.erased()` on it produces an unsatisfied trait bound on the adapter type parameters.
  Evidence: First compile of `encode_command_set` produced `the trait bound 'Box<dyn DataRWAdapter + Send + Sync>: DataRWAdapter' is not satisfied` with `required by a bound in 'TransferSyntax::<D, R, W>::erased'`.
  Resolution: Drop the `.erased()` call after `TransferSyntaxRegistry.get(...)` and pass the borrowed `&TransferSyntax` directly to `write_dataset_with_ts` / `read_dataset_with_ts`.

- Observation (M3): `dicom_ul::ServerAssociation::client_ae_title()` is deprecated in 0.9.1; use `Association::peer_ae_title` from the `dicom_ul::association::Association` trait. The deprecation is a `#[warn]` not an `#[error]`, so it would have compiled and shipped if not noticed.
  Evidence: `warning: use of deprecated method 'dicom_ul::ServerAssociation::<S>::client_ae_title': Call 'peer_ae_title' from trait 'Association'`.
  Resolution: Imported the trait and switched to the trait method. Any future code working with `ServerAssociation` should follow the same pattern.

## Decision Log

Record every decision in the format below.

- Decision: Use the `dicom-rs` ecosystem (the `dicom`, `dicom-object`, `dicom-encoding`, `dicom-transfer-syntax-registry`, `dicom-dictionary-std`, and `dicom-ul` crates from <https://github.com/Enet4/dicom-rs>) for all DICOM parsing and network operations.
  Rationale: It is the only mature pure-Rust DICOM library. Avoids needing a C dependency (DCMTK) or a Python sidecar. The crate already ships example binaries for `storescp`, `storescu`, `echoscu`, `findscu`, `movescu`, `getscu` which give us a known-good reference implementation to follow. Pure-Rust also keeps the macOS build simple.
  Date/Author: 2026-05-23 / plan author.

- Decision: Persist the local SOP Instance index in SQLite via `rusqlite` (bundled feature so we do not require a system SQLite).
  Rationale: C-FIND is a structured query at Patient / Study / Series / Image level. SQL is the natural way to answer those queries. SQLite is one file, no server, and works inside a Tauri app trivially. The alternative (in-memory `HashMap` rebuilt from a directory scan on every launch) does not scale and complicates C-FIND.
  Date/Author: 2026-05-23 / plan author.

- Decision: One single AE Title for the SCP for now; no support for multiple virtual AEs.
  Rationale: A development tool does not need multi-tenant AE behavior. Keeps the configuration model simple. Easy to extend later.
  Date/Author: 2026-05-23 / plan author.

- Decision: No TLS, no authentication, no DICOM user identity negotiation in this iteration.
  Rationale: Local developer tool on the developer's own machine. Adding TLS would expand the scope significantly and is not requested. Document the omission in the Settings page.
  Date/Author: 2026-05-23 / plan author.

- Decision: Verification is done with the DCMTK command-line tools (`echoscu`, `findscu`, `movescu`, `getscu`, `storescu`, `storescp`) installed via Homebrew (`brew install dcmtk`). DCMTK is the reference DICOM implementation; if Phantom interoperates with DCMTK it is by definition correct.
  Rationale: We need an external, independently-implemented peer to test against, otherwise we are only checking that our code is internally consistent. DCMTK is free, widely used, and ships on Homebrew.
  Date/Author: 2026-05-23 / plan author.

- Decision: Persist application config (local AE Title, listen port, store directory, peers) as JSON files in the Tauri app config dir, accessed via the `tauri-plugin-store` plugin.
  Rationale: Native to Tauri 2, no extra dependency to evaluate, isolates the config from the SOP Instance store. Avoids reinventing path handling. Secrets are not stored (we have none in this iteration), so we do not need the OS keychain that `CLAUDE.md` mentions.
  Date/Author: 2026-05-23 / plan author.

- Decision: Frontend talks to backend over Tauri `invoke` for request/response and over Tauri `emit` events for the activity stream.
  Rationale: Standard Tauri pattern. `invoke` gives us request-scoped error handling; `emit` gives us a fire-and-forget broadcast for the activity log.
  Date/Author: 2026-05-23 / plan author.

- Decision: Use React 19 (latest stable) rather than React 18 as `CLAUDE.md` states.
  Rationale: User asked for latest stable versions of all dependencies, which overrides the CLAUDE.md text. React 19 is fully released. `CLAUDE.md` will be updated when M0 lands to keep the two consistent.
  Date/Author: 2026-05-23 / plan author.

- Decision: Pin dependencies to the latest stable major.minor lines at scaffold time using `^` ranges so security patches flow in. Concrete versions captured below in `Interfaces and Dependencies`.
  Rationale: Avoid pinning to obsolete versions; allow patch upgrades without manual intervention; but lock the major because Tauri, React, and the dicom-rs crates make breaking changes between majors.
  Date/Author: 2026-05-23 / plan author.

- Decision: Pin Vite to `^7.3.3` (not the absolute latest `^8.0.x`) and `@vitejs/plugin-react` to `^5.2.0`.
  Rationale: Vite 8 has hard-coded Rolldown bindings that require Node `^20.19.0 || >=22.12.0`. The user's machine runs Node 20.16. npm silently drops the rolldown native binding on engine mismatch, so `vite build` fails with a module-not-found at runtime. Vite 7 uses Rollup, has no native binding requirement, and works on Node 20.16. When Node is upgraded to 22 LTS (recommended), the constraint can be relaxed back to `^8`.
  Date/Author: 2026-05-23 / plan author.

- Decision (M1): Persist `config.json` with plain `serde_json` + `std::fs` (atomic write-temp-then-rename) instead of `tauri-plugin-store`.
  Rationale: The plugin is a key-value store optimised for frontend access with change watchers. For one typed config struct touched only on Save, it adds a runtime dependency without value, and forces `core::config` to depend on Tauri (which makes it impossible to unit-test without an `AppHandle`). The plain implementation takes `&Path`, so the six unit tests around `validate`, `load_or_default`, and `save` round-trip without booting Tauri. The `@tauri-apps/plugin-store` frontend dependency was removed.
  Date/Author: 2026-05-23 / M1 implementer.

- Decision (M1): Reject listen ports below 1024 in `validate(&AppConfig)`.
  Rationale: Binding below 1024 requires root on macOS. If we accepted, e.g., the DICOM-registered port 104, the SCP listener (M3) would silently fail at bind time. Failing fast at validation surfaces the constraint where the user sees it.
  Date/Author: 2026-05-23 / M1 implementer.

- Decision (M1): Tauri commands use snake_case field names on the JSON wire (`local_ae_title`, not `localAeTitle`).
  Rationale: The wire shape mirrors the on-disk `config.json` and the Rust struct exactly — one vocabulary throughout. The TypeScript `AppConfig` interface in `src/lib/api.ts` declares snake_case fields, which is unusual for JS but trades a small style oddity for full schema alignment.
  Date/Author: 2026-05-23 / M1 implementer.

- Decision (M2): The Store module lives in one file `src-tauri/src/core/store.rs` rather than the `store/` subdirectory tree the plan's Interfaces section suggested.
  Rationale: ~500 lines holds the schema, the `Index` struct, the DICOM parser, the directory scanner, and tests together. Splitting into `index.rs` / `parser.rs` / `scanner.rs` would add three files for clarity that the file table of contents already provides through section comments. Will reconsider in M4 when C-FIND adds non-trivial query translation that may want its own file.
  Date/Author: 2026-05-23 / M2 implementer.

- Decision (M2): The initial scan runs in `tauri::async_runtime::spawn_blocking` from `setup()`, not synchronously.
  Rationale: `rusqlite` is sync and `WalkDir` walks the filesystem; doing the scan synchronously in setup would block window paint until the directory is fully indexed. For an empty store it's microseconds; for a large store it could be seconds. Spawning to a blocking task and emitting `store/scan-completed` when done keeps the UI responsive from frame one.
  Date/Author: 2026-05-23 / M2 implementer.

- Decision (M2): The Store schema treats `patient_id` as `TEXT NOT NULL` but allows empty string.
  Rationale: DICOM declares PatientID as Type 2 — required to be present but may be empty. Real-world files often have empty values. Rejecting them would discard legitimate data; coercing missing/empty PatientID to `""` keeps the column non-nullable (simpler GROUP BY) while still ingesting the file.
  Date/Author: 2026-05-23 / M2 implementer.

- Decision (M2): Phantom only ingests Part-10 DICOM files (the "DICM" preamble + meta header format that `dicom_object::open_file` accepts). Raw datasets without a file meta header are recorded as `Skipped`.
  Rationale: A development tool needs to interoperate with real PACS exports, which are always Part-10. Supporting raw datasets adds a fallback parsing path without a known consumer. Will revisit if a real workflow demands it.
  Date/Author: 2026-05-23 / M2 implementer.

- Decision (M3): The SCP listener uses a thread-per-association model with `std::net::TcpListener` and `dicom-ul`'s synchronous server API, rather than tokio's async network types.
  Rationale: `dicom-ul` 0.9 exposes `establish(std::net::TcpStream)` as the stable path; the async variant is unstable and the conversion between tokio and std streams is awkward in 2026. A dev tool that expects single-digit concurrent peers does not need the async story. Thread-per-association is simple, debuggable, and matches `dicom-ul`'s own test patterns. Reconsider if connection counts become a real concern.
  Date/Author: 2026-05-23 / M3 implementer.

- Decision (M3): The listener binds on `0.0.0.0` (every interface) rather than `127.0.0.1` (loopback only).
  Rationale: The plan specified `0.0.0.0` so the same Phantom instance can talk to a real modality on the LAN, not just DCMTK on the same machine. The Settings page already carries an amber "no TLS / no auth" warning that doubles as the warning against exposing this on hostile networks. If a follow-up makes the bind interface configurable, default to loopback.
  Date/Author: 2026-05-23 / M3 implementer.

- Decision (M3): Activity events fire over a single Tauri event name (`activity`) with a stable JSON payload (`ActivityEvent` in `core/dimse.rs`).
  Rationale: M9 will build the Activity page and persistent log on top of this stream. Pinning the event name and shape now means M9 is just persistence + UI; the producer side does not need to change. Direction (`inbound` / `outbound` / `info`) and Status (`info` / `success` / `warning` / `error`) are kept narrow so the UI can colour-code with a small switch.
  Date/Author: 2026-05-23 / M3 implementer.

- Decision (M3): Bind failure is fatal (`setup()` returns Err and the app does not start).
  Rationale: The user picked the port. Silently running without an SCP would defeat the whole point of the app. If the port is in use the error surfaces immediately in the dev console / dock launch failure, prompting the user to free the port or change Settings before relaunch.
  Date/Author: 2026-05-23 / M3 implementer.

- Decision: Build M9 (activity log) ahead of M4 / M5 / M6 / M8.
  Rationale: M3 already emits activity events into the void. Persisting and visualising them now means M4–M6 (C-FIND / C-STORE / C-MOVE) become visually verifiable for free — every new DIMSE command lights up the Activity page without any extra wiring. Otherwise the implementer of M4 would need temporary logging instrumentation only to throw it away when M9 lands. Reordering the milestones costs us nothing because M9 only depends on the `ActivityEvent` shape that M3 has already pinned.
  Date/Author: 2026-05-23 / M9 implementer.

- Decision (M9): The activity table lives in the same `store.sqlite` file as the SOP Instance index, but `ActivityLog` opens its own SQLite `Connection`.
  Rationale: One database file is simpler to back up / clear / migrate than two. WAL mode (already enabled by the index) supports multiple readers without lock contention. Separate connections mean the activity mutex does not contend with the store-index mutex during a busy rescan.
  Date/Author: 2026-05-23 / M9 implementer.

- Decision (M9): `dimse::emit` fetches the `ActivityLog` from Tauri state via `app.try_state::<Arc<ActivityLog>>()` rather than receiving it as an explicit parameter.
  Rationale: `emit` is called from a dozen sites in the dimse code (inbound dispatch, outbound responses, listener lifecycle). Threading an explicit `&ActivityLog` through every call site would double the parameter count of half the functions. State lookup is a `RwLock::try_read` on a small map — cheap. The signature stays `emit(&AppHandle, ActivityEvent)`, matching M3.
  Date/Author: 2026-05-23 / M9 implementer.

- Decision (M9): Direction and Status are persisted as their lowercase JSON discriminator strings (`"inbound"`, `"success"`, …) rather than as integer enums.
  Rationale: The SQLite file is human-readable from the `sqlite3` shell during development without any decoding step. Adding a new enum variant later does not invalidate the existing rows (the unknown token maps to `Info`). The cost is a few bytes per row, which is irrelevant at the 50,000-row cap.
  Date/Author: 2026-05-23 / M9 implementer.

- Decision (M9): Tauri events emit `PersistedActivityEvent` (with `id`) always, even when persistence failed (id = -1 sentinel).
  Rationale: The frontend listener writes events directly into a React state list keyed by `id`. Switching the wire shape on failure would force every consumer to handle two possibilities; emitting one shape always keeps the UI code simple. The `id = -1` value is documented and distinguishable from real ids.
  Date/Author: 2026-05-23 / M9 implementer.

- Decision (M4): The DIMSE receive loop accumulates Command + Data PDVs into an `InFlightCommand` before dispatching, rather than dispatching each PDV separately.
  Rationale: DICOM messages span PDVs — a C-FIND-RQ has the command set in one PDV and the identifier in one or more subsequent Data PDVs (terminated by `is_last`). The dispatcher needs both halves at once. The pattern also generalises to C-STORE / C-MOVE / C-GET where the data set is the SOP Instance itself.
  Date/Author: 2026-05-23 / M4 implementer.

- Decision (M4): Wildcard matching escapes literal SQL metacharacters (`%`, `_`, `\`) in the user input before translating DICOM wildcards (`*` → `%`, `?` → `_`).
  Rationale: Without escaping, a user searching for a literal underscore in PatientID would match every single-character pad. The two-stage translation (escape first, then translate DICOM wildcards) means `?` and `*` are the only wildcard characters at the SQL boundary.
  Date/Author: 2026-05-23 / M4 implementer.

- Decision (M4): The response identifier is built by walking the request identifier and populating each tag from the matched `FindRow`. Tags we do not track are returned with empty values.
  Rationale: DICOM C-FIND semantics — every key the client requested (matching or return) appears in the response, populated where available and empty otherwise. This matches what real PACS do; clients written against real PACS work unchanged.
  Date/Author: 2026-05-23 / M4 implementer.

- Decision (M4): Modality filtering at STUDY level is NOT implemented in this milestone (Modality filters at SERIES and IMAGE levels work).
  Rationale: Our schema stores one modality per instance. A STUDY-level filter on Modality should match any study where at least one instance has that modality, which requires either an EXISTS subquery or HAVING with conditional aggregation. Pragmatically, the standard tag at STUDY level is `ModalitiesInStudy` (0008,0061) — a multi-valued return key, not a filter — and most clients use it as a return key. Will revisit if a real workflow needs filter-by-modality at STUDY.
  Date/Author: 2026-05-23 / M4 implementer.

- Decision (M4): `transfer_syntax_uid_for` returns an owned `String` rather than a `&str` tied to the association's lifetime.
  Rationale: Holding an immutable borrow on the association across mutable `send()` calls in the response loop is a borrow-checker error. The cheapest fix is to clone the UID once, drop the borrow, and re-resolve `&TransferSyntax` from the (static) registry inside the loop. `lookup_ts` returns a `&'static TransferSyntax` so the lifetime is unbounded.
  Date/Author: 2026-05-23 / M4 implementer.

- Decision (M5): Introduced `ScpContext { index, store_dir }` to bundle the per-association dependencies, rather than threading two separate parameters through `start_listener` → `run_accept_loop` → `handle_association` → `dispatch` → handler.
  Rationale: Each new handler added in M6 / future milestones will likely need both the index and the store directory (and probably the peer list). One `Arc<ScpContext>` parameter beats N positional arguments; adding a field to `ScpContext` is a one-line change versus a five-site refactor.
  Date/Author: 2026-05-23 / M5 implementer.

- Decision (M5): The plan asked for Explicit VR Big Endian (UID `1.2.840.10008.1.2.2`) in the negotiated transfer syntax list; we DROPPED it.
  Rationale: `dicom_dictionary_std::uids::EXPLICIT_VR_BIG_ENDIAN` is marked `#[deprecated = "Retired DICOM UID"]` — DICOM PS3.5-2025 formally retired big-endian transmission. Anything still requiring it is on retired equipment. If a real workflow ever surfaces it, the constant can be inlined as a raw `&str` to bypass the deprecation.
  Date/Author: 2026-05-23 / M5 implementer.

- Decision (M5): UIDs used as filesystem path components are validated against `is_safe_uid` before any path join: 1-64 chars, only ASCII digits and dots, no leading/trailing dot, no `..` segment.
  Rationale: PS3.5 §9.1 already constrains UIDs to this character set, but the data on the wire is peer-controlled. Without validation, a hostile or malformed peer could send `../../../etc/passwd` as a SOPInstanceUID and `target_dir.join(uid)` would escape `store_dir`. The validation is six lines and turns the failure into a clean `Failed: Unable to Process` response.
  Date/Author: 2026-05-23 / M5 implementer.

- Decision (M5): After writing the file, `ingest_c_store` calls `Index::ingest_file(&path)` to refresh the index, re-parsing the file we just wrote.
  Rationale: The alternative is a parallel `Index::ingest_object(path, &InMemDicomObject)` that skips the re-parse. The re-parse cost for one ~500 KB MR slice is sub-millisecond and the code path is one already-tested function. Optimize if a real ingest rate makes it visible.
  Date/Author: 2026-05-23 / M5 implementer.

## Outcomes & Retrospective

Summarize outcomes, gaps, and lessons learned at major milestones or at completion.

### M0 (2026-05-23)

What landed: a Tauri 2.11 desktop app on macOS with a five-page React 19 + Tailwind v4 sidebar shell. The frontend round-trips a `ping` IPC call to the Rust backend on mount and displays the response in a footer, proving the invoke channel works end-to-end. The Rust scaffold has the `main.rs` / `lib.rs` / `core.rs` split that `CLAUDE.md` prescribes, with a unit test on `core::ping`.

Verification (commands run, output observed):

    $ cd src-tauri && cargo check
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.16s
    $ npm run build
    vite v7.3.3 building client environment for production...
    transforming...
    ✓ 1759 modules transformed.
    ...
    ✓ built in 1.20s
    $ npm run tauri dev   # killed after 90s
    Compiling tauri-runtime-wry v2.11.2
    Compiling tauri v2.11.2
    Compiling phantom v0.1.0 (/.../src-tauri)
    Finished `dev` profile target(s) in 36.47s
    Running `target/debug/phantom`

The binary launched without crashing inside the 90-second window we sampled. A human running `npm run tauri dev` from this state should see a 1100×750 window titled "Phantom" with the sidebar and an "IPC self-check: pong" footer.

Gaps to address before M1: none. The scaffold is clean.

Follow-on observations (worth fixing soon, not blocking M1):

- The user's Node is 20.16 which is below Vite 8's required 20.19+. Upgrading to Node 22 LTS would let us bump Vite back to the absolute latest. Not done in M0 because changing the developer's runtime is out of scope.
- `CLAUDE.md` still says "React 18". The Decision Log captures the deviation but `CLAUDE.md` itself should be updated by the project owner to keep the documentation honest.

### M1 (2026-05-23)

What landed: a persistent, validated application configuration with a Settings UI. The user can change the local AE Title, the listen port, and the store directory; invalid input is rejected at the Rust boundary with a structured error the frontend renders inline; valid input is written atomically to `~/Library/Application Support/cloud.aurabox.phantom/config.json` and round-trips across restarts.

Backend pieces (in `src-tauri/src/core/`):

- `error.rs` — single `AppError` enum, externally tagged `Serialize` (`{kind, message}`). Variants: `Io`, `Json`, `Validation` (with `field` + `reason`), `Tauri`, `Internal`. `From` impls for `std::io::Error`, `serde_json::Error`, `tauri::Error`.
- `config.rs` — `AppConfig` struct, `default_with_home`, `is_valid_ae_title`, `validate`, `load_or_default`, `save` (atomic via write-temp + rename). Zero Tauri imports; functions take `&Path`. Six unit tests cover both happy and validation-failure paths.
- `lib.rs` — `AppState { config: Mutex<AppConfig> }` registered via `app.manage`; `setup` resolves `app_config_dir`, creates both dirs, loads the config, seeds the state. `#[tauri::command] get_config / save_config` shim into core::config.

Frontend pieces:

- `src/lib/api.ts` — typed `AppConfig`, `AppError` discriminated union, `isAppError` guard, `formatError` helper, and `getConfig` / `saveConfig` thin wrappers around `invoke`.
- `src/components/Field.tsx` — label + hint + inline-error wrapper used by every form input.
- `src/pages/Settings.tsx` — three inputs (text, number, text), Save and Revert buttons (disabled when not dirty), saved-indicator, an amber warning panel calling out the no-TLS / no-auth posture.

Verification:

    $ cargo test
    test result: ok. 8 passed; 0 failed; ...
    $ npm run build
    ✓ 1761 modules transformed.
    ✓ built in 1.14s
    $ rm -f ~/Library/Application\ Support/cloud.aurabox.phantom/config.json
    $ npm run tauri dev   # killed after 75s
    Finished `dev` profile target(s) in 1.72s
    Running `target/debug/phantom`
    # After kill:
    $ ls ~/Library/Application\ Support/cloud.aurabox.phantom/
    (directory created, no config.json yet — expected; setup only creates dirs)
    $ ls ~/dicom-store/
    (directory created from default)

Gaps to address before M2: none. The config plumbing is what M2's SQLite indexer needs.

Follow-on observations:

- The Settings page accepts a free-text store directory path. A native folder picker (via `tauri-plugin-dialog`) would be a much better UX. Adding it in M2 alongside the Store page makes more sense than retrofitting M1.
- The current SCP-listener-required restart on port change is not yet enforced (because M1 has no SCP listener). When M3 lands, `save_config` will need to either gracefully rebind or surface "restart required" to the user.

### M2 (2026-05-23)

What landed: a SQLite-backed SOP Instance index over the configured store directory, a recursive scanner that ingests Part-10 DICOM files, and a Store page that browses the Patient / Study / Series / SOP Instance hierarchy. The index lives at `<app config dir>/store.sqlite`; an initial scan runs automatically when the app boots and emits `store/scan-completed` so the UI refreshes without polling; the user can also trigger a rescan from the Store page.

Backend pieces (`src-tauri/src/core/store.rs`):

- `Index::open(path)` creates the schema (one `sop_instances` table plus three secondary indexes), turns on WAL journaling, and returns a clonable handle.
- `parse_dicom(path)` extracts SOPInstanceUID, SeriesInstanceUID, StudyInstanceUID, SOPClassUID, transfer syntax UID (from `obj.meta()`), and optional human-facing tags (PatientName, StudyDescription, etc.). Non-DICOM files return `Err(reason)` which becomes `IngestOutcome::Skipped` rather than a hard failure.
- `Index::ingest_file` `INSERT OR REPLACE`s a row keyed on `sop_instance_uid`, distinguishing first ingest from replacement.
- `Index::rescan_dir` walks the directory, calling `ingest_file` per regular file, summarising into a `ScanReport`.
- `list_studies` / `list_series_for_study` / `list_instances_for_series` / `total_instance_count` queries return shapes ready for direct JSON-IPC serialisation.
- Three unit tests cover schema creation, empty-dir rescan, and non-DICOM-file skip behavior.

Backend wiring (`src-tauri/src/lib.rs`):

- `AppState` now holds `Arc<Index>` alongside the existing config mutex.
- `setup` opens the index and spawns the initial scan via `tauri::async_runtime::spawn_blocking`, so the window paints before the scan completes.
- `rescan_store` / `list_studies` / `list_series_for_study` / `list_instances_for_series` / `total_instance_count` exposed as Tauri commands.
- `tracing_subscriber` initialised with `RUST_LOG` honoured; scan summaries appear at `info` level.

Frontend (`src/pages/Store.tsx`):

- Tree view with three depth levels (Study → Series → SOP Instance).
- Lazy expansion: series and instance lists are fetched only when the user clicks the chevron, so opening a study with thousands of instances doesn't fetch everything upfront.
- Listens to `store/scan-completed` via `@tauri-apps/api/event#listen` and refreshes the study list when it fires. The "Rescan now" button kicks off `rescan_store` and the same event-driven refresh.
- Last-scan summary in the header (`X seen · Y new · Z updated · …`) is the same data emitted by the backend.
- Empty-state message instructs the user how to populate the store.

Verification (commands run, output captured):

    $ make test-rust
    test result: ok. 11 passed; 0 failed
    $ make build-web
    ✓ 1762 modules transformed, built in 1.56s

    # End-to-end: copy 3 real MR images + 1 non-DICOM into the store,
    # boot the app, kill it, read the SQLite index back.
    $ cp /path/to/IM00000{1,2,3}.dcm ~/dicom-store/
    $ echo "this is not dicom" > ~/dicom-store/readme.txt
    $ npm run tauri dev   # killed after 75s
    INFO phantom_lib: initial scan completed seen=4 inserted=3 updated=0 skipped=1 errored=0 elapsed_ms=4
    $ sqlite3 ~/Library/Application\ Support/cloud.aurabox.phantom/store.sqlite \
        "SELECT patient_name, modality, sop_class_uid, transfer_syntax_uid FROM sop_instances;"
    Doe^Giovanni|MR|1.2.840.10008.5.1.4.1.1.4|1.2.840.10008.1.2.1
    Doe^Giovanni|MR|1.2.840.10008.5.1.4.1.1.4|1.2.840.10008.1.2.1
    Doe^Giovanni|MR|1.2.840.10008.5.1.4.1.1.4|1.2.840.10008.1.2.1
    $ sqlite3 ... "SELECT study_instance_uid, study_description FROM sop_instances GROUP BY study_instance_uid;"
    1.3.6.1.4.1.5962.99.1.2786334768...|RM SPALLA SN

Gaps to address before M3: none. The index is the data source M4 (C-FIND) will query.

Follow-on observations:

- No filesystem watcher yet. If a file is dropped into the store dir while the app is running, the user has to click "Rescan now". `notify` is in the dep list and a small follow-up can hook it up.
- The Settings page still has a free-text store-dir input. With the Store page now showing real data, the UX argument for a native folder picker is stronger; queueing for the M7/M8 round.
- Changing `store_dir` in Settings does not currently re-open the index against the new directory; restart needed. Noting for an M3 or M7 cleanup.

### M5 (2026-05-23)

What landed: Phantom accepts inbound DICOM objects via C-STORE. The SCP negotiates seven Storage SOP Classes (CT, MR, Secondary Capture, Ultrasound, Computed Radiography, DX, Encapsulated PDF) on Implicit VR LE / Explicit VR LE / JPEG Baseline 8-bit. Each C-STORE-RQ → the data set is decoded with the negotiated transfer syntax, the UIDs are validated as filesystem-safe, the object is wrapped with a Part-10 file meta header (`FileMetaTableBuilder`) and written to `<store_dir>/<study>/<series>/<sop>.dcm`, the SQLite index is refreshed by calling `Index::ingest_file` on the freshly-written file, and a C-STORE-RSP with status `0x0000` is returned. Parse or write failures map to `0xC000` (Failed: Unable to Process) or `0xA700` (Refused: Out of Resources).

Backend additions (`src-tauri/src/core/dimse.rs`):

- `ScpContext { index, store_dir }` bundles the per-association dependencies. Threaded through `start_listener` → `run_accept_loop` → `handle_association` → `dispatch` → handler. Future modules (peer list, worklist) drop into this struct without further refactors.
- `STORAGE_SOP_CLASSES` constant array of the seven UIDs; the negotiation builder iterates it. Adding a new modality is a one-line change.
- `handle_c_store` is the outer half — reads MessageID / SOPClassUID / SOPInstanceUID from the command, emits an `inbound C-STORE-RQ` activity event, calls `ingest_c_store`, and emits the C-STORE-RSP with the right DIMSE status from the inner half's `Result`.
- `ingest_c_store` is the inner half — decodes the data set, validates UIDs via `is_safe_uid`, builds the file meta, writes the Part-10 file, and refreshes the SQLite index. Returns the written path on success.
- `build_c_store_rsp` mirrors `build_c_echo_rsp` / `build_c_find_rsp` — the same shape with the C-STORE-RSP command field and an Affected SOP Instance UID echoed back.
- `is_safe_uid` validates 1–64 ASCII chars of digits and dots, no leading/trailing dot, no `..`.

Backend wiring (`src-tauri/src/lib.rs`):

- Builds `Arc<ScpContext>` in `setup` from the config's `store_dir` plus the index handle.
- Added an `info`-level `loaded config` trace log so a future debugger does not have to spelunk to find which `store_dir` the running app is honouring.

Verification:

    $ make test-rust
    test result: ok. 18 passed; 0 failed

    # Repoint config to a clean dir, boot, run storescu against three real MRs.
    $ npm run tauri dev   (background)
    INFO phantom_lib: loaded config store_dir=/Users/xtfer/dicom-store-m5 ae_title=PHANTOM port=11112
    $ storescu -v -aec PHANTOM -aet TESTSCU localhost 11112 \
        IM000001.dcm IM000002.dcm IM000003.dcm
    I: Requesting Association
    I: Association Accepted (Max Send PDV: 16366)
    I: Sending file: IM000001.dcm
    I: Sending Store Request (MsgID 1, MR)
    I: Received Store Response (Success)
    I: Sending file: IM000002.dcm
    I: Received Store Response (Success)
    I: Sending file: IM000003.dcm
    I: Received Store Response (Success)
    I: Releasing Association

    # Files on disk in the expected hierarchy.
    $ find ~/dicom-store-m5 -name "*.dcm" | sort
    .../723.0/729.0/728.0.dcm
    .../723.0/729.0/730.0.dcm
    .../723.0/729.0/731.0.dcm

    # And the SQLite index file_paths agree.
    $ sqlite3 store.sqlite "SELECT file_path FROM sop_instances"
    /Users/xtfer/dicom-store-m5/.../728.0.dcm
    /Users/xtfer/dicom-store-m5/.../730.0.dcm
    /Users/xtfer/dicom-store-m5/.../731.0.dcm

    # And each file passes dcmdump — valid Part-10.
    $ dcmdump ~/dicom-store-m5/.../728.0.dcm
    # Dicom-File-Format
    # Dicom-Meta-Information-Header
    (0002,0002) UI =MRImageStorage
    (0002,0010) UI =LittleEndianExplicit
    (0002,0012) UI [2.25.269557681719719925684832053554655571250] ImplementationClassUID
    (0002,0013) SH [DICOM-rs 0.9.1]
    ...

The Activity page (M9) lights up with one `C-STORE-RQ` (inbound) + one `stored …` (info, success) + one `C-STORE-RSP` (outbound, success) per file — exactly as designed.

Gaps to address before M6: none. The receive loop already accumulates Command + Data PDVs. `Index::find` already returns matched rows. `handle_c_store` already knows how to write a Part-10 file. M6 (C-MOVE / C-GET) is "for each row in the find result, send the file" — which is the same SCU operations that M8 needs anyway, so M6 and M8 share infrastructure.

Follow-on observations:

- Existing rows are replaced via `INSERT OR REPLACE ON sop_instance_uid`. If the same SOP Instance is stored a second time from a different path, the old path is forgotten — but the old file on disk is NOT deleted. A "rescan and purge orphans" garbage-collection pass is a sensible follow-up.
- Big-endian transfer syntax (1.2.840.10008.1.2.2) is not offered — it was retired in DICOM PS3.5-2025 and the dictionary marks it deprecated. Documented in Decision Log; re-add via raw UID string if needed.
- The Part-10 file the SCP writes uses Phantom's implementation class UID (auto-derived by dicom-rs). DICOM allows this; some clinical-system test harnesses log the Implementation Class UID for traceability and may report unfamiliar implementations. Not a real problem; documented.

### M4 (2026-05-23)

What landed: Phantom answers DICOM C-FIND queries. The SCP negotiates Patient Root and Study Root Query/Retrieve Information Models for Find, parses the inbound identifier dataset, translates the matching keys (single value, wildcard, UID list, date range, or Universal) into SQL against the SOP Instance index, and emits one C-FIND-RSP Pending per match (carrying the requested return keys populated) followed by a final C-FIND-RSP Success.

Backend changes (`src-tauri/src/core/dimse.rs`):

- Imports: pulled in `PATIENT_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND`, `STUDY_ROOT_QUERY_RETRIEVE_INFORMATION_MODEL_FIND`, the relevant `tags::*` constants, and the new `FindQuery` / `FindLevel` / `FindRow` / `KeyMatch` types from `core::store`.
- Status codes added: `STATUS_PENDING` (0xFF00), placeholders for Refused / Failed (used by M5 / M6).
- `start_listener` and `run_accept_loop` now take `Arc<Index>` so the dispatcher can query the SOP index.
- Receive loop rewritten around `InFlightCommand`: Command PDV with `CommandDataSetType != 0x0101` parks the command until subsequent Data PDVs complete (`is_last`); only then does dispatch fire. C-ECHO (no data) still dispatches inline.
- `handle_c_find`: looks up the negotiated transfer syntax for the presentation context, decodes the identifier, parses `QueryRetrieveLevel`, builds a `FindQuery`, runs `Index::find`, and for each match builds a response identifier and sends a Pending RSP. Final RSP with Success.
- `build_find_query` translates each key with three helpers: `key_match_text` (wildcards `*` / `?`), `key_match_uid` (backslash-separated → `List`), `key_match_date` (`YYYYMMDD-YYYYMMDD` → `Range`).
- `build_response_identifier` walks the request identifier, copies `QueryRetrieveLevel` from the level we resolved, and populates every other tag from the matched row via `response_value_for`. Tags we don't track come back empty — the DICOM-correct behaviour for return-only keys with no source data.

Backend changes (`src-tauri/src/core/store.rs`):

- New types: `FindLevel`, `KeyMatch`, `FindQuery`, `FindRow`.
- `Index::find(query)` dispatches to one of four level-specific SQL builders.
- `apply_match` is the shared WHERE-clause + parameter binder; handles all four `KeyMatch` variants. Wildcard match escapes SQL metacharacters first (`\\` `%` `_`) then translates DICOM wildcards (`*` → `%`, `?` → `_`) using `ESCAPE '\'`.
- STUDY-level query uses `GROUP_CONCAT(DISTINCT modality)` for `ModalitiesInStudy` and counts series and instances.

Verification:

    $ make test-rust
    test result: ok. 18 passed; 0 failed
    # 3 MR images already in ~/dicom-store from M2.
    $ npm run tauri dev  (background)
    $ findscu -S -k QueryRetrieveLevel=STUDY -k PatientID -k PatientName \
              -k StudyInstanceUID -k StudyDescription -k ModalitiesInStudy \
              -aec PHANTOM -aet TESTSCU localhost 11112
    Find Response: 1 (Pending)
      QueryRetrieveLevel = STUDY
      ModalitiesInStudy   = MR
      StudyDescription    = RM SPALLA SN
      PatientName         = Doe^Giovanni
      PatientID           = 48213468
      StudyInstanceUID    = 1.3.6.1.4.1.5962.99.1.2786334768...
    Received Final Find Response (Success)

    # SERIES level filtered by StudyInstanceUID
    Find Response: 1 (Pending)
      Modality            = MR
      SeriesDescription   = TSE T2 TRS1
      SeriesInstanceUID   = 1.3.6.1.4.1...729.0
    Received Final Find Response (Success)

    # PATIENT level with wildcard
    findscu -P -k QueryRetrieveLevel=PATIENT -k PatientID -k 'PatientName=Doe*'
    Find Response: 1 (Pending)
      PatientName = Doe^Giovanni
      PatientID   = 48213468
    Received Final Find Response (Success)

Gaps to address before M5: none. The PDV accumulator generalises to C-STORE (the inbound SOP Instance is the data set); the response-construction pattern generalises to C-MOVE / C-GET (responses report sub-operation counts in the command set, no identifier needed for the final RSP).

Follow-on observations:

- Modality filtering at STUDY level is not supported (only at SERIES / IMAGE). Documented in Decision Log; revisit when a real workflow demands it.
- AccessionNumber is a common C-FIND key we do not yet index. Easy schema migration when needed.
- C-CANCEL-RQ is not handled — long-running queries cannot be aborted by the requester. Acceptable for a dev tool against a tiny database; document.
- The 30-second association read timeout is per-receive; a slow client could keep the association alive forever by sending PDUs slowly. Not a real attack surface for a dev tool but worth noting.

### M9 (2026-05-23, completed ahead of M4-M8)

What landed: a persistent, live, filterable activity log. Every association event and every DIMSE message that flows through `core::dimse` is now stored in a new `activity_events` table in `store.sqlite` and re-broadcast to the frontend as a Tauri `activity` event with an assigned id. The Activity page subscribes to that stream and prepends rows in real time; filters (direction, status, free-text search), a pause-resume toggle, and a clear-log button are all wired.

Backend pieces (`src-tauri/src/core/activity.rs`):

- `ActivityLog::open(path)` creates the table and two indexes (timestamp_ms, association_id). Opens its own SQLite connection in the shared `store.sqlite` file.
- `record(event)` writes one row, returns `PersistedActivityEvent { id, event }`. Trims on every 500 inserts if `COUNT(*) > 50000`, deleting the oldest excess.
- `list(filter)` builds a parametric WHERE clause from optional fields (direction, status, peer_ae_title, command, association_id, free-text search on message + peer_ae_title, since_ms, limit). Returns newest first. Default limit 500, max 5000.
- `clear()` truncates the table; used by the "Clear log" button.
- Five unit tests: record-then-list round trip, substring search, clear, newest-first ordering, limit capping.

Backend wiring:

- `core/dimse.rs::emit` now calls `app.try_state::<Arc<ActivityLog>>()` and persists each event before emitting it. On persistence failure it emits with `id = -1` so the live UI is never silenced by a transient DB error.
- `lib.rs::setup` manages an `Arc<ActivityLog>` (separate from `AppState`) so any function with an `AppHandle` can fetch it. Three Tauri commands exposed: `list_activity`, `clear_activity`, `activity_count`.

Frontend (`src/pages/Activity.tsx`):

- Initial fetch via `listActivity(filter)`; live subscription via `listen("activity", …)` registered once and reading `paused` from a ref so it always sees the latest value.
- Filter dropdowns and a free-text input refetch through the backend, so SQL does the heavy lifting; the same filter is applied client-side to live-prepended events to keep the table consistent.
- Per-association colour via a tiny hash → palette mapping so messages from one association visually group.
- Direction iconography (down-arrow inbound / up-arrow outbound / info dot) and status dot (info / success / warning / error) colour-coded.
- "Clear log" guarded by a `confirm()` so an accidental click can't nuke debugging context.

Verification:

    $ make test-rust
    test result: ok. 17 passed; 0 failed
    $ make build-web
    ✓ 1762 modules transformed, built in 1.94s
    # Clear prior rows, boot the app, run echoscu twice, query SQLite:
    $ sqlite3 store.sqlite "DELETE FROM activity_events"
    $ npm run tauri dev          (background)
    $ echoscu -aec PHANTOM -aet TESTSCU localhost 11112
    $ echoscu -aec PHANTOM -aet TESTSCU localhost 11112
    $ sqlite3 store.sqlite "SELECT id, direction, peer_ae_title, command, status, message FROM activity_events ORDER BY id"
    3 |info     |        |             |info    |SCP listening on 0.0.0.0:11112 as AE PHANTOM
    4 |info     |TESTSCU |             |info    |association accepted from TESTSCU
    5 |inbound  |TESTSCU |C-ECHO-RQ    |info    |message id 1
    6 |outbound |TESTSCU |C-ECHO-RSP   |success |message id 1 status 0x0000 (Success)
    7 |inbound  |TESTSCU |A-RELEASE-RQ |info    |release requested
    8 |outbound |TESTSCU |A-RELEASE-RP |success |release acknowledged
    9 |info     |TESTSCU |             |info    |association closed
    10..15  (second echoscu — same six-event sequence)

13 rows persisted from two `echoscu` runs (1 startup + 2 × 6 association events). The Tauri event for each landed in the frontend's `listen` callback within a frame of the row being committed.

Gaps to address: none. The producer side (M3 emits, M9 persists + lists) is complete.

Follow-on observations:

- The frontend table is unvirtualized. At MAX_DISPLAYED=2000 rows with current event sizes that should still scroll fluidly, but a busy multi-hour session could hit it. `react-virtuoso` or `@tanstack/react-virtual` is the obvious follow-up.
- The `since_ms` filter is in the backend but unused by the frontend. Polling endpoints that want incremental updates can use it.
- Pause currently drops live events from the display (they are still persisted). A "queue while paused, prepend on resume" mode is possible but adds complexity to the listener; the existing "Refresh" button covers the catch-up case.

### M3 (2026-05-23)

What landed: a working DIMSE SCP. Phantom now binds `0.0.0.0:<listen_port>` on boot, accepts associations from any caller via `dicom-ul` 0.9, negotiates the Verification SOP Class on Implicit and Explicit VR Little Endian, and answers C-ECHO with status `0x0000`. Every association open, every DIMSE message in or out, every release, and every close emits a structured `activity` event ready for the M9 page to subscribe to.

Backend (`src-tauri/src/core/dimse.rs`, ~570 lines):

- `start_listener(port, ae_title, app)` binds `std::net::TcpListener` synchronously so the user learns immediately if the port is in use, spawns a named accept thread, and emits the `SCP listening …` info event.
- `run_accept_loop` accepts connections sequentially and hands each to a per-association `std::thread::spawn`. Each association gets a UUID-derived id so all its events group together in the activity log.
- `handle_association` calls `ServerAssociationOptions::new().accept_any().with_abstract_syntax(VERIFICATION).with_transfer_syntax(IMPLICIT_VR_LE).with_transfer_syntax(EXPLICIT_VR_LE).ae_title(...).establish(stream)`, then loops on `association.receive()`.
- `dispatch_command` decodes the Command Set from the inbound PDV (Implicit VR LE), looks up the CommandField, and dispatches. C-ECHO is implemented; other DIMSE commands log a warning and continue (M4–M6 will implement them).
- `build_c_echo_rsp` and `encode_command_set` use `InMemDicomObject::command_from_element_iter` + `write_dataset_with_ts`. A unit test round-trips the encoder against the decoder.
- `ActivityEvent` payload pinned for M9; `Direction` and `Status` enums are narrow so the UI can colour-code with a small switch.

Wiring (`src-tauri/src/lib.rs`):

- `AppState` now also holds a `ListenerHandle` so the listener cannot be garbage-collected mid-life.
- `setup()` calls `start_listener` *after* opening the index, so a port-in-use failure does not leave the SQLite database opened-then-orphaned. Bind failure is fatal.

Verification (commands run, output captured):

    $ make test-rust
    test result: ok. 12 passed; 0 failed
    $ npm run tauri dev   (running in background)
    INFO phantom_lib::dimse: SCP listening on 0.0.0.0:11112 as AE PHANTOM
    $ echoscu -v -aec PHANTOM -aet TESTSCU localhost 11112
    I: Requesting Association
    I: Association Accepted (Max Send PDV: 16366)
    I: Sending Echo Request (MsgID 1)
    I: Received Echo Response (Success)
    I: Releasing Association
    $ echo $?
    0

Phantom's activity stream during the echoscu run, in order:

    SCP listening on 0.0.0.0:11112 as AE PHANTOM
    association accepted from TESTSCU
    inbound  C-ECHO-RQ      message id 1
    outbound C-ECHO-RSP     message id 1 status 0x0000 (Success)
    inbound  A-RELEASE-RQ   release requested
    outbound A-RELEASE-RP   release acknowledged
    association closed

Gaps to address before M4: none. M4 (C-FIND SCP) layers query handling onto the same dispatch path.

Follow-on observations:

- Settings changes to AE title or port currently take a restart to apply. The listener handle has a `shutdown()` method ready to use; wiring `save_config` to rebind is a follow-up.
- The `cmd` module catalogues the full DIMSE command field table even though M3 only uses two values, so M4/M5/M6 can match on names rather than re-deriving the hex.
- Activity events are fire-and-forget over the `activity` Tauri event channel. They are NOT persisted yet; M9 adds the SQLite-backed log and the UI page.

## Context and Orientation

At the time this plan was authored, the working directory `/Users/xtfer/working/aurabx/_experiments/phantom/` contained only configuration directories (`.claude/`, `.codex/`, `.automatic/`, `.agents/`), instruction files (`CLAUDE.md`, `AGENTS.md`, `opencode.json`, `.mcp.json`), and no source code whatsoever. There is no Tauri scaffold yet. There is no `Cargo.toml`, no `package.json`, no `src/`, no `src-tauri/`. This plan therefore starts from empty.

`CLAUDE.md` at the repo root prescribes the target architecture:

- A Tauri 2 shell with a Rust backend and a webview frontend.
- React 18 + TypeScript on the frontend.
- Tailwind CSS v4 for styling.
- `lucide-react` for icons.
- A specific file layout: `src/` for the frontend, `src/App.tsx` as the root layout with sidebar navigation, `src/components/` for reusable UI; `src-tauri/src/main.rs` as the binary entry, `src-tauri/src/lib.rs` for the thin Tauri command wrappers, `src-tauri/src/core.rs` for shared business logic.
- A coding pattern: business logic lives in `core.rs`; Tauri commands in `lib.rs` are thin wrappers. Frontend calls backend with `invoke()` from `@tauri-apps/api/core`. All `#[tauri::command]` functions must be registered in `generate_handler![]`. User-provided names must be validated with `is_valid_name()` before any filesystem use. Secrets (we have none) would use the `keyring` crate.

We will respect this layout and add subdirectories where it helps: `src-tauri/src/dicom/` for the DICOM module, `src-tauri/src/store/` for the local SOP Instance index, `src-tauri/src/config/` for configuration loading, `src/pages/` for one file per sidebar page, `src/lib/` for shared frontend helpers.

The DICOM library ecosystem we will rely on is `dicom-rs` (<https://github.com/Enet4/dicom-rs>). The relevant crates are:

- `dicom` — umbrella crate that re-exports the others.
- `dicom-object` — read and write DICOM files (the `.dcm` format).
- `dicom-encoding` — transfer syntax encoding and decoding (the wire format).
- `dicom-transfer-syntax-registry` — the registry of supported transfer syntaxes (implicit VR little endian, explicit VR little endian, JPEG, and so on). A *transfer syntax* is how the in-memory DICOM data set is serialized on the wire.
- `dicom-dictionary-std` — the standard data dictionary (mapping of DICOM tags such as `(0010,0010)` "Patient Name" to their meaning).
- `dicom-ul` — the DICOM Upper Layer protocol, which provides PDU encoding, association negotiation, and the primitives needed to build SCP and SCU.

DICOM-specific definitions we will use freely below:

- **SCP** (Service Class Provider): the server side of a DIMSE operation. Phantom *is an SCP* for C-ECHO, C-FIND, C-STORE, C-MOVE, and C-GET.
- **SCU** (Service Class User): the client side. Phantom *is also an SCU* — the user can initiate any of those operations against a configured Peer.
- **PDU** (Protocol Data Unit): the wire-level message format. An association is a sequence of PDUs.
- **Association**: a negotiated TCP connection between two AEs. Both sides agree on which Abstract Syntaxes (services such as "C-ECHO") and which Transfer Syntaxes they support before any DIMSE message is exchanged.
- **Presentation Context**: one (Abstract Syntax, list of Transfer Syntaxes) pair offered during association negotiation. The acceptor picks one Transfer Syntax per offered context, or rejects the context.

Two related projects in the user's workspace handle DICOM but are not Tauri apps and should not be copied wholesale: `/Users/xtfer/working/aurabx/_active/dicom-connector` (terraform and certificate material for a deployed connector) and `/Users/xtfer/working/aurabx/_active/dicom-gateway` (a cloud gateway built around Orthanc and Lua scripts). They establish that the user is comfortable with DICOM concepts, but their code is not directly reusable here.

## Plan of Work

The work is divided into eleven milestones. Each milestone is a complete, demonstrable step. Do not start a later milestone before the earlier ones are observably passing.

### M0 — Tauri scaffold and shell UI

Bootstrap the project so that `npm run tauri dev` opens an empty window with a left sidebar of five empty pages. After this milestone exists: `package.json`, `tsconfig.json`, `vite.config.ts`, `tailwind.config.ts` (or v4 equivalent), `index.html`, `src/main.tsx`, `src/App.tsx`, `src/components/Sidebar.tsx`, `src/pages/{Peers,Scu,Activity,Store,Settings}.tsx`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/src/{main.rs,lib.rs,core.rs}`, `.gitignore`.

The Rust backend should expose one trivial Tauri command (`#[tauri::command] fn ping() -> &'static str { "pong" }`) wired through `lib.rs` and called from `App.tsx` on mount to verify the IPC channel works.

### M1 — Configuration and Settings page

Add a persistent application configuration. The configuration data shape is:

    pub struct AppConfig {
        pub local_ae_title: String,   // up to 16 ASCII characters, default "PHANTOM"
        pub listen_port: u16,         // default 11112 (the DICOM-registered port)
        pub store_dir: PathBuf,       // default $HOME/dicom-store, created if missing
    }

Persist as JSON via `tauri-plugin-store` to a file named `config.json` in the Tauri app config directory. On launch, `core::config::load_or_default()` returns the config; if `store_dir` does not exist it is created.

The Settings page has three labeled inputs (AE Title text, listen port number, store directory path) and a Save button. After save, the values round-trip on app restart. The page also displays a non-editable note that this iteration has no TLS or authentication.

Validate the AE Title with `is_valid_ae_title(&str) -> bool`: one to sixteen characters, ASCII printable, no leading or trailing whitespace, no control characters. Reject invalid input at the boundary and return a structured error from the `save_config` Tauri command. The frontend renders that error inline next to the field.

### M2 — Local Store: directory scanner and SQLite index

Create the SOP Instance index. The schema (in `src-tauri/src/store/schema.sql`):

    CREATE TABLE IF NOT EXISTS sop_instances (
        sop_instance_uid TEXT PRIMARY KEY,
        series_instance_uid TEXT NOT NULL,
        study_instance_uid TEXT NOT NULL,
        patient_id TEXT NOT NULL,
        patient_name TEXT,
        study_description TEXT,
        series_description TEXT,
        modality TEXT,
        study_date TEXT,
        sop_class_uid TEXT NOT NULL,
        transfer_syntax_uid TEXT NOT NULL,
        file_path TEXT NOT NULL UNIQUE,
        size_bytes INTEGER NOT NULL,
        ingested_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_study ON sop_instances(study_instance_uid);
    CREATE INDEX IF NOT EXISTS idx_series ON sop_instances(series_instance_uid);
    CREATE INDEX IF NOT EXISTS idx_patient ON sop_instances(patient_id);

The database lives at `<app config dir>/store.sqlite`.

On startup, after config is loaded, run a background scan of `store_dir` recursively. For each file, attempt to parse as DICOM using `dicom_object::open_file(path)`. Extract the tags above. Insert (or replace by `sop_instance_uid`) into the table. Files that fail to parse are skipped with a warning event.

Expose three Tauri commands:

    #[tauri::command] async fn rescan_store(...) -> Result<ScanReport, AppError>
    #[tauri::command] async fn list_studies(...) -> Result<Vec<StudyRow>, AppError>
    #[tauri::command] async fn list_series_for_study(study_uid: String, ...) -> Result<Vec<SeriesRow>, AppError>

`ScanReport` returns counts: files seen, parsed, skipped, errored.

The Store page shows a tree: each Study expandable into its Series, each Series expandable into its SOP Instances. A "Rescan now" button triggers `rescan_store`. The header shows the total count.

### M3 — DIMSE spike: C-ECHO SCP

This milestone is the riskiest because it is our first contact with `dicom-ul` association negotiation. Treat it as a prototyping milestone: the goal is to *demonstrate* that we can accept an association and answer one C-ECHO before we go any further.

Add a module `src-tauri/src/dicom/scp.rs`. On app start, spawn a tokio task that binds a TCP listener to `0.0.0.0:<listen_port>` from config. For each accepted connection:

1. Use `dicom_ul::association::server::ServerAssociationOptions` to negotiate the association. Accept the Verification SOP Class (UID `1.2.840.10008.1.1`) on Implicit VR Little Endian (`1.2.840.10008.1.2`) and Explicit VR Little Endian (`1.2.840.10008.1.2.1`).
2. Read DIMSE PDUs in a loop. When a C-ECHO-RQ arrives, respond with a C-ECHO-RSP with status `0x0000` (Success).
3. On association release or abort, close the connection.

Emit `activity` events on association open, on each DIMSE message in or out, and on close.

Validation: with Phantom running on `localhost:11112` (the default), and DCMTK installed:

    echoscu -aec PHANTOM -aet TESTSCU localhost 11112

Expected exit code 0 and stdout containing `Received Echo Response (Success)`.

If `dicom-ul` does not expose a clean server API by the time this milestone is implemented, fall back to manually reading PDUs from a `tokio::net::TcpStream` using `dicom_ul::pdu::reader::read_pdu` and writing responses with `dicom_ul::pdu::writer::write_pdu`. The reference for these primitives is the `dicom-rs` repository's `storescp` example. If neither path works after a day of investigation, raise it in the Decision Log and consider an alternative (a thin Rust binding to a small DCMTK helper).

### M4 — SCP C-FIND

Implement Patient Root and Study Root Query/Retrieve Information Models — Find (SOP Class UIDs `1.2.840.10008.5.1.4.1.2.1.1` and `1.2.840.10008.5.1.4.1.2.2.1`). For each C-FIND-RQ:

1. Parse the identifier data set (the query keys).
2. Translate the keys to SQL. Support these matching key types in this iteration: single value, list of UID, wildcard (`*` and `?`) on `PatientName`, range matching on `StudyDate` (`YYYYMMDD-YYYYMMDD`), and Universal Matching (empty key returns all values).
3. Query at the requested level (`PATIENT`, `STUDY`, `SERIES`, `IMAGE`).
4. For each match, emit a C-FIND-RSP with status `0xFF00` (Pending) carrying the identifier with the requested return keys populated.
5. After the last match, send one final C-FIND-RSP with status `0x0000` (Success).

Validation:

    findscu -P -k QueryRetrieveLevel=STUDY -k PatientID -k StudyInstanceUID \
            -aec PHANTOM -aet TESTSCU localhost 11112

Expected: zero or more `--`-delimited identifier blocks printed by `findscu` followed by `Releasing Association`. Place a few sample `.dcm` files in `store_dir` before running.

### M5 — SCP C-STORE and live indexing

Accept the Storage SOP Classes for the common modalities at minimum: CT Image Storage (`1.2.840.10008.5.1.4.1.1.2`), MR Image Storage (`1.2.840.10008.5.1.4.1.1.4`), Secondary Capture (`1.2.840.10008.5.1.4.1.1.7`), Ultrasound (`1.2.840.10008.5.1.4.1.1.6.1`), CR (`1.2.840.10008.5.1.4.1.1.1`), DX (`1.2.840.10008.5.1.4.1.1.1.1`), and the Encapsulated PDF SOP Class (`1.2.840.10008.5.1.4.1.1.104.1`). Negotiate Implicit VR Little Endian and Explicit VR Little Endian; also negotiate Explicit VR Big Endian and the JPEG baseline (`1.2.840.10008.1.2.4.50`) for compatibility (we do not decode the pixel data, we just store the bytes).

For each C-STORE-RQ:

1. Read the data set into a `dicom_object::InMemDicomObject`.
2. Write to `<store_dir>/<study_instance_uid>/<series_instance_uid>/<sop_instance_uid>.dcm`. Create intermediate directories. Use `dicom_object::FileDicomObject::write_to_file` so the file is a valid Part-10 DICOM file with a preamble and meta header.
3. Insert or replace into the SQLite index.
4. Respond with C-STORE-RSP status `0x0000` (Success). On parse or write failure respond with status `0xA700` (Out of Resources) or `0xC000` (Processing failure) as appropriate.

Validation:

    storescu -aec PHANTOM -aet TESTSCU localhost 11112 /path/to/sample.dcm

Expected: `storescu` exits 0; the file appears under `store_dir/<study>/<series>/<sop>.dcm`; the row appears in `store.sqlite`; the Store page in the UI reflects the new study after the next rescan (or immediately, if M2's filesystem watcher is wired up).

### M6 — SCP C-MOVE and C-GET

For C-MOVE, accept the Patient Root and Study Root Query/Retrieve Information Models — Move (`1.2.840.10008.5.1.4.1.2.1.2` and `1.2.840.10008.5.1.4.1.2.2.2`). For each C-MOVE-RQ:

1. The request includes a Move Destination AE Title. Resolve that against the configured Peers list (M7). If unknown, respond C-MOVE-RSP status `0xA801` (Move Destination unknown).
2. Run the query against the index (same logic as C-FIND).
3. Open a *new* association *as an SCU* to the destination Peer, negotiating the relevant Storage SOP Classes.
4. For each matched SOP Instance, send a C-STORE-RQ over that association.
5. Periodically send C-MOVE-RSP `0xFF00` (Pending) updates showing completed / remaining / failed counts.
6. Send final C-MOVE-RSP `0x0000` (Success) when done, or `0xB000` (sub-operations complete, one or more failures) on partial failure.

For C-GET, accept `1.2.840.10008.5.1.4.1.2.1.3` and `1.2.840.10008.5.1.4.1.2.2.3`. Similar to C-MOVE except the C-STORE-RQ sub-operations go back over the *same* association (presentation contexts for the Storage SOP Classes must be negotiated on that association — this is why C-GET requesters offer Storage SOP Classes as SCP-role contexts at association time).

Validation:

    movescu -P -k QueryRetrieveLevel=STUDY -k StudyInstanceUID=<uid> \
            -aem TESTSCU -aec PHANTOM -aet TESTSCU \
            localhost 11112 --port 11113

Here `--port 11113` makes `movescu` start its own SCP on port 11113 so Phantom can send the studies back. Configure a Peer named `TESTSCU` pointing to `localhost:11113` in the Peers page first. Expected: the files arrive in `movescu`'s working directory.

    getscu -P -k QueryRetrieveLevel=STUDY -k StudyInstanceUID=<uid> \
           -aec PHANTOM -aet TESTSCU localhost 11112

Expected: `getscu` writes the files locally.

### M7 — Peers CRUD

Add a `peers.json` file in the Tauri app config dir holding an array of:

    pub struct Peer {
        pub id: String,            // UUID v4
        pub name: String,          // human-readable
        pub ae_title: String,      // up to 16 ASCII chars, validated
        pub host: String,          // DNS or IP
        pub port: u16,
    }

Tauri commands: `list_peers`, `create_peer`, `update_peer`, `delete_peer`. Validation rules: `name` non-empty, `ae_title` passes `is_valid_ae_title`, `host` non-empty, `port` in `1..=65535`. Reject duplicate `ae_title` at the command boundary.

The Peers page shows a table of peers with row actions (edit, delete) and a "+ Add Peer" button that opens a modal form. Edits and deletes persist immediately.

The Peers list is the resolution source for C-MOVE destinations (M6) and for the SCU page (M8).

### M8 — SCU page: outbound DIMSE

The SCU page has a top section to pick a Peer (dropdown of the configured peer list) and an operation (radio buttons for ECHO, FIND, MOVE, GET, STORE). Below that, an operation-specific form:

- ECHO: no inputs. Button "Send Echo".
- FIND: dropdown for level (PATIENT, STUDY, SERIES, IMAGE), text inputs for the matching keys (PatientID, PatientName, StudyInstanceUID, StudyDate, Modality), and a "Run Query" button. Results displayed as a table.
- MOVE: same query inputs as FIND, plus a "Move Destination AE Title" text input. Button "Send Move". Live progress updates from C-MOVE-RSP pending statuses appear under the form.
- GET: same query inputs as FIND. Button "Send Get". Received SOP Instances are written into `store_dir` and indexed.
- STORE: a file picker (or drag-and-drop area) listing `.dcm` files to send. Button "Send Store". Per-file progress and status.

Implement these as `#[tauri::command] async fn scu_echo(peer_id: String) -> Result<EchoResult, AppError>`, `scu_find(...)`, `scu_move(...)`, `scu_get(...)`, `scu_store(...)`. The functions construct a `dicom_ul::association::client::ClientAssociation`, negotiate the relevant SOP Classes, exchange the DIMSE messages, and return a structured result.

Validation: against a known external SCP (most easily a second Phantom instance running on a different port, or DCMTK's `storescp -d --port 11113`), every operation succeeds and the Activity log shows the matching outbound association.

### M9 — Activity log: persistent, live, scrollable

Define one event type:

    pub struct ActivityEvent {
        pub id: i64,                       // autoincrement
        pub timestamp_ms: i64,             // unix ms
        pub direction: Direction,          // Inbound | Outbound
        pub peer_ae_title: Option<String>, // None until association is identified
        pub peer_host: Option<String>,
        pub command: Option<String>,       // "C-ECHO-RQ", "C-STORE-RSP", etc.
        pub status: ActivityStatus,        // Info | Success | Warning | Error
        pub message: String,               // human-readable
        pub association_id: String,        // groups events from one association
    }

Persist to a new SQLite table `activity_events` in `store.sqlite` (capped to the most recent 50,000 rows; trim older rows in a background task). Every event is also `app_handle.emit("activity", &event)` for live streaming.

The Activity page subscribes to the `activity` event with `listen` from `@tauri-apps/api/event`, prepends new events into a virtualized list, and offers filters (direction, status, peer, free-text search) and a "Clear log" button (which truncates the table). A "Pause stream" toggle stops the live prepend without dropping events.

Validation: open the Activity page, then in a terminal run `echoscu -aec PHANTOM -aet TESTSCU localhost 11112`. Expect three rows to appear within one second: association open, C-ECHO-RQ inbound, C-ECHO-RSP outbound, then association close.

### M10 — Worklist stub (placeholder only)

Add a Worklist sidebar entry and an empty page with a single line "Modality Worklist support is planned." Do not implement DMWL SCP in this iteration; the user has explicitly deferred this. The placeholder exists so the future milestone can drop in without re-arranging the sidebar.

## Concrete Steps

Run all commands from the repo root `/Users/xtfer/working/aurabx/_experiments/phantom/` unless stated otherwise.

### Prerequisites

Install the toolchain:

    # Node (use a recent LTS via nvm or homebrew)
    node --version    # expect v20.x or v22.x
    npm --version

    # Rust toolchain
    rustup --version
    rustc --version   # expect 1.78+ (Tauri 2 minimum) — newer is fine

    # macOS build dependency
    xcode-select --install   # if not already present

    # DCMTK (used for verification across all milestones)
    brew install dcmtk
    echoscu --version

### M0 — scaffold

Create the Tauri 2 project. We do this by hand (not via `create-tauri-app`) because the surrounding rules in this repo prescribe specific file layouts.

    # 1. Frontend setup
    npm init -y
    # Set "name" in package.json to "phantom" and "type" to "module".
    npm install --save-dev typescript vite @vitejs/plugin-react
    npm install react react-dom
    npm install --save-dev @types/react @types/react-dom
    npm install lucide-react
    npm install --save-dev tailwindcss@^4 @tailwindcss/vite

    # 2. Tauri CLI (use the v2 CLI as a dev dependency so it's pinned per-project)
    npm install --save-dev @tauri-apps/cli@^2 @tauri-apps/api@^2
    npx tauri init   # answer: app name "phantom", window title "Phantom",
                     # frontend dist "../dist", dev URL "http://localhost:5173",
                     # before dev cmd "npm run dev", before build cmd "npm run build".

This produces `src-tauri/`. Then create the frontend tree (full contents are listed in "Plan of Work" above):

    mkdir -p src/components src/pages src/lib
    # Author index.html, src/main.tsx, src/App.tsx, src/index.css with @import "tailwindcss";
    # src/components/Sidebar.tsx, and the five empty src/pages/*.tsx files.

Edit `src-tauri/src/lib.rs` so its `run` function registers a `ping` command:

    #[tauri::command]
    fn ping() -> &'static str { "pong" }

    #[cfg_attr(mobile, tauri::mobile_entry_point)]
    pub fn run() {
        tauri::Builder::default()
            .invoke_handler(tauri::generate_handler![ping])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }

Expected: `npm run tauri dev` opens a window. The console shows `pong` from `App.tsx` (which calls `invoke("ping")` on mount and `console.log`s the result). The sidebar shows five entries, each routes to an empty page.

### M1 — config and Settings page

    # Backend
    # In src-tauri/Cargo.toml under [dependencies]:
    #   tauri-plugin-store = "2"
    #   serde = { version = "1", features = ["derive"] }
    #   serde_json = "1"
    #   tokio = { version = "1", features = ["full"] }
    #   thiserror = "1"
    #   anyhow = "1"
    # In src-tauri/src/lib.rs, add .plugin(tauri_plugin_store::Builder::new().build())
    # Implement core::config::{AppConfig, load_or_default, save}.
    # Implement is_valid_ae_title.
    # Expose Tauri commands get_config and save_config.

    # Frontend
    npm install @tauri-apps/plugin-store

Run `npm run tauri dev`. On the Settings page, set AE Title to `PHANTOM`, port to `11112`, store dir to `~/dicom-store`, click Save. Close the app. Reopen. The same values appear. Test invalid input: AE Title `"a really long title with spaces"` is rejected with an inline error and the previous value is preserved.

### M2 — Local Store index

    # In src-tauri/Cargo.toml:
    #   rusqlite = { version = "0.32", features = ["bundled"] }
    #   dicom = "0.7"               # double-check the latest published version at impl time
    #   dicom-object = "0.7"
    #   walkdir = "2"
    #   notify = "6"                # filesystem watcher (used at M5 too)
    #   chrono = { version = "0.4", features = ["serde"] }

Implement `core::store::{Index, open_index, ScanReport, scan, query_studies, query_series, query_instances}`. The scanner walks `store_dir` with `walkdir`, parses each file with `dicom_object::open_file`, extracts the tags listed in M2's schema, and writes rows. On first launch with an empty config dir, the SQLite file is created from `schema.sql`.

Validation: drop a known sample DICOM file (any anonymised CT or MR slice you have on disk) into `~/dicom-store`. Click "Rescan now" on the Store page. The study appears in the tree.

### M3 — C-ECHO SCP

    # In src-tauri/Cargo.toml:
    #   dicom-ul = "0.7"
    #   dicom-encoding = "0.7"
    #   dicom-transfer-syntax-registry = "0.7"
    #   dicom-dictionary-std = "0.7"

Implement `core::dicom::scp::{Server, run_server}`. On `App::run`, spawn `tokio::spawn(run_server(config, app_handle))`. For each accepted TCP connection, run the association negotiation accepting Verification SOP Class only, then loop reading PDUs. On C-ECHO-RQ, send C-ECHO-RSP success.

Validation:

    # In terminal A, leave Phantom running.
    # In terminal B:
    echoscu -v -aec PHANTOM -aet TESTSCU localhost 11112

Expected DCMTK output:

    I: Requesting Association
    I: Association Accepted (Max Send PDV: ...)
    I: Sending Echo Request (MsgID 1)
    I: Received Echo Response (Success)
    I: Releasing Association

The Activity page (still empty at this point — we have not yet wired the UI) should at least see the events through `app_handle.emit` printed in the dev console.

### M4 through M9

Follow the implementation outline in "Plan of Work" above. For each milestone:

1. Add the relevant code under `src-tauri/src/dicom/` and `src-tauri/src/store/`.
2. Add or update the corresponding page under `src/pages/`.
3. Add the Tauri commands to the `generate_handler!` macro.
4. Run the DCMTK verification command listed in the milestone.
5. Tick off the matching checkbox in the `Progress` section with a timestamp, and commit.

Frequent commits are expected — one per milestone at minimum, more often is encouraged. Commit messages should reference the milestone, for example `M3: C-ECHO SCP responds successfully to echoscu`.

### M10 — worklist stub

Add `src/pages/Worklist.tsx` containing one paragraph, register it in the sidebar, and tick off the box. No backend code.

## Validation and Acceptance

The final acceptance gate is a scripted end-to-end exercise:

1. Build a release: `npm run tauri build`. The result is `src-tauri/target/release/bundle/macos/Phantom.app`. Open it.
2. On Settings, configure AE Title `PHANTOM`, port `11112`, store dir `~/dicom-store-test` (created automatically).
3. On Peers, add a peer named "DCMTK" with AE Title `TESTSCU`, host `localhost`, port `11113`.
4. In a terminal:

       # Have a few real DICOM files on hand. If you do not, generate dummy ones with:
       img2dcm --input-format JPEG sample.jpg /tmp/sample.dcm   # from dcmtk

       # Send three files to Phantom via C-STORE.
       storescu -aec PHANTOM -aet TESTSCU localhost 11112 /tmp/sample.dcm
       storescu -aec PHANTOM -aet TESTSCU localhost 11112 /tmp/sample2.dcm
       storescu -aec PHANTOM -aet TESTSCU localhost 11112 /tmp/sample3.dcm

5. On the Store page, three new SOP Instances are visible.
6. On the SCU page, select peer "DCMTK", operation `C-FIND`, level `STUDY`, click Run Query. While that runs in another terminal start `storescp --port 11113 -od /tmp/sink -aet TESTSCU` so the move destination exists. Results appear in the SCU page.
7. On the SCU page, select operation `C-MOVE` with the same query and destination `TESTSCU`. Phantom opens an outbound association to localhost:11113 and forwards the studies. `/tmp/sink` fills with the files.
8. On the Activity page, every step above appears in chronological order: inbound C-STORE-RQ, C-STORE-RSP, outbound C-MOVE-RQ, C-FIND-RQ, the move sub-operations as outbound C-STORE-RQ to TESTSCU, and the final C-MOVE-RSP success.

The plan is accepted when steps 1 through 8 complete without errors and the observable behavior matches.

## Idempotence and Recovery

The milestones are additive. Re-running any of the build commands or the DCMTK verifications is safe: the SQLite index uses `INSERT OR REPLACE` on `sop_instance_uid`, the store directory uses content-addressed paths (`<study>/<series>/<sop>.dcm`), and the activity log appends only.

If the SQLite database becomes inconsistent with disk (manual edits, partial scans), delete `<app config dir>/store.sqlite` and restart the app. The scanner rebuilds the index from the directory.

If the Tauri build fails on a clean machine, the most common causes are: missing Xcode command-line tools (`xcode-select --install`); missing Rust toolchain (`rustup install stable`); a corrupted `node_modules` (`rm -rf node_modules && npm install`); or an out-of-date `@tauri-apps/cli` (`npm install --save-dev @tauri-apps/cli@latest`).

Listening port `11112` may be in use; either change the port in Settings or run `lsof -i :11112` to find the offender. The DICOM-registered port is `104`, but binding below 1024 requires root on macOS, so we default to `11112` (the de facto convention for development).

## Artifacts and Notes

A typical successful echoscu verification at M3 should produce something like:

    $ echoscu -v -aec PHANTOM -aet TESTSCU localhost 11112
    I: Requesting Association
    I: Association Accepted (Max Send PDV: 16372)
    I: Sending Echo Request (MsgID 1)
    I: Received Echo Response (Success)
    I: Releasing Association
    $ echo $?
    0

A typical SQLite row after a successful storescu at M5:

    $ sqlite3 ~/Library/Application\ Support/cloud.aurabox.phantom/store.sqlite \
        'SELECT sop_instance_uid, study_instance_uid, modality, file_path FROM sop_instances LIMIT 3;'
    1.2.840...1234|1.2.840...5678|CT|/Users/xtfer/dicom-store-test/1.2.840...5678/1.2.840...9012/1.2.840...1234.dcm

Keep these examples up to date in this section as evidence accumulates.

## Interfaces and Dependencies

Versions captured from npm and crates.io on 2026-05-23. Use the `^` ranges below; allow patch and minor upgrades, lock the major. If a newer major release renames any of the symbols this plan references, update this section.

Crate versions (Rust, `src-tauri/Cargo.toml`):

    [build-dependencies]
    tauri-build = { version = "2.6", features = [] }

    [dependencies]
    tauri = { version = "2.11", features = [] }
    tauri-plugin-store = "2.4"
    serde = { version = "1.0", features = ["derive"] }
    serde_json = "1.0"
    tokio = { version = "1.52", features = ["full"] }
    thiserror = "2.0"
    anyhow = "1.0"
    chrono = { version = "0.4", features = ["serde"] }
    uuid = { version = "1.23", features = ["v4", "serde"] }
    walkdir = "2.5"
    notify = "8.2"
    rusqlite = { version = "0.39", features = ["bundled"] }
    dicom = "0.9"
    dicom-object = "0.9"
    dicom-encoding = "0.9"
    dicom-transfer-syntax-registry = "0.9"
    dicom-dictionary-std = "0.9"
    dicom-ul = "0.9"
    tracing = "0.1"
    tracing-subscriber = { version = "0.3", features = ["env-filter"] }

Package versions (npm, `package.json`):

    {
      "dependencies": {
        "@tauri-apps/api": "^2.11",
        "@tauri-apps/plugin-store": "^2.4",
        "react": "^19.2",
        "react-dom": "^19.2",
        "lucide-react": "^1.16"
      },
      "devDependencies": {
        "@tauri-apps/cli": "^2.11",
        "@types/react": "^19.2",
        "@types/react-dom": "^19.2",
        "@vitejs/plugin-react": "^5.2",
        "typescript": "^6.0",
        "vite": "^7.3",
        "tailwindcss": "^4.3",
        "@tailwindcss/vite": "^4.3"
      }
    }

In `src-tauri/src/core.rs`, declare the module tree:

    pub mod config;
    pub mod store;
    pub mod dicom;
    pub mod activity;
    pub mod peers;

In `src-tauri/src/core/config.rs`, define:

    pub struct AppConfig { pub local_ae_title: String, pub listen_port: u16, pub store_dir: PathBuf }
    pub fn load_or_default(app: &tauri::AppHandle) -> Result<AppConfig, AppError>;
    pub fn save(app: &tauri::AppHandle, cfg: &AppConfig) -> Result<(), AppError>;
    pub fn is_valid_ae_title(s: &str) -> bool;
    pub fn is_valid_name(s: &str) -> bool;

In `src-tauri/src/core/store/mod.rs`:

    pub struct Index { /* opaque, wraps rusqlite::Connection in a Mutex */ }
    pub fn open_index(path: &Path) -> Result<Index, AppError>;
    impl Index {
        pub fn ingest_file(&self, path: &Path) -> Result<IngestOutcome, AppError>;
        pub fn rescan_dir(&self, dir: &Path) -> Result<ScanReport, AppError>;
        pub fn query_studies(&self, q: &FindQuery) -> Result<Vec<StudyRow>, AppError>;
        pub fn query_series(&self, study_uid: &str) -> Result<Vec<SeriesRow>, AppError>;
        pub fn query_instances(&self, series_uid: &str) -> Result<Vec<InstanceRow>, AppError>;
    }

In `src-tauri/src/core/dicom/scp.rs`:

    pub async fn run_server(cfg: AppConfig, app: tauri::AppHandle, idx: Arc<Index>, peers: Arc<PeersStore>) -> Result<(), AppError>;

In `src-tauri/src/core/dicom/scu.rs`:

    pub async fn c_echo(peer: &Peer, calling_ae: &str) -> Result<EchoResult, AppError>;
    pub async fn c_find(peer: &Peer, calling_ae: &str, q: &FindQuery) -> Result<Vec<Identifier>, AppError>;
    pub async fn c_move(peer: &Peer, calling_ae: &str, q: &FindQuery, dest_ae: &str) -> Result<MoveOutcome, AppError>;
    pub async fn c_get(peer: &Peer, calling_ae: &str, q: &FindQuery, idx: &Index, store_dir: &Path) -> Result<GetOutcome, AppError>;
    pub async fn c_store(peer: &Peer, calling_ae: &str, files: &[PathBuf]) -> Result<Vec<StoreOutcome>, AppError>;

The Tauri commands in `src-tauri/src/lib.rs` are thin shims around these — they take `tauri::State<Arc<...>>` for shared state, call the function, and translate `AppError` into a `Result<T, String>` for the frontend.

`AppError` is defined in `src-tauri/src/core/error.rs` using `thiserror::Error`. All boundary failures (config IO, SQLite, DICOM parse, network, validation) are variants of this single enum. The frontend never sees panics; commands never `unwrap` on inputs.

## Revision history

This is the inaugural revision of `PLAN.md`. It establishes the eleven milestones, the technology choices, and the verification approach. No code yet exists.
