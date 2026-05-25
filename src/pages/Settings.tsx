import { useEffect, useMemo, useState } from "react";
import { Check, Copy, ShieldAlert } from "lucide-react";
import {
  type AppConfig,
  formatError,
  getConfig,
  isAppError,
  saveConfig,
} from "../lib/api";
import { Field } from "../components/Field";

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

const INPUT_CLASS =
  "w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm " +
  "text-slate-100 focus:border-sky-500 focus:outline-none";

export function SettingsPage() {
  const [loaded, setLoaded] = useState<AppConfig | null>(null);
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [savedAt, setSavedAt] = useState<number | null>(null);
  // Transient "Copied!" indicator on the MCP config copy button. Reset
  // by a `setTimeout` two seconds after a successful copy. Cleared
  // synchronously if the user clicks again to copy a freshly-saved
  // snippet.
  const [mcpCopiedAt, setMcpCopiedAt] = useState<number | null>(null);
  const [mcpCopyError, setMcpCopyError] = useState<string | null>(null);

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        setLoaded(cfg);
        setDraft(cfg);
      })
      .catch((err) => setLoadError(formatError(err)));
  }, []);

  const dirty = useMemo(() => {
    if (!loaded || !draft) return false;
    return (
      loaded.local_ae_title !== draft.local_ae_title ||
      loaded.listen_port !== draft.listen_port ||
      loaded.store_dir !== draft.store_dir ||
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

  // Snippet reflects the SAVED port, not the in-flight draft. Copying a
  // snippet built from a port that has not been persisted would point
  // the other app at a port NightOwl is not actually listening on.
  const mcpConfigSnippet = useMemo(
    () => (loaded ? buildMcpConfigSnippet(loaded.mcp.port) : ""),
    [loaded],
  );

  const handleCopyMcpConfig = async () => {
    if (!mcpConfigSnippet) return;
    setMcpCopyError(null);
    try {
      await navigator.clipboard.writeText(mcpConfigSnippet);
      setMcpCopiedAt(Date.now());
      // Reset the "Copied!" badge after two seconds so a repeat copy
      // visibly re-fires.
      window.setTimeout(() => setMcpCopiedAt(null), 2000);
    } catch (err: unknown) {
      // The Tauri webview should grant clipboard write to the in-app
      // origin, but a future capability tightening could break this.
      // Surface the failure inline so the user can fall back to
      // selecting the snippet text and copying manually.
      setMcpCopyError(
        err instanceof Error ? err.message : "Clipboard write failed.",
      );
    }
  };

  if (loadError) {
    return (
      <section className="max-w-xl">
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="mt-4 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          Failed to load config: {loadError}
        </p>
      </section>
    );
  }

  if (!draft) {
    return (
      <section className="max-w-xl">
        <h1 className="text-2xl font-semibold">Settings</h1>
        <p className="mt-4 text-sm text-slate-500">Loading…</p>
      </section>
    );
  }

  return (
    <section className="max-w-xl">
      <h1 className="text-2xl font-semibold">Settings</h1>
      <p className="mt-1 text-sm text-slate-400">
        Local DICOM service identity and storage. Changes apply on save.
      </p>

      <div className="mt-6 space-y-5">
        <Field
          label="Local AE Title"
          hint="1–16 ASCII characters. The identifier other DICOM peers see when this app responds. Default: NIGHTOWL."
          error={fieldErrors.local_ae_title}
        >
          <input
            type="text"
            className={INPUT_CLASS}
            value={draft.local_ae_title}
            maxLength={16}
            spellCheck={false}
            onChange={(e) =>
              setDraft({ ...draft, local_ae_title: e.target.value })
            }
          />
        </Field>

        <Field
          label="Listen port"
          hint="TCP port for inbound DICOM associations. Use 1024 or higher (DICOM convention is 11112)."
          error={fieldErrors.listen_port}
        >
          <input
            type="number"
            className={INPUT_CLASS}
            value={draft.listen_port}
            min={1024}
            max={65535}
            onChange={(e) =>
              setDraft({
                ...draft,
                listen_port: Number.parseInt(e.target.value, 10) || 0,
              })
            }
          />
        </Field>

        <Field
          label="Store directory"
          hint="Absolute path on disk where received SOP Instances are written and indexed. Created on save if missing."
          error={fieldErrors.store_dir}
        >
          <input
            type="text"
            className={INPUT_CLASS}
            value={draft.store_dir}
            spellCheck={false}
            onChange={(e) => setDraft({ ...draft, store_dir: e.target.value })}
          />
        </Field>
      </div>

      <div className="mt-10">
        <h2 className="text-lg font-semibold">Local MCP server</h2>
        <p className="mt-1 text-sm text-slate-400">
          Optional. When enabled, NightOwl binds a Model Context Protocol server
          on <code className="rounded bg-slate-800 px-1 text-xs">127.0.0.1</code>
          {" "}so LLM clients (Claude Code, etc.) can drive NightOwl's read and
          SCU operations as MCP tools. Loopback only — no authentication.
          Restart NightOwl after changing these settings.
        </p>

        <div className="mt-4 space-y-5">
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

          {loaded && (
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
                      Server is currently disabled; enable, save, and restart
                      NightOwl before connecting a client.
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
          )}
        </div>
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

      <div className="mt-10 flex gap-3 rounded border border-amber-700/40 bg-amber-900/15 p-3 text-sm text-amber-200">
        <ShieldAlert className="mt-0.5 size-4 shrink-0" />
        <div>
          This iteration has no TLS and no authentication. The SCP listens on
          every interface; use only on a trusted local network or behind a
          firewall.
        </div>
      </div>
    </section>
  );
}
