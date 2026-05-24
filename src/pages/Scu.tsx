import { useEffect, useMemo, useState } from "react";
import { Loader2, Send } from "lucide-react";
import {
  type Peer,
  type QrLevel,
  type QrRoot,
  type ScuEchoResult,
  type ScuFindMatch,
  type ScuFindResult,
  type ScuMoveResult,
  type ScuQueryKeys,
  type ScuStoreOutcome,
  formatError,
  listPeers,
  scuEcho,
  scuFind,
  scuMove,
  scuStore,
} from "../lib/api";
import { Field } from "../components/Field";
import { Select } from "../components/Select";

type Op = "echo" | "find" | "move" | "store";

const INPUT_CLASS =
  "w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm " +
  "text-slate-100 focus:border-sky-500 focus:outline-none";

interface QueryForm {
  root: QrRoot;
  level: QrLevel;
  patient_id: string;
  patient_name: string;
  study_instance_uid: string;
  study_date: string;
  modality: string;
  series_instance_uid: string;
  sop_instance_uid: string;
  return_keys: string;
  destination_ae: string; // for MOVE only
}

const EMPTY_QUERY: QueryForm = {
  root: "study",
  level: "STUDY",
  patient_id: "",
  patient_name: "",
  study_instance_uid: "",
  study_date: "",
  modality: "",
  series_instance_uid: "",
  sop_instance_uid: "",
  return_keys:
    "PatientID, PatientName, StudyInstanceUID, StudyDate, StudyDescription, ModalitiesInStudy",
  destination_ae: "",
};

function queryKeysFromForm(form: QueryForm): ScuQueryKeys {
  const trim = (v: string) => (v.trim() ? v.trim() : undefined);
  const returnKeys = form.return_keys
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
  return {
    patient_id: trim(form.patient_id),
    patient_name: trim(form.patient_name),
    study_instance_uid: trim(form.study_instance_uid),
    study_date: trim(form.study_date),
    modality: trim(form.modality),
    series_instance_uid: trim(form.series_instance_uid),
    sop_instance_uid: trim(form.sop_instance_uid),
    return_keys: returnKeys.length > 0 ? returnKeys : undefined,
  };
}

export function ScuPage() {
  const [peers, setPeers] = useState<Peer[]>([]);
  const [peerId, setPeerId] = useState<string>("");
  const [op, setOp] = useState<Op>("echo");
  const [running, setRunning] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  // Per-operation result holders; only one is set at a time.
  const [echoResult, setEchoResult] = useState<ScuEchoResult | null>(null);
  const [findResult, setFindResult] = useState<ScuFindResult | null>(null);
  const [moveResult, setMoveResult] = useState<ScuMoveResult | null>(null);
  const [storeResult, setStoreResult] = useState<ScuStoreOutcome[] | null>(null);

  const [form, setForm] = useState<QueryForm>(EMPTY_QUERY);
  const [storeFiles, setStoreFiles] = useState<string>("");

  useEffect(() => {
    listPeers()
      .then((rows) => {
        setPeers(rows);
        if (rows.length > 0 && !peerId) setPeerId(rows[0].id);
      })
      .catch((err) => setError(formatError(err)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const clearResults = () => {
    setEchoResult(null);
    setFindResult(null);
    setMoveResult(null);
    setStoreResult(null);
    setError(null);
  };

  const handleRun = async () => {
    if (!peerId) {
      setError("Add a peer on the Peers page first.");
      return;
    }
    setRunning(true);
    clearResults();
    try {
      if (op === "echo") {
        setEchoResult(await scuEcho(peerId));
      } else if (op === "find") {
        setFindResult(await scuFind(peerId, form.root, form.level, queryKeysFromForm(form)));
      } else if (op === "move") {
        const dest = form.destination_ae.trim();
        if (!dest) {
          setError("Move Destination AE Title is required.");
          setRunning(false);
          return;
        }
        setMoveResult(
          await scuMove(peerId, form.root, form.level, queryKeysFromForm(form), dest),
        );
      } else if (op === "store") {
        const files = storeFiles
          .split(/\r?\n/)
          .map((f) => f.trim())
          .filter(Boolean);
        if (files.length === 0) {
          setError("Add at least one file path (one per line).");
          setRunning(false);
          return;
        }
        setStoreResult(await scuStore(peerId, files));
      }
    } catch (err) {
      setError(formatError(err));
    } finally {
      setRunning(false);
    }
  };

  return (
    <section className="max-w-5xl">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">SCU</h1>
          <p className="mt-1 text-sm text-slate-400">
            Send outbound DIMSE operations to a configured peer.
          </p>
        </div>
      </div>

      {/* Peer + operation switcher */}
      <div className="mt-6 grid grid-cols-1 gap-4 md:grid-cols-2">
        <Field label="Peer" hint="Configured on the Peers page.">
          <Select
            value={peerId}
            onChange={(e) => setPeerId(e.target.value)}
            className="w-full"
          >
            {peers.length === 0 ? (
              <option value="">No peers configured</option>
            ) : (
              peers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} ({p.ae_title} @ {p.host}:{p.port})
                </option>
              ))
            )}
          </Select>
        </Field>
        <Field label="Operation">
          <div className="flex flex-wrap gap-2">
            {(["echo", "find", "move", "store"] as Op[]).map((o) => (
              <button
                key={o}
                type="button"
                onClick={() => {
                  setOp(o);
                  clearResults();
                }}
                className={
                  "rounded border px-3 py-1.5 text-sm font-medium uppercase tracking-wide " +
                  (op === o
                    ? "border-sky-500 bg-sky-600 text-white"
                    : "border-slate-700 text-slate-300 hover:border-slate-500")
                }
              >
                {o === "echo" ? "C-ECHO" : `C-${o.toUpperCase()}`}
              </button>
            ))}
          </div>
        </Field>
      </div>

      {/* Per-operation form */}
      <div className="mt-6">
        {op === "echo" ? (
          <EchoForm />
        ) : op === "store" ? (
          <StoreForm value={storeFiles} onChange={setStoreFiles} />
        ) : (
          <QueryForm op={op} form={form} setForm={setForm} />
        )}
      </div>

      <div className="mt-6 flex items-center gap-3">
        <button
          type="button"
          onClick={handleRun}
          disabled={running || !peerId}
          className={
            "flex items-center gap-2 rounded bg-sky-600 px-4 py-1.5 text-sm font-medium text-white " +
            "hover:bg-sky-500 disabled:bg-slate-700 disabled:text-slate-500 disabled:cursor-not-allowed"
          }
        >
          {running ? <Loader2 className="size-4 animate-spin" /> : <Send className="size-4" />}
          {opLabel(op)}
        </button>
        {running && (
          <span className="text-xs text-slate-500">running…</span>
        )}
      </div>

      {error && (
        <p className="mt-4 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          {error}
        </p>
      )}

      {echoResult && <EchoResultPanel result={echoResult} />}
      {findResult && <FindResultPanel result={findResult} />}
      {moveResult && <MoveResultPanel result={moveResult} />}
      {storeResult && <StoreResultPanel outcomes={storeResult} />}
    </section>
  );
}

function opLabel(op: Op): string {
  switch (op) {
    case "echo":
      return "Send Echo";
    case "find":
      return "Run Query";
    case "move":
      return "Send Move";
    case "store":
      return "Send Store";
  }
}

// ----- Sub-forms ----------------------------------------------------

function EchoForm() {
  return (
    <p className="rounded border border-slate-800 bg-slate-900/30 p-3 text-sm text-slate-400">
      C-ECHO has no parameters. Click "Send Echo" to verify the peer accepts a
      Verification association and answers with status 0x0000.
    </p>
  );
}

function StoreForm({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <Field
      label="Files (absolute paths, one per line)"
      hint="Each file is parsed once on the way out to determine its SOP Class for negotiation."
    >
      <textarea
        className={INPUT_CLASS + " h-32 font-mono text-xs"}
        spellCheck={false}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="/Users/you/dicom-samples/IM000001.dcm"
      />
    </Field>
  );
}

function QueryForm({
  op,
  form,
  setForm,
}: {
  op: Op;
  form: QueryForm;
  setForm: (f: QueryForm) => void;
}) {
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Field label="Q/R root" hint="Patient Root uses PatientID as the entry key; Study Root uses StudyInstanceUID.">
          <Select
            value={form.root}
            onChange={(e) => setForm({ ...form, root: e.target.value as QrRoot })}
            className="w-full"
          >
            <option value="study">Study Root</option>
            <option value="patient">Patient Root</option>
          </Select>
        </Field>
        <Field label="Level">
          <Select
            value={form.level}
            onChange={(e) => setForm({ ...form, level: e.target.value as QrLevel })}
            className="w-full"
          >
            <option value="PATIENT">PATIENT</option>
            <option value="STUDY">STUDY</option>
            <option value="SERIES">SERIES</option>
            <option value="IMAGE">IMAGE</option>
          </Select>
        </Field>
      </div>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <Field label="PatientID">
          <input
            className={INPUT_CLASS}
            value={form.patient_id}
            spellCheck={false}
            onChange={(e) => setForm({ ...form, patient_id: e.target.value })}
          />
        </Field>
        <Field label="PatientName" hint="Wildcards * and ? allowed.">
          <input
            className={INPUT_CLASS}
            value={form.patient_name}
            spellCheck={false}
            onChange={(e) => setForm({ ...form, patient_name: e.target.value })}
          />
        </Field>
        <Field label="StudyInstanceUID">
          <input
            className={INPUT_CLASS + " font-mono text-xs"}
            value={form.study_instance_uid}
            spellCheck={false}
            onChange={(e) => setForm({ ...form, study_instance_uid: e.target.value })}
          />
        </Field>
        <Field label="StudyDate" hint="YYYYMMDD or YYYYMMDD-YYYYMMDD range.">
          <input
            className={INPUT_CLASS}
            value={form.study_date}
            spellCheck={false}
            onChange={(e) => setForm({ ...form, study_date: e.target.value })}
          />
        </Field>
        <Field label="Modality">
          <input
            className={INPUT_CLASS}
            value={form.modality}
            spellCheck={false}
            onChange={(e) => setForm({ ...form, modality: e.target.value })}
          />
        </Field>
        <Field label="SeriesInstanceUID">
          <input
            className={INPUT_CLASS + " font-mono text-xs"}
            value={form.series_instance_uid}
            spellCheck={false}
            onChange={(e) => setForm({ ...form, series_instance_uid: e.target.value })}
          />
        </Field>
      </div>

      <Field
        label="Return keys"
        hint="Comma-separated tag names included as Universal Matching (no filter) so the response carries them."
      >
        <input
          className={INPUT_CLASS}
          value={form.return_keys}
          spellCheck={false}
          onChange={(e) => setForm({ ...form, return_keys: e.target.value })}
        />
      </Field>

      {op === "move" && (
        <Field
          label="Move Destination AE Title"
          hint="The receiving peer's AE Title — must be configured on the source peer's side as a known destination."
        >
          <input
            className={INPUT_CLASS}
            value={form.destination_ae}
            maxLength={16}
            spellCheck={false}
            onChange={(e) => setForm({ ...form, destination_ae: e.target.value })}
          />
        </Field>
      )}
    </div>
  );
}

// ----- Result panels ------------------------------------------------

function EchoResultPanel({ result }: { result: ScuEchoResult }) {
  return (
    <div
      className={
        "mt-4 rounded border p-3 text-sm " +
        (result.success
          ? "border-emerald-700/50 bg-emerald-900/20 text-emerald-200"
          : "border-red-700/50 bg-red-900/20 text-red-300")
      }
    >
      <div className="font-medium">{result.message}</div>
      <div className="mt-1 text-xs opacity-70">
        status 0x{result.status.toString(16).toUpperCase().padStart(4, "0")} · {result.elapsed_ms} ms
      </div>
    </div>
  );
}

function MoveResultPanel({ result }: { result: ScuMoveResult }) {
  const ok = result.status === 0;
  return (
    <div
      className={
        "mt-4 rounded border p-3 text-sm " +
        (ok
          ? "border-emerald-700/50 bg-emerald-900/20 text-emerald-200"
          : "border-amber-700/50 bg-amber-900/20 text-amber-200")
      }
    >
      <div className="font-medium">
        C-MOVE-RSP {result.status_label} (0x
        {result.status.toString(16).toUpperCase().padStart(4, "0")})
      </div>
      <div className="mt-1 text-xs opacity-70">
        completed {result.completed} · failed {result.failed} · {result.elapsed_ms} ms
      </div>
    </div>
  );
}

function StoreResultPanel({ outcomes }: { outcomes: ScuStoreOutcome[] }) {
  const ok = outcomes.filter((o) => o.success).length;
  return (
    <div className="mt-4 rounded border border-slate-800 bg-slate-900/30">
      <div className="border-b border-slate-800 px-3 py-2 text-sm text-slate-300">
        <strong className="text-slate-100">{ok}</strong> of {outcomes.length} files
        stored
      </div>
      <table className="w-full text-sm">
        <tbody className="divide-y divide-slate-800/60">
          {outcomes.map((o) => (
            <tr key={o.file}>
              <td className="px-3 py-1.5 align-top">
                <div
                  className={
                    "h-2 w-2 mt-1.5 rounded-full " +
                    (o.success ? "bg-emerald-400" : "bg-red-400")
                  }
                />
              </td>
              <td className="px-1 py-1.5 font-mono text-xs text-slate-300 truncate max-w-[24rem]">
                {o.file}
              </td>
              <td className="px-3 py-1.5 text-xs text-slate-400">{o.message}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function FindResultPanel({ result }: { result: ScuFindResult }) {
  const columns = useFindColumns(result.matches);
  return (
    <div className="mt-4">
      <div className="mb-2 text-xs text-slate-500">
        {result.matches.length} match{result.matches.length === 1 ? "" : "es"} in{" "}
        {result.elapsed_ms} ms
      </div>
      <div className="overflow-x-auto rounded border border-slate-800 bg-slate-900/30">
        <table className="w-full text-sm">
          <thead className="border-b border-slate-800 bg-slate-900/60 text-xs uppercase tracking-wide text-slate-500">
            <tr>
              {columns.map((c) => (
                <th key={c} className="px-3 py-2 text-left font-medium">
                  {c}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-800/60">
            {result.matches.length === 0 ? (
              <tr>
                <td
                  colSpan={Math.max(1, columns.length)}
                  className="px-3 py-6 text-center text-sm text-slate-500"
                >
                  No matches.
                </td>
              </tr>
            ) : (
              result.matches.map((m, idx) => (
                <tr key={idx} className="hover:bg-slate-800/30">
                  {columns.map((c) => (
                    <td key={c} className="px-3 py-1.5 text-slate-200">
                      <span
                        className={
                          c.endsWith("UID")
                            ? "font-mono text-xs text-slate-300"
                            : ""
                        }
                      >
                        {m.fields[c] ?? "—"}
                      </span>
                    </td>
                  ))}
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function useFindColumns(matches: ScuFindMatch[]): string[] {
  return useMemo(() => {
    const seen = new Set<string>();
    for (const m of matches) {
      for (const key of Object.keys(m.fields)) seen.add(key);
    }
    // Keep QueryRetrieveLevel first, then friendly ordering, then everything else alphabetically.
    const preferred = [
      "QueryRetrieveLevel",
      "PatientID",
      "PatientName",
      "StudyDate",
      "Modality",
      "ModalitiesInStudy",
      "StudyDescription",
      "SeriesDescription",
      "StudyInstanceUID",
      "SeriesInstanceUID",
      "SOPInstanceUID",
    ];
    const cols: string[] = [];
    for (const p of preferred) {
      if (seen.has(p)) {
        cols.push(p);
        seen.delete(p);
      }
    }
    return [...cols, ...Array.from(seen).sort()];
  }, [matches]);
}
