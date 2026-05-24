import { useEffect, useState } from "react";
import { Network, Pencil, Plus, Trash2 } from "lucide-react";
import {
  type NewPeer,
  type Peer,
  createPeer,
  deletePeer,
  formatError,
  isAppError,
  listPeers,
  updatePeer,
} from "../lib/api";
import { Field } from "../components/Field";
import { Modal } from "../components/Modal";

const INPUT_CLASS =
  "w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm " +
  "text-slate-100 focus:border-sky-500 focus:outline-none";

interface DraftPeer {
  // `id` is only set when editing an existing peer.
  id?: string;
  name: string;
  ae_title: string;
  host: string;
  port: number;
}

const EMPTY_DRAFT: DraftPeer = {
  name: "",
  ae_title: "",
  host: "",
  port: 11112,
};

export function PeersPage() {
  const [peers, setPeers] = useState<Peer[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [editing, setEditing] = useState<DraftPeer | null>(null);
  const [savingErr, setSavingErr] = useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState<boolean>(false);

  // Two-click delete confirmation per row, keyed by peer id.
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setLoadError(null);
    try {
      setPeers(await listPeers());
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

  const openEdit = (peer: Peer) => {
    setEditing({
      id: peer.id,
      name: peer.name,
      ae_title: peer.ae_title,
      host: peer.host,
      port: peer.port,
    });
    setFieldErrors({});
    setSavingErr(null);
  };

  const closeModal = () => {
    setEditing(null);
    setFieldErrors({});
    setSavingErr(null);
  };

  const handleSave = async () => {
    if (!editing) return;
    setSaving(true);
    setFieldErrors({});
    setSavingErr(null);
    try {
      if (editing.id) {
        await updatePeer({
          id: editing.id,
          name: editing.name,
          ae_title: editing.ae_title,
          host: editing.host,
          port: editing.port,
        });
      } else {
        const newPeer: NewPeer = {
          name: editing.name,
          ae_title: editing.ae_title,
          host: editing.host,
          port: editing.port,
        };
        await createPeer(newPeer);
      }
      await refresh();
      closeModal();
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

  const handleDelete = async (peer: Peer) => {
    if (confirmDeleteId !== peer.id) {
      setConfirmDeleteId(peer.id);
      window.setTimeout(() => {
        // Only clear if still pointing at this peer.
        setConfirmDeleteId((cur) => (cur === peer.id ? null : cur));
      }, 4000);
      return;
    }
    setConfirmDeleteId(null);
    try {
      await deletePeer(peer.id);
      await refresh();
    } catch (err) {
      setLoadError(formatError(err));
    }
  };

  return (
    <section className="max-w-4xl">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold">Peers</h1>
          <p className="mt-1 text-sm text-slate-400">
            Remote DICOM nodes NightOwl can talk to. Stored as
            <code className="mx-1 rounded bg-slate-800 px-1.5 py-0.5 text-xs">
              peers.json
            </code>
            in the app config directory. C-MOVE resolves Move Destination AE
            Titles against this list.
          </p>
        </div>
        <button
          type="button"
          onClick={openAdd}
          className="flex items-center gap-1.5 rounded bg-sky-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-sky-500"
        >
          <Plus className="size-4" />
          Add peer
        </button>
      </div>

      {loadError && (
        <p className="mt-4 rounded border border-red-700/50 bg-red-900/20 p-3 text-sm text-red-300">
          {loadError}
        </p>
      )}

      <div className="mt-6 overflow-x-auto rounded border border-slate-800 bg-slate-900/30">
        <table className="w-full text-sm">
          <thead className="border-b border-slate-800 bg-slate-900/60 text-xs uppercase tracking-wide text-slate-500">
            <tr>
              <th className="px-3 py-2 text-left font-medium">Name</th>
              <th className="px-3 py-2 text-left font-medium">AE Title</th>
              <th className="px-3 py-2 text-left font-medium">Host</th>
              <th className="px-3 py-2 text-left font-medium w-20">Port</th>
              <th className="px-3 py-2 text-right font-medium w-24"></th>
            </tr>
          </thead>
          <tbody className="divide-y divide-slate-800/60">
            {loading ? (
              <tr>
                <td colSpan={5} className="px-3 py-8 text-center text-sm text-slate-500">
                  Loading…
                </td>
              </tr>
            ) : peers.length === 0 ? (
              <tr>
                <td colSpan={5} className="px-3 py-10 text-center text-sm text-slate-500">
                  <Network className="mx-auto size-6 text-slate-700" />
                  <div className="mt-2">No peers configured.</div>
                  <div className="mt-1 text-xs text-slate-600">
                    Click "Add peer" to register a remote DICOM node.
                  </div>
                </td>
              </tr>
            ) : (
              peers.map((peer) => (
                <tr key={peer.id} className="hover:bg-slate-800/30">
                  <td className="px-3 py-2 text-slate-100">{peer.name}</td>
                  <td className="px-3 py-2 font-mono text-xs text-sky-300">
                    {peer.ae_title}
                  </td>
                  <td className="px-3 py-2 text-slate-300">{peer.host}</td>
                  <td className="px-3 py-2 text-slate-300">{peer.port}</td>
                  <td className="px-3 py-2 text-right">
                    <button
                      type="button"
                      onClick={() => openEdit(peer)}
                      className="mr-1 inline-flex rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-200"
                      title="Edit"
                    >
                      <Pencil className="size-3.5" />
                    </button>
                    <button
                      type="button"
                      onClick={() => handleDelete(peer)}
                      className={
                        "inline-flex rounded p-1 " +
                        (confirmDeleteId === peer.id
                          ? "bg-red-600 text-white hover:bg-red-500"
                          : "text-slate-400 hover:bg-slate-800 hover:text-red-300")
                      }
                      title={
                        confirmDeleteId === peer.id
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

      <Modal
        open={editing !== null}
        onClose={closeModal}
        title={editing?.id ? "Edit peer" : "Add peer"}
        footer={
          <>
            <button
              type="button"
              onClick={closeModal}
              className="rounded border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-500"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleSave}
              disabled={saving}
              className="rounded bg-sky-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-sky-500 disabled:opacity-50"
            >
              {saving ? "Saving…" : "Save"}
            </button>
          </>
        }
      >
        {editing && (
          <div className="space-y-4">
            <Field label="Name" error={fieldErrors.name} hint="Free text. Shown in the table.">
              <input
                type="text"
                className={INPUT_CLASS}
                value={editing.name}
                spellCheck={false}
                onChange={(e) => setEditing({ ...editing, name: e.target.value })}
              />
            </Field>
            <Field
              label="AE Title"
              error={fieldErrors.ae_title}
              hint="1–16 ASCII characters. The DICOM identifier the peer uses on the wire."
            >
              <input
                type="text"
                className={INPUT_CLASS}
                maxLength={16}
                spellCheck={false}
                value={editing.ae_title}
                onChange={(e) =>
                  setEditing({ ...editing, ae_title: e.target.value })
                }
              />
            </Field>
            <Field label="Host" error={fieldErrors.host} hint="Hostname or IP address.">
              <input
                type="text"
                className={INPUT_CLASS}
                spellCheck={false}
                value={editing.host}
                onChange={(e) => setEditing({ ...editing, host: e.target.value })}
              />
            </Field>
            <Field
              label="Port"
              error={fieldErrors.port}
              hint="TCP port the peer's SCP listens on (typically 104, 4242, or 11112)."
            >
              <input
                type="number"
                className={INPUT_CLASS}
                min={1}
                max={65535}
                value={editing.port}
                onChange={(e) =>
                  setEditing({
                    ...editing,
                    port: Number.parseInt(e.target.value, 10) || 0,
                  })
                }
              />
            </Field>
            {savingErr && (
              <p className="rounded border border-red-700/50 bg-red-900/20 p-2 text-sm text-red-300">
                {savingErr}
              </p>
            )}
          </div>
        )}
      </Modal>
    </section>
  );
}
