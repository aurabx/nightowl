import { useEffect, useMemo, useState } from "react";
import { Check, Copy } from "lucide-react";
import {
  type AppConfig,
  type McpStatus,
  formatError,
  getConfig,
  isAppError,
  mcpStatus,
  saveConfig,
} from "../lib/api";
import { Field } from "../components/Field";

const INPUT_CLASS =
  "w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm " +
  "text-slate-100 focus:border-sky-500 focus:outline-none";

/**
 * Builds the cross-client `mcpServers` snippet for the given port. This
 * format is accepted by Claude Desktop, Claude Code (`~/.claude.json`),
 * Cursor and the other modern MCP clients that talk to remote HTTP
 * servers. The `type: "http"` discriminator selects the streamable-HTTP
 * transport so the client does not try to spawn NightOwl as a
 * subprocess.
 */
function buildMcpConfigSnippet(port: number): string {
  return JSON.stringify(
    {
      mcpServers: {
        nightowl: {
          type: "http",
          url: `http://127.0.0.1:${port}/mcp`,
        },
      },
    },
    null,
    2,
  );
}

/**
 * Builds the `claude mcp add` one-liner for Claude Code users who prefer
 * the CLI over hand-editing `~/.claude.json`. Idempotent: re-running
 * `claude mcp add` with the same name updates the existing entry.
 */
function buildMcpCliCommand(port: number): string {
  return `claude mcp add --transport http nightowl http://127.0.0.1:${port}/mcp`;
}

/**
 * The MCP page is its own tab so the user can manage the local Model
 * Context Protocol server independently of the DICOM listener settings.
 * It owns the `mcp` block of `AppConfig` exclusively — the Settings
 * page does not modify it. Both pages call `saveConfig` with the full
 * config, but because they fetch a fresh copy on mount and the router
 * shows only one page at a time, there is no clobbering: when this
 * page saves, the non-mcp fields it sends back are whatever was on
 * disk at the moment this page mounted (which is also what is in
 * `AppState.config` since no other writer exists).
 */
export function McpPage() {
  const [loaded, setLoaded] = useState<AppConfig | null>(null);
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [savedAt, setSavedAt] = useState<number | null>(null);
  // Transient "Copied!" indicators on the two copy buttons.
  const [mcpCopiedAt, setMcpCopiedAt] = useState<number | null>(null);
  const [mcpCopyError, setMcpCopyError] = useState<string | null>(null);
  const [mcpCliCopiedAt, setMcpCliCopiedAt] = useState<number | null>(null);
  const [mcpCliCopyError, setMcpCliCopyError] = useState<string | null>(null);
  // Live MCP runtime status. Refetched after save so the badge
  // reflects hot-reload outcomes without requiring an app restart.
  const [runtimeStatus, setRuntimeStatus] = useState<McpStatus | null>(null);

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        setLoaded(cfg);
        setDraft(cfg);
      })
      .catch((err) => setLoadError(formatError(err)));
    mcpStatus()
      .then(setRuntimeStatus)
      .catch(() => {
        // Read-only IPC; failure here means the backend rejected the
        // call entirely (unusual). Silently swallow and leave the
        // badge hidden.
      });
  }, []);

  const dirty = useMemo(() => {
    if (!loaded || !draft) return false;
    return (
      loaded.mcp.enabled !== draft.mcp.enabled ||
      loaded.mcp.port !== draft.mcp.port
    );
  }, [loaded, draft]);

  const handleSave = async () => {
    if (!draft) return;
    setSaving(true);
    setSaveError(null);
    setFieldErrors({});
    try {
      const saved = await saveConfig(draft);
      setLoaded(saved);
      setDraft(saved);
      setSavedAt(Date.now());
      try {
        const status = await mcpStatus();
        setRuntimeStatus(status);
      } catch {
        // Swallowed — see the on-mount fetch.
      }
    } catch (err: unknown) {
      if (isAppError(err) && err.kind === "Validation") {
        setFieldErrors({ [err.message.field]: err.message.reason });
      } else {
        setSaveError(formatError(err));
      }
    } finally {
      setSaving(false);
    }
  };

  const handleReset = () => {
    if (!loaded) return;
    setDraft(loaded);
    setFieldErrors({});
    setSaveError(null);
  };

  // Snippets reflect the SAVED port, not the in-flight draft. Copying a
  // snippet built from an unsaved port would point the consuming app at
  // a port NightOwl is not actually listening on.
  const mcpConfigSnippet = useMemo(
    () => (loaded ? buildMcpConfigSnippet(loaded.mcp.port) : ""),
    [loaded],
  );
  const mcpCliCommand = useMemo(
    () => (loaded ? buildMcpCliCommand(loaded.mcp.port) : ""),
    [loaded],
  );

  const copyToClipboard = async (
    body: string,
    onSuccess: (ts: number) => void,
    onError: (msg: string) => void,
  ) => {
    if (!body) return;
    onError("");
    try {
      await navigator.clipboard.writeText(body);
      onSuccess(Date.now());
    } catch (err: unknown) {
      onError(err instanceof Error ? err.message : "Clipboard write failed.");
    }
  };

  const handleCopyMcpConfig = () =>
    copyToClipboard(
      mcpConfigSnippet,
      (ts) => {
        setMcpCopyError(null);
        setMcpCopiedAt(ts);
        window.setTimeout(() => setMcpCopiedAt(null), 2000);
      },
      setMcpCopyError,
    );

  const handleCopyMcpCli = () =>
    copyToClipboard(
      mcpCliCommand,
      (ts) => {
        setMcpCliCopyError(null);
        setMcpCliCopiedAt(ts);
        window.setTimeout(() => setMcpCliCopiedAt(null), 2000);
      },
      setMcpCliCopyError,
    );

  if (loadError) {
    return (
      <section className="max-w-xl">
        <h1 className="text-2xl font-semibold">MCP</h1>
        <p className="mt-4 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          Failed to load config: {loadError}
        </p>
      </section>
    );
  }

  if (!draft) {
    return (
      <section className="max-w-xl">
        <h1 className="text-2xl font-semibold">MCP</h1>
        <p className="mt-4 text-sm text-slate-500">Loading…</p>
      </section>
    );
  }

  return (
    <section className="max-w-xl">
      <div className="flex items-center gap-3">
        <h1 className="text-2xl font-semibold">MCP</h1>
        {runtimeStatus && <McpStatusBadge status={runtimeStatus} />}
      </div>
      <p className="mt-1 text-sm text-slate-400">
        Local Model Context Protocol server. When enabled, NightOwl binds an
        MCP server on <code className="rounded bg-slate-800 px-1 text-xs">127.0.0.1</code>
        {" "}so LLM clients (Claude Code, Claude Desktop, Cursor) can drive
        NightOwl's read and SCU operations as MCP tools. Loopback only — no
        authentication. Changes apply on save (no restart required).
      </p>
      {runtimeStatus?.state === "failed" && (
        <p className="mt-2 rounded border border-red-700/40 bg-red-900/20 p-2 text-xs text-red-300">
          MCP server failed to start: {runtimeStatus.reason}
        </p>
      )}

      <div className="mt-6 space-y-5">
        <label className="flex items-center gap-2 text-sm text-slate-200">
          <input
            type="checkbox"
            checked={draft.mcp.enabled}
            onChange={(e) =>
              setDraft({
                ...draft,
                mcp: { ...draft.mcp, enabled: e.target.checked },
              })
            }
          />
          Enable MCP server
        </label>

        <Field
          label="MCP port"
          hint="TCP port on 127.0.0.1 the MCP server listens on. Must differ from the DICOM listen port. Default 7300."
          error={fieldErrors["mcp.port"]}
        >
          <input
            type="number"
            className={INPUT_CLASS}
            value={draft.mcp.port}
            min={1024}
            max={65535}
            disabled={!draft.mcp.enabled}
            onChange={(e) =>
              setDraft({
                ...draft,
                mcp: {
                  ...draft.mcp,
                  port: Number.parseInt(e.target.value, 10) || 0,
                },
              })
            }
          />
        </Field>
      </div>

      <div className="mt-6 flex items-center gap-3">
        <button
          type="button"
          onClick={handleSave}
          disabled={!dirty || saving}
          className={
            "rounded bg-sky-600 px-4 py-1.5 text-sm font-medium text-white " +
            "hover:bg-sky-500 disabled:bg-slate-700 disabled:text-slate-500 " +
            "disabled:cursor-not-allowed"
          }
        >
          {saving ? "Saving…" : "Save"}
        </button>
        <button
          type="button"
          onClick={handleReset}
          disabled={!dirty || saving}
          className={
            "rounded border border-slate-700 px-4 py-1.5 text-sm text-slate-300 " +
            "hover:border-slate-500 disabled:opacity-50 disabled:cursor-not-allowed"
          }
        >
          Revert
        </button>
        {savedAt && !dirty && (
          <span className="text-xs text-emerald-400">Saved.</span>
        )}
      </div>

      {saveError && (
        <p className="mt-4 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          {saveError}
        </p>
      )}

      {loaded && (
        <div className="mt-10 space-y-6">
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-xs uppercase tracking-wide text-slate-400">
                Client configuration snippet
              </span>
              <button
                type="button"
                onClick={handleCopyMcpConfig}
                aria-label="Copy MCP configuration JSON to clipboard"
                className={
                  "inline-flex items-center gap-1.5 rounded border border-slate-700 " +
                  "bg-slate-800 px-2.5 py-1 text-xs text-slate-200 " +
                  "hover:border-slate-500 hover:bg-slate-700"
                }
              >
                {mcpCopiedAt ? (
                  <>
                    <Check className="size-3.5 text-emerald-400" />
                    Copied
                  </>
                ) : (
                  <>
                    <Copy className="size-3.5" />
                    Copy
                  </>
                )}
              </button>
            </div>
            <pre
              className={
                "overflow-x-auto rounded border border-slate-700 bg-slate-950 " +
                "p-3 text-xs text-slate-200"
              }
            >
              {mcpConfigSnippet}
            </pre>
            <p className="text-xs text-slate-500">
              Paste this <code>mcpServers</code> block into your MCP client's
              config (Claude Desktop, Claude Code <code>~/.claude.json</code>,
              Cursor, etc.). The snippet uses the currently saved port — save
              changes before copying if you have edited the port.
              {!loaded.mcp.enabled && (
                <>
                  {" "}
                  <span className="text-amber-300">
                    Server is currently disabled; enable and save before
                    connecting a client.
                  </span>
                </>
              )}
            </p>
            {mcpCopyError && (
              <p className="text-xs text-red-400">
                Could not copy: {mcpCopyError}. Select the text above and
                copy manually.
              </p>
            )}
          </div>

          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-xs uppercase tracking-wide text-slate-400">
                Claude Code CLI
              </span>
              <button
                type="button"
                onClick={handleCopyMcpCli}
                aria-label="Copy claude mcp add command to clipboard"
                className={
                  "inline-flex items-center gap-1.5 rounded border border-slate-700 " +
                  "bg-slate-800 px-2.5 py-1 text-xs text-slate-200 " +
                  "hover:border-slate-500 hover:bg-slate-700"
                }
              >
                {mcpCliCopiedAt ? (
                  <>
                    <Check className="size-3.5 text-emerald-400" />
                    Copied
                  </>
                ) : (
                  <>
                    <Copy className="size-3.5" />
                    Copy
                  </>
                )}
              </button>
            </div>
            <pre
              className={
                "overflow-x-auto rounded border border-slate-700 bg-slate-950 " +
                "p-3 text-xs text-slate-200"
              }
            >
              {mcpCliCommand}
            </pre>
            <p className="text-xs text-slate-500">
              Run this in a terminal to register the server with Claude Code.
              Idempotent — re-running it updates the existing entry.
            </p>
            {mcpCliCopyError && (
              <p className="text-xs text-red-400">
                Could not copy: {mcpCliCopyError}. Select the text above and
                copy manually.
              </p>
            )}
          </div>
        </div>
      )}
    </section>
  );
}

/**
 * Small pill that reports whether the MCP server is currently bound,
 * disabled, or failed to start. Reflects the live runtime — the page
 * re-fetches after a successful save so hot-reload outcomes are
 * visible immediately.
 */
function McpStatusBadge({ status }: { status: McpStatus }) {
  if (status.state === "running") {
    return (
      <span
        className="inline-flex items-center gap-1.5 rounded-full border border-emerald-700/40 bg-emerald-900/30 px-2 py-0.5 text-xs text-emerald-300"
        title={`Listening on ${status.bind_addr}`}
      >
        <span className="size-1.5 rounded-full bg-emerald-400" />
        Running on {status.bind_addr}
      </span>
    );
  }
  if (status.state === "failed") {
    return (
      <span
        className="inline-flex items-center gap-1.5 rounded-full border border-red-700/40 bg-red-900/30 px-2 py-0.5 text-xs text-red-300"
        title={status.reason}
      >
        <span className="size-1.5 rounded-full bg-red-400" />
        Failed
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full border border-slate-700 bg-slate-800 px-2 py-0.5 text-xs text-slate-400">
      <span className="size-1.5 rounded-full bg-slate-500" />
      Disabled
    </span>
  );
}
