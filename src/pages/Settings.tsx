import { useEffect, useMemo, useState } from "react";
import { ShieldAlert } from "lucide-react";
import {
  type AppConfig,
  formatError,
  getConfig,
  isAppError,
  saveConfig,
} from "../lib/api";
import { Field } from "../components/Field";

const INPUT_CLASS =
  "w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm " +
  "text-slate-100 focus:border-sky-500 focus:outline-none";

/**
 * Settings owns the DICOM identity fields (local AE Title, listen port,
 * store directory). The MCP server lives on its own tab (`McpPage`) and
 * is responsible for the `mcp` block of `AppConfig`. This page never
 * mutates `draft.mcp` — it copies whatever was loaded from disk back
 * unchanged on save, so a `saveConfig` from here cannot accidentally
 * disable the MCP server or revert its port.
 */
export function SettingsPage() {
  const [loaded, setLoaded] = useState<AppConfig | null>(null);
  const [draft, setDraft] = useState<AppConfig | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [savedAt, setSavedAt] = useState<number | null>(null);

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
      loaded.store_dir !== draft.store_dir
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
