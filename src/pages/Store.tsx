import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  Database,
  FileText,
  Folder,
  Loader2,
  RefreshCw,
} from "lucide-react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type InstanceRow,
  type ScanReport,
  type SeriesRow,
  type StudyRow,
  formatError,
  listInstancesForSeries,
  listSeriesForStudy,
  listStudies,
  rescanStore,
  totalInstanceCount,
} from "../lib/api";

const KB = 1024;
const MB = 1024 * KB;
const GB = 1024 * MB;

function humanBytes(n: number): string {
  if (n >= GB) return `${(n / GB).toFixed(2)} GB`;
  if (n >= MB) return `${(n / MB).toFixed(1)} MB`;
  if (n >= KB) return `${(n / KB).toFixed(0)} KB`;
  return `${n} B`;
}

function formatDicomDate(d: string | null): string {
  if (!d || d.length !== 8) return d ?? "";
  return `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}`;
}

export function StorePage() {
  const [studies, setStudies] = useState<StudyRow[]>([]);
  const [totalCount, setTotalCount] = useState<number>(0);
  const [loading, setLoading] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastScan, setLastScan] = useState<ScanReport | null>(null);
  const [expandedStudies, setExpandedStudies] = useState<Set<string>>(new Set());
  const [seriesByStudy, setSeriesByStudy] = useState<Record<string, SeriesRow[]>>({});
  const [expandedSeries, setExpandedSeries] = useState<Set<string>>(new Set());
  const [instancesBySeries, setInstancesBySeries] = useState<Record<string, InstanceRow[]>>({});

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [rows, count] = await Promise.all([listStudies(), totalInstanceCount()]);
      setStudies(rows);
      setTotalCount(count);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setLoading(false);
    }
  }, []);

  // Initial load.
  useEffect(() => {
    refresh();
  }, [refresh]);

  // The backend emits `store/scan-completed` after the initial background
  // scan finishes and after any manual rescan. Refresh the tree when we
  // see one so the user does not have to think about it.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ScanReport>("store/scan-completed", (event) => {
      setLastScan(event.payload);
      refresh();
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((err) => setError(formatError(err)));
    return () => {
      unlisten?.();
    };
  }, [refresh]);

  const handleRescan = async () => {
    setScanning(true);
    setError(null);
    try {
      const report = await rescanStore();
      setLastScan(report);
      // `rescan_store` also emits the event; the listener above will
      // refresh. But refresh defensively in case the listener has not
      // attached yet on a fast first paint.
      await refresh();
    } catch (err) {
      setError(formatError(err));
    } finally {
      setScanning(false);
    }
  };

  const toggleStudy = async (studyUid: string) => {
    const next = new Set(expandedStudies);
    if (next.has(studyUid)) {
      next.delete(studyUid);
      setExpandedStudies(next);
      return;
    }
    next.add(studyUid);
    setExpandedStudies(next);
    if (!seriesByStudy[studyUid]) {
      try {
        const series = await listSeriesForStudy(studyUid);
        setSeriesByStudy((prev) => ({ ...prev, [studyUid]: series }));
      } catch (err) {
        setError(formatError(err));
      }
    }
  };

  const toggleSeries = async (seriesUid: string) => {
    const next = new Set(expandedSeries);
    if (next.has(seriesUid)) {
      next.delete(seriesUid);
      setExpandedSeries(next);
      return;
    }
    next.add(seriesUid);
    setExpandedSeries(next);
    if (!instancesBySeries[seriesUid]) {
      try {
        const instances = await listInstancesForSeries(seriesUid);
        setInstancesBySeries((prev) => ({ ...prev, [seriesUid]: instances }));
      } catch (err) {
        setError(formatError(err));
      }
    }
  };

  const scanSummary = useMemo(() => {
    if (!lastScan) return null;
    const parts = [
      `${lastScan.files_seen} seen`,
      `${lastScan.files_inserted} new`,
      `${lastScan.files_updated} updated`,
      `${lastScan.files_skipped} skipped`,
    ];
    if (lastScan.files_errored > 0) parts.push(`${lastScan.files_errored} errored`);
    parts.push(`${lastScan.elapsed_ms} ms`);
    return parts.join(" · ");
  }, [lastScan]);

  return (
    <section className="max-w-4xl">
      <div className="flex items-start justify-between">
        <div>
          <h1 className="text-2xl font-semibold">Store</h1>
          <p className="mt-1 text-sm text-slate-400">
            SOP Instances in the configured store directory. The index is
            rebuilt on every rescan and on app start.
          </p>
        </div>
        <button
          type="button"
          onClick={handleRescan}
          disabled={scanning}
          className={
            "flex items-center gap-2 rounded bg-sky-600 px-3 py-1.5 text-sm font-medium text-white " +
            "hover:bg-sky-500 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed"
          }
        >
          {scanning ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <RefreshCw className="size-4" />
          )}
          {scanning ? "Scanning…" : "Rescan now"}
        </button>
      </div>

      <div className="mt-4 flex items-center gap-3 rounded border border-slate-800 bg-slate-900/50 px-3 py-2 text-xs text-slate-400">
        <Database className="size-4 text-slate-500" />
        <span>
          <strong className="text-slate-200">{totalCount}</strong> SOP Instance
          {totalCount === 1 ? "" : "s"} indexed
        </span>
        {scanSummary && (
          <span className="ml-auto text-slate-500">Last scan: {scanSummary}</span>
        )}
      </div>

      {error && (
        <p className="mt-4 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          {error}
        </p>
      )}

      <div className="mt-6">
        {loading ? (
          <p className="text-sm text-slate-500">Loading…</p>
        ) : studies.length === 0 ? (
          <p className="rounded border border-dashed border-slate-700 p-6 text-center text-sm text-slate-500">
            No studies in the store yet. Send a file via C-STORE or drop a
            <code className="mx-1 rounded bg-slate-800 px-1.5 py-0.5 text-xs">
              .dcm
            </code>
            into the store directory and click "Rescan now".
          </p>
        ) : (
          <ul className="space-y-1.5">
            {studies.map((study) => (
              <li
                key={study.study_instance_uid}
                className="rounded border border-slate-800 bg-slate-900/40"
              >
                <button
                  type="button"
                  onClick={() => toggleStudy(study.study_instance_uid)}
                  className="flex w-full items-center gap-3 px-3 py-2 text-left text-sm hover:bg-slate-800/40"
                >
                  {expandedStudies.has(study.study_instance_uid) ? (
                    <ChevronDown className="size-4 shrink-0 text-slate-500" />
                  ) : (
                    <ChevronRight className="size-4 shrink-0 text-slate-500" />
                  )}
                  <Folder className="size-4 shrink-0 text-amber-400" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-medium text-slate-100">
                      {study.patient_name || "(no patient name)"}{" "}
                      <span className="ml-1 text-xs text-slate-500">
                        {study.patient_id}
                      </span>
                    </div>
                    <div className="truncate text-xs text-slate-500">
                      {study.study_description || "(no description)"}
                    </div>
                  </div>
                  <div className="shrink-0 text-right text-xs text-slate-500">
                    <div>{formatDicomDate(study.study_date) || "—"}</div>
                    <div>
                      {study.modalities || "—"} · {study.series_count} series ·{" "}
                      {study.instance_count} instances
                    </div>
                  </div>
                </button>

                {expandedStudies.has(study.study_instance_uid) && (
                  <SeriesList
                    seriesList={seriesByStudy[study.study_instance_uid]}
                    expandedSeries={expandedSeries}
                    onToggleSeries={toggleSeries}
                    instancesBySeries={instancesBySeries}
                  />
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

interface SeriesListProps {
  seriesList: SeriesRow[] | undefined;
  expandedSeries: Set<string>;
  onToggleSeries: (uid: string) => void;
  instancesBySeries: Record<string, InstanceRow[]>;
}

function SeriesList({
  seriesList,
  expandedSeries,
  onToggleSeries,
  instancesBySeries,
}: SeriesListProps) {
  if (!seriesList) {
    return (
      <div className="border-t border-slate-800 px-3 py-2 text-xs text-slate-500">
        Loading series…
      </div>
    );
  }
  if (seriesList.length === 0) {
    return (
      <div className="border-t border-slate-800 px-3 py-2 text-xs text-slate-500">
        No series.
      </div>
    );
  }
  return (
    <ul className="border-t border-slate-800">
      {seriesList.map((series) => (
        <li key={series.series_instance_uid}>
          <button
            type="button"
            onClick={() => onToggleSeries(series.series_instance_uid)}
            className="flex w-full items-center gap-3 px-3 py-1.5 pl-9 text-left text-sm hover:bg-slate-800/40"
          >
            {expandedSeries.has(series.series_instance_uid) ? (
              <ChevronDown className="size-4 shrink-0 text-slate-500" />
            ) : (
              <ChevronRight className="size-4 shrink-0 text-slate-500" />
            )}
            <Folder className="size-4 shrink-0 text-slate-500" />
            <div className="min-w-0 flex-1 truncate text-slate-200">
              {series.series_description || "(no description)"}
            </div>
            <div className="shrink-0 text-xs text-slate-500">
              {series.modality || "—"} · {series.instance_count} instances
            </div>
          </button>
          {expandedSeries.has(series.series_instance_uid) && (
            <InstanceList instances={instancesBySeries[series.series_instance_uid]} />
          )}
        </li>
      ))}
    </ul>
  );
}

function InstanceList({ instances }: { instances: InstanceRow[] | undefined }) {
  if (!instances) {
    return (
      <div className="border-t border-slate-800 px-3 py-2 pl-16 text-xs text-slate-500">
        Loading instances…
      </div>
    );
  }
  if (instances.length === 0) {
    return (
      <div className="border-t border-slate-800 px-3 py-2 pl-16 text-xs text-slate-500">
        No instances.
      </div>
    );
  }
  return (
    <ul className="border-t border-slate-800 bg-slate-950/40">
      {instances.map((inst) => (
        <li
          key={inst.sop_instance_uid}
          className="flex items-center gap-3 px-3 py-1 pl-16 text-xs text-slate-400"
          title={inst.file_path}
        >
          <FileText className="size-3.5 shrink-0 text-slate-600" />
          <span className="truncate font-mono">{inst.sop_instance_uid}</span>
          <span className="ml-auto shrink-0 text-slate-500">
            {humanBytes(inst.size_bytes)}
          </span>
        </li>
      ))}
    </ul>
  );
}
