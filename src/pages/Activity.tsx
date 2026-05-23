import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownLeft,
  ArrowUpRight,
  Circle,
  Eraser,
  Info,
  Pause,
  Play,
  RotateCw,
  Search,
  TriangleAlert,
} from "lucide-react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type ActivityDirection,
  type ActivityEvent,
  type ActivityFilter,
  type ActivityStatus,
  clearActivity,
  formatError,
  listActivity,
} from "../lib/api";
import { Select } from "../components/Select";
import { Pagination } from "../components/Pagination";

const PAGE_SIZE = 50;

function formatTime(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  const fff = String(d.getMilliseconds()).padStart(3, "0");
  return `${hh}:${mm}:${ss}.${fff}`;
}

function DirectionGlyph({ direction }: { direction: ActivityDirection }) {
  if (direction === "inbound") {
    return <ArrowDownLeft className="size-3.5 text-sky-400" aria-label="inbound" />;
  }
  if (direction === "outbound") {
    return <ArrowUpRight className="size-3.5 text-emerald-400" aria-label="outbound" />;
  }
  return <Info className="size-3.5 text-slate-500" aria-label="info" />;
}

const STATUS_DOT: Record<ActivityStatus, string> = {
  info: "text-slate-500 fill-slate-500",
  success: "text-emerald-400 fill-emerald-400",
  warning: "text-amber-400 fill-amber-400",
  error: "text-red-400 fill-red-400",
};

const ASSOCIATION_PALETTE = [
  "text-sky-300",
  "text-emerald-300",
  "text-amber-300",
  "text-fuchsia-300",
  "text-cyan-300",
  "text-rose-300",
  "text-violet-300",
  "text-lime-300",
];

function colourForAssociation(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    h = (h * 31 + id.charCodeAt(i)) | 0;
  }
  return ASSOCIATION_PALETTE[Math.abs(h) % ASSOCIATION_PALETTE.length];
}

export function ActivityPage() {
  const [events, setEvents] = useState<ActivityEvent[]>([]);
  const [total, setTotal] = useState<number>(0);
  const [page, setPage] = useState<number>(0);
  const [paused, setPaused] = useState<boolean>(false);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);

  // Number of new live events that arrived while the user is NOT on
  // page 1. We display a "N new events" banner; clicking it jumps to
  // page 1 (which triggers a refetch).
  const [newSincePage1, setNewSincePage1] = useState<number>(0);

  // Two-click confirmation for Clear log. `window.confirm()` is
  // unreliable inside Tauri's WKWebView (returns undefined without
  // showing a dialog on macOS), so we morph the button into its own
  // confirmation prompt for a few seconds.
  const [confirmingClear, setConfirmingClear] = useState<boolean>(false);
  const confirmTimer = useRef<number | null>(null);

  const [direction, setDirection] = useState<ActivityDirection | "">("");
  const [status, setStatus] = useState<ActivityStatus | "">("");
  const [search, setSearch] = useState<string>("");

  // Mirror state into refs so the event listener (registered once)
  // reads the latest values without being torn down on every change.
  const pausedRef = useRef(paused);
  const pageRef = useRef(page);
  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);
  useEffect(() => {
    pageRef.current = page;
  }, [page]);

  const filter: ActivityFilter = useMemo(
    () => ({
      direction: direction || undefined,
      status: status || undefined,
      search: search.trim() || undefined,
      limit: PAGE_SIZE,
      offset: page * PAGE_SIZE,
    }),
    [direction, status, search, page],
  );

  const fetchPage = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await listActivity(filter);
      setEvents(result.events);
      setTotal(result.total);
      if (page === 0) setNewSincePage1(0);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setLoading(false);
    }
  }, [filter, page]);

  // Reset to page 0 whenever a filter changes. Without this you could
  // be sitting on (e.g.) page 5 of one filter and silently see a
  // different filter's page 5.
  useEffect(() => {
    setPage(0);
  }, [direction, status, search]);

  useEffect(() => {
    fetchPage();
  }, [fetchPage]);

  // Subscribe once to the live `activity` stream. On page 0 we splice
  // the new event into the visible list (and bump total); on later
  // pages we just count it for the "N new events" banner.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ActivityEvent>("activity", (e) => {
      if (pausedRef.current) return;
      const onFirstPage = pageRef.current === 0;
      if (onFirstPage) {
        setEvents((prev) => {
          if (prev.length && prev[0].id === e.payload.id) return prev;
          const next = [e.payload, ...prev];
          // Keep the page size honoured — we'd otherwise grow without
          // bound when many events arrive between fetches.
          return next.length > PAGE_SIZE ? next.slice(0, PAGE_SIZE) : next;
        });
        setTotal((n) => n + 1);
      } else {
        setNewSincePage1((n) => n + 1);
        setTotal((n) => n + 1);
      }
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((err) => setError(formatError(err)));
    return () => {
      unlisten?.();
    };
  }, []);

  const handleClear = async () => {
    // First click: arm the confirmation. Second click within 4 seconds:
    // actually clear. Any other click or mouse-out resets via the
    // timer.
    if (!confirmingClear) {
      setConfirmingClear(true);
      if (confirmTimer.current !== null) {
        window.clearTimeout(confirmTimer.current);
      }
      confirmTimer.current = window.setTimeout(() => {
        setConfirmingClear(false);
        confirmTimer.current = null;
      }, 4000);
      return;
    }
    if (confirmTimer.current !== null) {
      window.clearTimeout(confirmTimer.current);
      confirmTimer.current = null;
    }
    setConfirmingClear(false);
    try {
      await clearActivity();
      setEvents([]);
      setTotal(0);
      setPage(0);
      setNewSincePage1(0);
    } catch (err) {
      setError(formatError(err));
    }
  };

  // Cancel any pending confirmation timer on unmount so we don't fire
  // a state update against a dead component.
  useEffect(() => {
    return () => {
      if (confirmTimer.current !== null) {
        window.clearTimeout(confirmTimer.current);
        confirmTimer.current = null;
      }
    };
  }, []);

  const handleJumpToLatest = () => {
    if (page === 0) {
      fetchPage();
    } else {
      setPage(0);
    }
    setNewSincePage1(0);
  };

  return (
    <section>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">Activity</h1>
          <p className="mt-1 text-sm text-slate-400">
            Live log of every DICOM association and DIMSE message. Persisted to
            <code className="mx-1 rounded bg-slate-800 px-1.5 py-0.5 text-xs">
              store.sqlite
            </code>
            and capped at 50,000 rows.
          </p>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => setPaused((p) => !p)}
            className={
              "flex items-center gap-1.5 rounded border px-3 py-1.5 text-sm " +
              (paused
                ? "border-amber-700/50 bg-amber-900/20 text-amber-200 hover:bg-amber-900/30"
                : "border-slate-700 text-slate-300 hover:border-slate-500")
            }
          >
            {paused ? <Play className="size-3.5" /> : <Pause className="size-3.5" />}
            {paused ? "Resume" : "Pause"}
          </button>
          <button
            type="button"
            onClick={fetchPage}
            disabled={loading}
            className="flex items-center gap-1.5 rounded border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-500 disabled:opacity-50"
          >
            <RotateCw className={"size-3.5" + (loading ? " animate-spin" : "")} />
            Refresh
          </button>
          <button
            type="button"
            onClick={handleClear}
            className={
              "flex items-center gap-1.5 rounded border px-3 py-1.5 text-sm " +
              (confirmingClear
                ? "border-red-500 bg-red-600 text-white hover:bg-red-500"
                : "border-red-800/50 bg-red-900/20 text-red-200 hover:bg-red-900/30")
            }
          >
            {confirmingClear ? (
              <TriangleAlert className="size-3.5" />
            ) : (
              <Eraser className="size-3.5" />
            )}
            {confirmingClear ? "Click again to confirm" : "Clear log"}
          </button>
        </div>
      </div>

      <div className="mt-4 flex flex-wrap items-center gap-2 text-sm">
        <Select
          value={direction}
          onChange={(e) => setDirection(e.target.value as ActivityDirection | "")}
          className="w-40"
        >
          <option value="">All directions</option>
          <option value="inbound">Inbound</option>
          <option value="outbound">Outbound</option>
          <option value="info">Info</option>
        </Select>
        <Select
          value={status}
          onChange={(e) => setStatus(e.target.value as ActivityStatus | "")}
          className="w-36"
        >
          <option value="">All statuses</option>
          <option value="info">Info</option>
          <option value="success">Success</option>
          <option value="warning">Warning</option>
          <option value="error">Error</option>
        </Select>
        <div className="relative flex-1 min-w-[12rem] max-w-md">
          <Search className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-slate-500" />
          <input
            type="text"
            placeholder="Search message or peer…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full rounded border border-slate-700 bg-slate-900 py-1 pl-7 pr-2 text-sm text-slate-200 focus:border-sky-500 focus:outline-none"
          />
        </div>
        {paused && (
          <span className="ml-auto text-xs text-amber-300">
            paused — new events skipped
          </span>
        )}
      </div>

      {newSincePage1 > 0 && page !== 0 && (
        <button
          type="button"
          onClick={handleJumpToLatest}
          className="mt-3 w-full rounded border border-sky-700/50 bg-sky-900/20 px-3 py-1.5 text-sm text-sky-200 hover:bg-sky-900/30"
        >
          {newSincePage1} new event{newSincePage1 === 1 ? "" : "s"} — jump to latest
        </button>
      )}

      {error && (
        <p className="mt-3 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          {error}
        </p>
      )}

      <div className="mt-4 mb-2">
        <Pagination
          total={total}
          pageSize={PAGE_SIZE}
          page={page}
          onPageChange={setPage}
        />
      </div>

      <div className="overflow-x-auto rounded border border-slate-800 bg-slate-900/30">
        <table className="w-full text-sm">
          <thead className="border-b border-slate-800 bg-slate-900/60 text-xs uppercase tracking-wide text-slate-500">
            <tr>
              <th className="px-3 py-2 text-left font-medium w-28">Time</th>
              <th className="px-2 py-2 text-left font-medium w-6"></th>
              <th className="px-3 py-2 text-left font-medium w-40">Peer</th>
              <th className="px-3 py-2 text-left font-medium w-28">Command</th>
              <th className="px-2 py-2 text-left font-medium w-3"></th>
              <th className="px-3 py-2 text-left font-medium">Message</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-800/60">
            {events.length === 0 ? (
              <tr>
                <td colSpan={6} className="px-3 py-8 text-center text-sm text-slate-500">
                  {loading
                    ? "Loading…"
                    : "No activity yet. Run an SCP or SCU operation and events will appear here."}
                </td>
              </tr>
            ) : (
              events.map((e) => (
                <tr
                  key={e.id}
                  className="hover:bg-slate-800/30"
                  title={`association ${e.association_id}`}
                >
                  <td className="px-3 py-1.5 font-mono text-xs text-slate-400">
                    {formatTime(e.timestamp_ms)}
                  </td>
                  <td className="px-2 py-1.5">
                    <DirectionGlyph direction={e.direction} />
                  </td>
                  <td className="px-3 py-1.5 text-slate-300">
                    {e.peer_ae_title ? (
                      <span className={colourForAssociation(e.association_id)}>
                        {e.peer_ae_title}
                      </span>
                    ) : (
                      <span className="text-slate-600">—</span>
                    )}
                  </td>
                  <td className="px-3 py-1.5">
                    {e.command ? (
                      <span className="rounded bg-slate-800 px-1.5 py-0.5 font-mono text-xs text-slate-300">
                        {e.command}
                      </span>
                    ) : (
                      <span className="text-slate-600">—</span>
                    )}
                  </td>
                  <td className="px-2 py-1.5">
                    <Circle className={`size-2 ${STATUS_DOT[e.status]}`} />
                  </td>
                  <td className="px-3 py-1.5 text-slate-200">{e.message}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

    </section>
  );
}
