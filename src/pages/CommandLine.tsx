import { useEffect, useState } from "react";
import { Terminal, Check, AlertTriangle, RefreshCw } from "lucide-react";
import {
  type CliInstallStatus,
  cliInstall,
  cliInstallStatus,
  cliUninstall,
  formatError,
} from "../lib/api";

/**
 * Command Line page.
 *
 * Installs a `nightowl-cli` entry on the user's `$PATH` so the CLI is
 * invokable from any shell. The desktop binary doubles as the CLI: the
 * backend (`core::cli_install`) symlinks a directory on `$PATH` to this
 * same binary, which dispatches to the CLI surface when invoked under
 * that name. This component renders the status the backend reports and
 * dispatches the three Tauri commands — it owns no install logic itself.
 */

type Busy = "loading" | "idle" | "installing" | "uninstalling";

export function CommandLinePage() {
  const [status, setStatus] = useState<CliInstallStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<Busy>("loading");
  const [lastMessage, setLastMessage] = useState<string | null>(null);

  const refresh = async () => {
    setBusy("loading");
    setError(null);
    try {
      setStatus(await cliInstallStatus());
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy("idle");
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const install = async () => {
    setBusy("installing");
    setError(null);
    setLastMessage(null);
    try {
      const path = await cliInstall();
      setLastMessage(`Installed at ${path}`);
      await refresh();
    } catch (e) {
      setError(formatError(e));
      setBusy("idle");
    }
  };

  const uninstall = async () => {
    setBusy("uninstalling");
    setError(null);
    setLastMessage(null);
    try {
      setLastMessage(await cliUninstall());
      await refresh();
    } catch (e) {
      setError(formatError(e));
      setBusy("idle");
    }
  };

  return (
    <div className="max-w-2xl">
      <h1 className="text-xl font-semibold mb-1">Command Line</h1>
      <p className="text-sm text-slate-400 mb-6">
        Install the{" "}
        <code className="rounded bg-slate-800 px-1 py-0.5 text-xs">
          nightowl-cli
        </code>{" "}
        command so you can drive NightOwl from any terminal. The same
        desktop binary powers the app, the MCP server, and the CLI — no
        separate download.
      </p>

      {error && (
        <div className="mb-6 rounded border border-rose-500/40 bg-rose-500/10 p-3 text-xs text-rose-300">
          {error}
        </div>
      )}

      <div className="rounded-lg border border-slate-800 bg-slate-900 overflow-hidden">
        <div className="flex items-start gap-3 p-5">
          <Terminal className="mt-0.5 size-5 shrink-0 text-slate-500" />
          <div className="min-w-0 flex-1">
            <StatusLine status={status} busy={busy} />
            {status && (
              <div className="mt-3 space-y-1.5 text-xs text-slate-400">
                <DetailRow label="Binary" value={status.binary_path} />
                {status.install_path && (
                  <DetailRow label="Install path" value={status.install_path} />
                )}
                <DetailRow label="Platform" value={status.platform} />
              </div>
            )}
            {status?.path_hint && (
              <div className="mt-4 flex gap-2 rounded border border-amber-500/30 bg-amber-500/5 p-3 text-xs text-slate-200">
                <AlertTriangle className="mt-0.5 size-3.5 shrink-0 text-amber-400" />
                <span className="leading-relaxed">{status.path_hint}</span>
              </div>
            )}
            {lastMessage && (
              <div className="mt-4 flex gap-2 rounded border border-emerald-500/30 bg-emerald-500/5 p-3 text-xs text-slate-200">
                <Check className="mt-0.5 size-3.5 shrink-0 text-emerald-400" />
                <span className="break-all">{lastMessage}</span>
              </div>
            )}
          </div>
        </div>

        <div className="flex items-center gap-2 border-t border-slate-800 bg-slate-900/60 px-5 py-3">
          <ActionButtons
            status={status}
            busy={busy}
            onInstall={install}
            onUninstall={uninstall}
            onRefresh={refresh}
          />
        </div>
      </div>

      <h2 className="mt-8 mb-2 text-sm font-medium">Usage</h2>
      <p className="mb-3 text-sm text-slate-400">
        Once installed, run{" "}
        <code className="rounded bg-slate-800 px-1 py-0.5 text-xs">
          nightowl-cli --help
        </code>{" "}
        to see every command. A few examples:
      </p>
      <pre className="overflow-x-auto rounded-lg border border-slate-800 bg-slate-900 p-4 text-xs text-slate-200">
        {`nightowl-cli scu echo my-pacs
nightowl-cli --json studies list
nightowl-cli activity list --direction outbound --command C-STORE
nightowl-cli inspect file ~/scans/IM0001.dcm`}
      </pre>
    </div>
  );
}

function StatusLine({
  status,
  busy,
}: {
  status: CliInstallStatus | null;
  busy: Busy;
}) {
  if (busy === "loading" || !status) {
    return (
      <div className="text-sm text-slate-400">Checking install status…</div>
    );
  }
  const label =
    status.status === "installed"
      ? "Installed"
      : status.status === "stale"
        ? "Stale — a nightowl-cli on your PATH points to a different binary"
        : status.status === "unsupported"
          ? "Not available on this platform yet"
          : "Not installed";
  const tone =
    status.status === "installed"
      ? "text-emerald-400"
      : status.status === "stale"
        ? "text-amber-400"
        : status.status === "unsupported"
          ? "text-slate-400"
          : "text-slate-100";
  return <div className={`text-sm font-medium ${tone}`}>{label}</div>;
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex gap-2">
      <span className="w-24 shrink-0 text-slate-500">{label}</span>
      <span className="break-all font-mono text-[11.5px] text-slate-300">
        {value}
      </span>
    </div>
  );
}

function ActionButtons({
  status,
  busy,
  onInstall,
  onUninstall,
  onRefresh,
}: {
  status: CliInstallStatus | null;
  busy: Busy;
  onInstall: () => void;
  onUninstall: () => void;
  onRefresh: () => void;
}) {
  if (!status || status.status === "unsupported") {
    return (
      <span className="text-xs text-slate-500">
        Installing from the app is not yet available on this platform. Use
        the standalone <code className="text-slate-400">nightowl-cli</code>{" "}
        binary instead.
      </span>
    );
  }

  const installLabel =
    status.status === "installed"
      ? "Reinstall"
      : status.status === "stale"
        ? "Replace"
        : "Install";

  return (
    <>
      <button
        type="button"
        onClick={onInstall}
        disabled={busy !== "idle"}
        className="h-7 rounded-md bg-sky-600 px-3 text-xs text-white hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {busy === "installing" ? "Installing…" : installLabel}
      </button>
      {(status.status === "installed" || status.status === "stale") && (
        <button
          type="button"
          onClick={onUninstall}
          disabled={busy !== "idle"}
          className="h-7 rounded-md border border-slate-700 bg-slate-800 px-3 text-xs text-slate-200 hover:border-slate-600 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {busy === "uninstalling" ? "Removing…" : "Uninstall"}
        </button>
      )}
      <button
        type="button"
        onClick={onRefresh}
        disabled={busy !== "idle"}
        aria-label="Re-check install status"
        title="Re-check install status"
        className="flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs text-slate-400 hover:bg-slate-800 hover:text-slate-200 disabled:cursor-not-allowed disabled:opacity-50"
      >
        <RefreshCw className="size-3" />
      </button>
    </>
  );
}
