# NightOwl

NightOwl is a desktop tool for exercising the DICOM network protocol in
both directions. It listens for inbound DICOM associations, sends DIMSE
requests to remote peers from a GUI, and exposes the same surface as
Model Context Protocol (MCP) tools and a command-line driver so external
clients — including LLMs — can drive it programmatically.

NightOwl is a developer and integration-testing tool. It is not a
clinical product, makes no compliance claims, and ships without
transport security or authentication. Use it on a trusted local network
or behind a firewall.

Product page: <https://aurabox.cloud/nightowl>.

## What you can do with it

NightOwl combines five capabilities behind a single sidebar UI:

1. A **DICOM SCP** that accepts inbound associations on a configurable
   TCP port and AE Title.
2. A **DICOM SCU** UI for sending C-ECHO, C-FIND, C-MOVE, and C-STORE to
   configured peers.
3. A **local SOP Instance store** — a directory on disk indexed in
   SQLite — that behaves like a tiny PACS: received instances are
   written there, and inbound C-FIND, C-MOVE, and C-GET are answered
   from the same index.
4. A **Modality Worklist (DMWL) provider** with CRUD over scheduled
   procedure steps and a C-FIND responder on the DMWL information
   model.
5. A **persistent activity log** that records every association and
   every DIMSE message, streamed live to the UI.

## Install and launch

NightOwl is built and released for macOS, Windows, and Linux from
GitHub. Download the installer for your platform from the latest
release:

- **macOS** — `.dmg` (minimum macOS 12.0). Drag NightOwl into
  Applications and launch it like any other desktop app.
- **Windows** — `.msi` or `.exe` installer. Run it and launch NightOwl
  from the Start menu.
- **Linux** — `.AppImage` or `.deb`. Make the AppImage executable and
  run it, or install the `.deb` with `apt`.

On first launch, NightOwl creates its configuration directory in the
platform's standard app-data location and a default store directory at
`~/dicom-store` (or the equivalent on Windows). The defaults are:

- Local AE Title: `NIGHTOWL`
- DICOM listen port: `11112`
- Store directory: `~/dicom-store`
- MCP server: disabled

Open the **Settings** page to change the AE Title, listen port, or
store directory. Changes apply immediately — the DICOM listener is
rebound in place, with a test bind first so a bad port surfaces as an
error rather than leaving the app without a listener.

## Using the UI

The sidebar has eight pages:

- **Peers** — Add, edit, and remove remote DICOM nodes. Each peer has
  a Name, AE Title, Host, and Port. Peers are referenced by name on
  the SCU page and by id from the CLI and MCP tools.
- **SCU** — Pick a peer, pick an operation (Echo, Find, Move, Store),
  fill in the form, and run. The result panel shows status, elapsed
  time, and per-operation details (matched rows for Find, per-file
  outcomes for Store, and so on).
- **Activity** — Live, paginated, filterable view of every DIMSE
  event. Filter by direction (inbound / outbound / info), status,
  peer AE Title, command, association id, free-text, or
  since-timestamp. New events appear without polling.
- **Store** — Browse the local SOP Instance index as Patient → Study →
  Series → SOP Instance. Refreshes automatically when a background
  scan finishes; a manual rescan button is also available.
- **Worklist** — CRUD for Modality Worklist scheduled procedure step
  entries served by the DMWL C-FIND responder.
- **MCP** — Enable or disable the local MCP server, set its port, see
  the live runtime status, and copy a ready-made configuration
  snippet (including a `claude mcp add` one-liner for Claude Code).
- **Settings** — Local AE Title, listen port, store directory.
- **About** — Version and a link to the product page.

### DICOM capabilities

As an SCP, NightOwl negotiates Verification, Patient Root and Study
Root Query/Retrieve (Find, Move, Get), Modality Worklist Find, and the
following storage SOP classes: CT, MR, Secondary Capture, Ultrasound,
Computed Radiography, Digital X-Ray (For Presentation), and
Encapsulated PDF. Transfer syntaxes offered on every association are
Implicit VR Little Endian, Explicit VR Little Endian, and JPEG
Baseline 8-bit.

As an SCU, NightOwl can send C-ECHO, C-FIND (Patient Root or Study
Root at Patient / Study / Series / Image level, with wildcard, UID
list, and date-range matching), C-MOVE, and C-STORE. C-GET as SCU is
not implemented in the current iteration.

## MCP server

NightOwl can expose a curated subset of its capabilities as MCP tools
over HTTP on the loopback interface. The MCP server is disabled by
default. Enable it from the **MCP** page, set a port (default `7300`),
and the server starts immediately on `http://127.0.0.1:<port>/mcp`
using the streamable HTTP transport.

The MCP server binds only to `127.0.0.1` and has no authentication;
it is intended for local use by a client running on the same machine.

### Connecting Claude Code

The MCP page shows a ready-to-copy `claude mcp add` command. Run it
in a terminal and Claude Code will connect to NightOwl as a tool
provider.

### Tools exposed

Read tools:

| Tool                        | Purpose                                         |
|-----------------------------|-------------------------------------------------|
| `get_config`                | Local AE Title, listen port, store directory.   |
| `list_peers`                | Configured remote DICOM peers.                  |
| `list_studies`              | Studies in the local store.                     |
| `list_series_for_study`     | Series within a study.                          |
| `list_instances_for_series` | SOP Instances within a series.                  |
| `count_instances`           | Instance count across the store.                |
| `rescan_store`              | Trigger a background rescan of the store dir.   |
| `list_worklist`             | Scheduled procedure step entries.               |
| `list_activity`             | Filterable, paginated activity log.             |
| `count_activity`            | Activity row count.                             |

Active SCU tools:

| Tool         | Purpose                                                 |
|--------------|---------------------------------------------------------|
| `scu_echo`   | Verification ping against a configured peer.            |
| `scu_find`   | C-FIND against a peer at the chosen query level.        |
| `scu_move`   | Ask a peer to forward matching instances elsewhere.     |
| `scu_store`  | Send one or more local DICOM files to a peer.           |

Every tool input is declared as JSON Schema, so any spec-compliant MCP
client sees a typed surface.

## Command-line driver

`nightowl-cli` mirrors the MCP tool set as a local command-line driver.
It opens the same SQLite databases and `config.json` the desktop app
uses (SQLite runs in WAL mode, so concurrent access from the app and
the CLI is safe) and calls the same business-logic functions
in-process. There is no IPC step.

### Global flags

- `--data-dir <path>` — Override the data directory. Defaults to the
  platform's app config directory for the NightOwl bundle (the same
  path the desktop app uses).
- `--json` — Emit results as pretty-printed JSON, matching the shape
  the MCP server returns. Without it, results are human-readable.
- `-v` / `--verbose` — Raise log verbosity. `-v` for info, `-vv` for
  debug. Default is errors only.

### Exit codes

- `0` — success.
- `1` — runtime failure (IO, database, DICOM).
- `2` — validation failure (bad arguments, unknown peer id).

### Top-level commands

| Command                | Effect                                              |
|------------------------|-----------------------------------------------------|
| `config show`          | Print the effective NightOwl configuration.         |
| `peers list`           | List configured remote DICOM peers.                 |
| `studies list`         | List studies in the local store.                    |
| `studies series <uid>` | List series for a study.                            |
| `series instances <uid>` | List instances for a series.                      |
| `instances count`      | Count instances across the store.                   |
| `store rescan`         | Trigger a rescan of the store directory.            |
| `worklist list`        | List scheduled procedure step entries.              |
| `activity list`        | Read the activity log (filters available).          |
| `activity count`       | Count rows in the activity log.                     |
| `scu echo <peer-id>`   | Verification ping against a peer.                   |
| `scu find …`           | C-FIND against a peer.                              |
| `scu move …`           | C-MOVE against a peer.                              |
| `scu store …`          | C-STORE one or more local DICOM files to a peer.    |

Run `nightowl-cli --help` or `nightowl-cli <command> --help` for the
full flag set on each subcommand.

### Examples

Ping a peer by id:

```
nightowl-cli scu echo my-pacs
```

List studies as JSON for piping into `jq`:

```
nightowl-cli --json studies list
```

Search the activity log for outbound C-STORE attempts in the last hour:

```
nightowl-cli activity list --direction outbound --command C-STORE
```

Send a directory of DICOM files to a peer:

```
nightowl-cli scu store --peer my-pacs --path ~/scans/study-001
```

## Where state lives

All persistent state lives in the platform-specific app config
directory:

- **macOS** — `~/Library/Application Support/cloud.aurabox.nightowl/`
- **Linux** — `~/.config/cloud.aurabox.nightowl/` (or
  `$XDG_CONFIG_HOME` if set)
- **Windows** — `%APPDATA%\cloud.aurabox.nightowl\`

| File              | Purpose                                              |
|-------------------|------------------------------------------------------|
| `config.json`     | AE Title, listen port, store directory, MCP block.   |
| `peers.json`      | List of remote DICOM peers.                          |
| `store.sqlite`    | SOP Instance index and activity log.                 |
| `worklist.sqlite` | Modality Worklist scheduled procedure step entries.  |

The store directory itself defaults to `~/dicom-store`. NightOwl
scans, writes to, and answers Q/R requests from that directory.

## Limitations

- No TLS and no authentication on the DICOM listener or the MCP
  server. The SCP listens on every interface; use only on a trusted
  local network or behind a firewall.
- Not a clinical product. NightOwl makes no regulatory or compliance
  claims.
- C-GET as SCU is not implemented. C-GET as SCP is.
- Storage SOP classes are limited to the set listed above.

## License

See [LICENSE](LICENSE).
