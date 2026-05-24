import { ClipboardList } from "lucide-react";

/**
 * M10 stub. The Modality Worklist (DMWL) SCP is on the roadmap but
 * not implemented in this iteration; the page exists so the future
 * implementation drops in without re-arranging the sidebar.
 */
export function WorklistPage() {
  return (
    <section className="max-w-2xl">
      <h1 className="text-2xl font-semibold">Worklist</h1>
      <p className="mt-1 text-sm text-slate-400">
        Modality Worklist (DMWL) management.
      </p>

      <div className="mt-8 rounded border border-slate-800 bg-slate-900/40 p-8 text-center">
        <ClipboardList className="mx-auto size-8 text-slate-700" />
        <p className="mt-3 text-sm text-slate-300">
          Modality Worklist support is planned.
        </p>
        <p className="mt-1 text-xs text-slate-500">
          A DMWL SCP responds to N-FIND queries for scheduled procedure steps.
          Not yet implemented in this iteration.
        </p>
      </div>
    </section>
  );
}
