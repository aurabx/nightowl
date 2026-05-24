import { useEffect, useState } from "react";
import { ClipboardList, Pencil, Plus, Trash2, X } from "lucide-react";
import {
  type NewWorklistEntry,
  type WorklistEntry,
  createWorklistEntry,
  deleteWorklistEntry,
  formatError,
  isAppError,
  listWorklist,
  updateWorklistEntry,
} from "../lib/api";
import { Field } from "../components/Field";
import { Select } from "../components/Select";

const INPUT_CLASS =
  "w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm " +
  "text-slate-100 focus:border-sky-500 focus:outline-none";

interface DraftEntry {
  id?: string;
  accession_number: string;
  patient_id: string;
  patient_name: string;
  patient_birth_date: string;
  patient_sex: string;
  study_instance_uid: string;
  requested_procedure_id: string;
  requested_procedure_description: string;
  scheduled_station_ae_title: string;
  scheduled_procedure_step_start_date: string;
  scheduled_procedure_step_start_time: string;
  scheduled_procedure_step_id: string;
  scheduled_procedure_step_description: string;
  modality: string;
}

const EMPTY_DRAFT: DraftEntry = {
  accession_number: "",
  patient_id: "",
  patient_name: "",
  patient_birth_date: "",
  patient_sex: "",
  study_instance_uid: "",
  requested_procedure_id: "",
  requested_procedure_description: "",
  scheduled_station_ae_title: "",
  scheduled_procedure_step_start_date: today(),
  scheduled_procedure_step_start_time: "",
  scheduled_procedure_step_id: "",
  scheduled_procedure_step_description: "",
  modality: "MR",
};

function today(): string {
  const d = new Date();
  return `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;
}

function formatDicomDate(s: string | null): string {
  if (!s || s.length !== 8) return s ?? "—";
  return `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}`;
}

function formatDicomTime(s: string | null): string {
  if (!s) return "";
  // HHMMSS or HHMM
  if (s.length === 6) return `${s.slice(0, 2)}:${s.slice(2, 4)}:${s.slice(4, 6)}`;
  if (s.length === 4) return `${s.slice(0, 2)}:${s.slice(2, 4)}`;
  return s;
}

const MODALITIES = ["CT", "MR", "CR", "DX", "US", "PT", "NM", "XA", "MG", "OT", "SC"];

export function WorklistPage() {
  const [entries, setEntries] = useState<WorklistEntry[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [editing, setEditing] = useState<DraftEntry | null>(null);
  const [savingErr, setSavingErr] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<boolean>(false);

  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setLoadError(null);
    try {
      setEntries(await listWorklist());
    } catch (err) {
      setLoadError(formatError(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const openAdd = () => {
    setEditing({ ...EMPTY_DRAFT });
    setFieldErrors({});
    setSavingErr(null);
  };

  const openEdit = (entry: WorklistEntry) => {
    setEditing({
      id: entry.id,
      accession_number: entry.accession_number,
      patient_id: entry.patient_id,
      patient_name: entry.patient_name,
      patient_birth_date: entry.patient_birth_date ?? "",
      patient_sex: entry.patient_sex ?? "",
      study_instance_uid: entry.study_instance_uid,
      requested_procedure_id: entry.requested_procedure_id ?? "",
      requested_procedure_description: entry.requested_procedure_description ?? "",
      scheduled_station_ae_title: entry.scheduled_station_ae_title,
      scheduled_procedure_step_start_date: entry.scheduled_procedure_step_start_date,
      scheduled_procedure_step_start_time: entry.scheduled_procedure_step_start_time ?? "",
      scheduled_procedure_step_id: entry.scheduled_procedure_step_id,
      scheduled_procedure_step_description: entry.scheduled_procedure_step_description ?? "",
      modality: entry.modality,
    });
    setFieldErrors({});
    setSavingErr(null);
  };

  const cancelEdit = () => {
    setEditing(null);
    setFieldErrors({});
    setSavingErr(null);
  };

  useEffect(() => {
    if (editing === null) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") cancelEdit();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [editing]);

  const handleSave = async () => {
    if (!editing) return;
    setSaving(true);
    setFieldErrors({});
    setSavingErr(null);
    const optional = (v: string) => (v.trim() ? v.trim() : undefined);
    try {
      if (editing.id) {
        await updateWorklistEntry({
          id: editing.id,
          accession_number: editing.accession_number,
          patient_id: editing.patient_id,
          patient_name: editing.patient_name,
          patient_birth_date: editing.patient_birth_date.trim() || null,
          patient_sex: editing.patient_sex.trim() || null,
          study_instance_uid: editing.study_instance_uid,
          requested_procedure_id: editing.requested_procedure_id.trim() || null,
          requested_procedure_description:
            editing.requested_procedure_description.trim() || null,
          scheduled_station_ae_title: editing.scheduled_station_ae_title,
          scheduled_procedure_step_start_date: editing.scheduled_procedure_step_start_date,
          scheduled_procedure_step_start_time:
            editing.scheduled_procedure_step_start_time.trim() || null,
          scheduled_procedure_step_id: editing.scheduled_procedure_step_id,
          scheduled_procedure_step_description:
            editing.scheduled_procedure_step_description.trim() || null,
          modality: editing.modality,
        });
      } else {
        const created: NewWorklistEntry = {
          accession_number: editing.accession_number,
          patient_id: editing.patient_id,
          patient_name: editing.patient_name,
          patient_birth_date: optional(editing.patient_birth_date),
          patient_sex: optional(editing.patient_sex),
          study_instance_uid: optional(editing.study_instance_uid),
          requested_procedure_id: optional(editing.requested_procedure_id),
          requested_procedure_description: optional(editing.requested_procedure_description),
          scheduled_station_ae_title: editing.scheduled_station_ae_title,
          scheduled_procedure_step_start_date: editing.scheduled_procedure_step_start_date,
          scheduled_procedure_step_start_time: optional(
            editing.scheduled_procedure_step_start_time,
          ),
          scheduled_procedure_step_id: optional(editing.scheduled_procedure_step_id),
          scheduled_procedure_step_description: optional(
            editing.scheduled_procedure_step_description,
          ),
          modality: editing.modality,
        };
        await createWorklistEntry(created);
      }
      await refresh();
      cancelEdit();
    } catch (err: unknown) {
      if (isAppError(err) && err.kind === "Validation") {
        setFieldErrors({ [err.message.field]: err.message.reason });
      } else {
        setSavingErr(formatError(err));
      }
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (entry: WorklistEntry) => {
    if (confirmDeleteId !== entry.id) {
      setConfirmDeleteId(entry.id);
      window.setTimeout(() => {
        setConfirmDeleteId((cur) => (cur === entry.id ? null : cur));
      }, 4000);
      return;
    }
    setConfirmDeleteId(null);
    try {
      await deleteWorklistEntry(entry.id);
      await refresh();
    } catch (err) {
      setLoadError(formatError(err));
    }
  };

  return (
    <section className="max-w-6xl">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">Worklist</h1>
          <p className="mt-1 text-sm text-slate-400">
            Modality Worklist (DMWL) entries — scheduled procedure steps. A
            modality at the Scheduled Station AE Title can pull these via a
            C-FIND query on the Modality Worklist Information Model.
          </p>
        </div>
        {editing === null && (
          <button
            type="button"
            onClick={openAdd}
            className="flex items-center gap-1.5 rounded bg-sky-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-sky-500"
          >
            <Plus className="size-4" />
            Add entry
          </button>
        )}
      </div>

      {loadError && (
        <p className="mt-4 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          {loadError}
        </p>
      )}

      {editing !== null ? (
        <EntryForm
          editing={editing}
          setEditing={setEditing}
          fieldErrors={fieldErrors}
          savingErr={savingErr}
          saving={saving}
          onCancel={cancelEdit}
          onSave={handleSave}
        />
      ) : (
      <div className="mt-6 overflow-x-auto rounded border border-slate-800 bg-slate-900/30">
        <table className="w-full text-sm">
          <thead className="border-b border-slate-800 bg-slate-900/60 text-xs uppercase tracking-wide text-slate-500">
            <tr>
              <th className="px-3 py-2 text-left font-medium">Scheduled</th>
              <th className="px-3 py-2 text-left font-medium">Patient</th>
              <th className="px-3 py-2 text-left font-medium">Accession</th>
              <th className="px-3 py-2 text-left font-medium">Procedure</th>
              <th className="px-3 py-2 text-left font-medium w-24">Modality</th>
              <th className="px-3 py-2 text-left font-medium">Station AE</th>
              <th className="px-3 py-2 text-right font-medium w-24"></th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-800/60">
            {loading ? (
              <tr>
                <td colSpan={7} className="px-3 py-8 text-center text-sm text-slate-500">
                  Loading…
                </td>
              </tr>
            ) : entries.length === 0 ? (
              <tr>
                <td colSpan={7} className="px-3 py-10 text-center text-sm text-slate-500">
                  <ClipboardList className="mx-auto size-6 text-slate-700" />
                  <div className="mt-2">No worklist entries.</div>
                  <div className="mt-1 text-xs text-slate-600">
                    Click "Add entry" to schedule a procedure step.
                  </div>
                </td>
              </tr>
            ) : (
              entries.map((entry) => (
                <tr key={entry.id} className="hover:bg-slate-800/30">
                  <td className="px-3 py-2">
                    <div className="text-slate-100">
                      {formatDicomDate(entry.scheduled_procedure_step_start_date)}
                    </div>
                    {entry.scheduled_procedure_step_start_time && (
                      <div className="text-xs text-slate-500">
                        {formatDicomTime(entry.scheduled_procedure_step_start_time)}
                      </div>
                    )}
                  </td>
                  <td className="px-3 py-2">
                    <div className="text-slate-100">{entry.patient_name}</div>
                    <div className="text-xs text-slate-500">{entry.patient_id}</div>
                  </td>
                  <td className="px-3 py-2 font-mono text-xs text-slate-300">
                    {entry.accession_number}
                  </td>
                  <td className="px-3 py-2">
                    <div className="text-slate-300">
                      {entry.requested_procedure_description || "—"}
                    </div>
                    {entry.scheduled_procedure_step_description && (
                      <div className="text-xs text-slate-500">
                        {entry.scheduled_procedure_step_description}
                      </div>
                    )}
                  </td>
                  <td className="px-3 py-2">
                    <span className="rounded bg-slate-800 px-1.5 py-0.5 font-mono text-xs text-sky-300">
                      {entry.modality}
                    </span>
                  </td>
                  <td className="px-3 py-2 font-mono text-xs text-emerald-300">
                    {entry.scheduled_station_ae_title}
                  </td>
                  <td className="px-3 py-2 text-right">
                    <button
                      type="button"
                      onClick={() => openEdit(entry)}
                      className="mr-1 inline-flex rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-200"
                      title="Edit"
                    >
                      <Pencil className="size-3.5" />
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDelete(entry)}
                      className={
                        "inline-flex rounded p-1 " +
                        (confirmDeleteId === entry.id
                          ? "bg-red-600 text-white hover:bg-red-500"
                          : "text-slate-400 hover:bg-slate-800 hover:text-red-300")
                      }
                      title={
                        confirmDeleteId === entry.id
                          ? "Click again to confirm"
                          : "Delete"
                      }
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
      )}
    </section>
  );
}

interface EntryFormProps {
  editing: DraftEntry;
  setEditing: (draft: DraftEntry) => void;
  fieldErrors: Record<string, string>;
  savingErr: string | null;
  saving: boolean;
  onCancel: () => void;
  onSave: () => void;
}

function EntryForm({
  editing,
  setEditing,
  fieldErrors,
  savingErr,
  saving,
  onCancel,
  onSave,
}: EntryFormProps) {
  return (
    <div className="mt-6 flex max-h-[calc(100vh-12rem)] flex-col overflow-hidden rounded border border-slate-800 bg-slate-900/30">
      <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
        <h2 className="text-base font-semibold text-slate-100">
          {editing.id ? "Edit worklist entry" : "Add worklist entry"}
        </h2>
        <button
          type="button"
          onClick={onCancel}
          className="rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
          aria-label="Cancel"
        >
          <X className="size-4" />
        </button>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-4">
        <div className="space-y-4">
            <div className="grid grid-cols-2 gap-3">
              <Field
                label="Accession #"
                error={fieldErrors.accession_number}
              >
                <input
                  className={INPUT_CLASS}
                  value={editing.accession_number}
                  spellCheck={false}
                  onChange={(e) =>
                    setEditing({ ...editing, accession_number: e.target.value })
                  }
                />
              </Field>
              <Field label="Modality" error={fieldErrors.modality}>
                <Select
                  value={editing.modality}
                  onChange={(e) => setEditing({ ...editing, modality: e.target.value })}
                  className="w-full"
                >
                  {MODALITIES.map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </Select>
              </Field>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <Field label="Patient ID" error={fieldErrors.patient_id}>
                <input
                  className={INPUT_CLASS}
                  value={editing.patient_id}
                  spellCheck={false}
                  onChange={(e) => setEditing({ ...editing, patient_id: e.target.value })}
                />
              </Field>
              <Field
                label="Patient Name"
                hint='DICOM PN format: "Family^Given".'
                error={fieldErrors.patient_name}
              >
                <input
                  className={INPUT_CLASS}
                  value={editing.patient_name}
                  spellCheck={false}
                  onChange={(e) => setEditing({ ...editing, patient_name: e.target.value })}
                />
              </Field>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <Field label="Birth Date" hint="YYYYMMDD">
                <input
                  className={INPUT_CLASS}
                  value={editing.patient_birth_date}
                  spellCheck={false}
                  placeholder="19800101"
                  onChange={(e) =>
                    setEditing({ ...editing, patient_birth_date: e.target.value })
                  }
                />
              </Field>
              <Field label="Sex">
                <Select
                  value={editing.patient_sex}
                  onChange={(e) => setEditing({ ...editing, patient_sex: e.target.value })}
                  className="w-full"
                >
                  <option value="">—</option>
                  <option value="M">M</option>
                  <option value="F">F</option>
                  <option value="O">O</option>
                </Select>
              </Field>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <Field
                label="Scheduled Date"
                hint="YYYYMMDD"
                error={fieldErrors.scheduled_procedure_step_start_date}
              >
                <input
                  className={INPUT_CLASS}
                  value={editing.scheduled_procedure_step_start_date}
                  spellCheck={false}
                  placeholder={today()}
                  onChange={(e) =>
                    setEditing({
                      ...editing,
                      scheduled_procedure_step_start_date: e.target.value,
                    })
                  }
                />
              </Field>
              <Field label="Scheduled Time" hint="HHMMSS or HHMM (optional)">
                <input
                  className={INPUT_CLASS}
                  value={editing.scheduled_procedure_step_start_time}
                  spellCheck={false}
                  placeholder="093000"
                  onChange={(e) =>
                    setEditing({
                      ...editing,
                      scheduled_procedure_step_start_time: e.target.value,
                    })
                  }
                />
              </Field>
            </div>

            <Field
              label="Scheduled Station AE Title"
              hint="The modality that should perform this step."
              error={fieldErrors.scheduled_station_ae_title}
            >
              <input
                className={INPUT_CLASS}
                value={editing.scheduled_station_ae_title}
                maxLength={16}
                spellCheck={false}
                onChange={(e) =>
                  setEditing({ ...editing, scheduled_station_ae_title: e.target.value })
                }
              />
            </Field>

            <Field label="Requested Procedure Description">
              <input
                className={INPUT_CLASS}
                value={editing.requested_procedure_description}
                spellCheck={false}
                placeholder="MRI Knee"
                onChange={(e) =>
                  setEditing({
                    ...editing,
                    requested_procedure_description: e.target.value,
                  })
                }
              />
            </Field>

            <Field label="Scheduled Procedure Step Description">
              <input
                className={INPUT_CLASS}
                value={editing.scheduled_procedure_step_description}
                spellCheck={false}
                placeholder="T1 weighted sequence"
                onChange={(e) =>
                  setEditing({
                    ...editing,
                    scheduled_procedure_step_description: e.target.value,
                  })
                }
              />
            </Field>

            <details className="rounded border border-slate-800 p-2 text-xs">
              <summary className="cursor-pointer text-slate-400">
                Advanced (UIDs and IDs — auto-generated when empty)
              </summary>
              <div className="mt-3 grid grid-cols-1 gap-3">
                <Field label="Study Instance UID">
                  <input
                    className={INPUT_CLASS + " font-mono text-xs"}
                    value={editing.study_instance_uid}
                    spellCheck={false}
                    placeholder="auto-generated 2.25.…"
                    onChange={(e) =>
                      setEditing({ ...editing, study_instance_uid: e.target.value })
                    }
                  />
                </Field>
                <Field label="Scheduled Procedure Step ID">
                  <input
                    className={INPUT_CLASS}
                    value={editing.scheduled_procedure_step_id}
                    spellCheck={false}
                    placeholder="SPS-…"
                    onChange={(e) =>
                      setEditing({
                        ...editing,
                        scheduled_procedure_step_id: e.target.value,
                      })
                    }
                  />
                </Field>
                <Field label="Requested Procedure ID">
                  <input
                    className={INPUT_CLASS}
                    value={editing.requested_procedure_id}
                    spellCheck={false}
                    onChange={(e) =>
                      setEditing({
                        ...editing,
                        requested_procedure_id: e.target.value,
                      })
                    }
                  />
                </Field>
              </div>
            </details>

            {savingErr && (
              <p className="rounded border border-red-700/50 bg-red-900/20 p-2 text-sm text-red-300">
                {savingErr}
              </p>
            )}
        </div>
      </div>
      <div className="flex justify-end gap-2 border-t border-slate-800 bg-slate-950/50 px-4 py-3">
        <button
          type="button"
          onClick={onCancel}
          className="rounded border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-500"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={onSave}
          disabled={saving}
          className="rounded bg-sky-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-sky-500 disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  );
}
