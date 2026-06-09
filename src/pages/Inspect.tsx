import { useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileSearch,
  Loader2,
  UploadCloud,
} from "lucide-react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  type DicomElement,
  type DicomFileProperties,
  formatError,
  readDicomFile,
} from "../lib/api";

const KB = 1024;
const MB = 1024 * KB;

function humanBytes(n: number): string {
  if (n >= MB) return `${(n / MB).toFixed(1)} MB`;
  if (n >= KB) return `${(n / KB).toFixed(0)} KB`;
  return `${n} B`;
}

/** Counts an element subtree, including nested sequence items. */
function countElements(elements: DicomElement[]): number {
  return elements.reduce(
    (sum, el) =>
      sum +
      1 +
      (el.items
        ? el.items.reduce((m, item) => m + countElements(item.elements), 0)
        : 0),
    0,
  );
}

/** Returns a copy of `el` if it (or any descendant) matches `needle`,
 * otherwise null. A matching sequence keeps all of its items; a sequence
 * kept only because a descendant matched is pruned to the matches. */
function filterElement(el: DicomElement, needle: string): DicomElement | null {
  const selfMatch =
    el.tag.toLowerCase().includes(needle) ||
    el.name.toLowerCase().includes(needle) ||
    el.value.toLowerCase().includes(needle);

  if (!el.items) {
    return selfMatch ? el : null;
  }

  if (selfMatch) {
    return el;
  }

  const items = el.items
    .map((item) => ({
      elements: item.elements
        .map((child) => filterElement(child, needle))
        .filter((x): x is DicomElement => x !== null),
    }))
    .filter((item) => item.elements.length > 0);

  return items.length > 0 ? { ...el, items } : null;
}

export function InspectPage() {
  const [dragging, setDragging] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [props, setProps] = useState<DicomFileProperties | null>(null);
  const [filter, setFilter] = useState("");

  // Tauri delivers OS file drops as window-level events carrying
  // absolute paths. Registering the listener only while this page is
  // mounted scopes "drop to inspect" to the inspector view; the cleanup
  // unlistens when the user navigates away.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") {
          setDragging(true);
        } else if (payload.type === "leave") {
          setDragging(false);
        } else if (payload.type === "drop") {
          setDragging(false);
          const [first] = payload.paths;
          if (first) {
            void inspect(first);
          }
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        setError(formatError(err));
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  async function inspect(path: string) {
    setLoading(true);
    setError(null);
    try {
      const result = await readDicomFile(path);
      setProps(result);
    } catch (err) {
      setProps(null);
      setError(formatError(err));
    } finally {
      setLoading(false);
    }
  }

  const needle = filter.trim().toLowerCase();

  const visibleElements = useMemo(() => {
    if (!props) return [];
    if (!needle) return props.elements;
    return props.elements
      .map((el) => filterElement(el, needle))
      .filter((x): x is DicomElement => x !== null);
  }, [props, needle]);

  const totalCount = props ? countElements(props.elements) : 0;
  const visibleCount = countElements(visibleElements);

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <FileSearch className="size-6 text-slate-300" />
        <div>
          <h1 className="text-xl font-semibold">Inspect</h1>
          <p className="text-sm text-slate-400">
            Drop a DICOM file anywhere on this page to read its properties.
          </p>
        </div>
      </div>

      <div
        className={
          "flex flex-col items-center justify-center gap-3 rounded-lg border-2 border-dashed px-6 py-12 text-center transition-colors " +
          (dragging
            ? "border-sky-400 bg-sky-500/10 text-sky-200"
            : "border-slate-700 bg-slate-900/50 text-slate-400")
        }
      >
        {loading ? (
          <Loader2 className="size-8 animate-spin text-slate-300" />
        ) : (
          <UploadCloud className="size-8" />
        )}
        <div className="text-sm">
          {loading
            ? "Reading file…"
            : "Drag a DICOM Part-10 file here to inspect it."}
        </div>
      </div>

      {error && (
        <div className="rounded-md border border-red-900 bg-red-950/50 px-4 py-3 text-sm text-red-300">
          {error}
        </div>
      )}

      {props && (
        <div className="space-y-4">
          <div className="rounded-lg border border-slate-800 bg-slate-900/50 p-4">
            <h2 className="mb-3 text-sm font-semibold text-slate-200">
              {props.file_name}
            </h2>
            <dl className="grid grid-cols-1 gap-x-6 gap-y-1 text-sm sm:grid-cols-2">
              <MetaRow label="Path" value={props.file_path} mono />
              <MetaRow label="Size" value={humanBytes(props.size_bytes)} />
              <MetaRow
                label="Transfer syntax"
                value={props.transfer_syntax_uid}
                mono
              />
              <MetaRow
                label="SOP class UID"
                value={props.media_storage_sop_class_uid}
                mono
              />
              <MetaRow
                label="SOP instance UID"
                value={props.media_storage_sop_instance_uid}
                mono
              />
              <MetaRow label="Elements" value={String(props.element_count)} />
            </dl>
          </div>

          <div className="flex items-center justify-between gap-3">
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter by tag, name or value…"
              className="w-full max-w-sm rounded-md border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm text-slate-100 placeholder:text-slate-500 focus:border-sky-500 focus:outline-none"
            />
            <span className="shrink-0 text-xs text-slate-500">
              {visibleCount} of {totalCount} elements
            </span>
          </div>

          <div className="overflow-hidden rounded-lg border border-slate-800">
            <table className="w-full text-left text-sm">
              <thead className="bg-slate-900 text-xs uppercase text-slate-400">
                <tr>
                  <th className="px-3 py-2 font-medium">Tag</th>
                  <th className="px-3 py-2 font-medium">Name</th>
                  <th className="px-3 py-2 font-medium">VR</th>
                  <th className="px-3 py-2 text-right font-medium">Length</th>
                  <th className="px-3 py-2 font-medium">Value</th>
                </tr>
              </thead>
              <tbody>
                {visibleElements.map((el, i) => (
                  <ElementRows
                    key={`${el.tag}-${i}`}
                    element={el}
                    depth={0}
                    rowKey={`${el.tag}-${i}`}
                    expandAll={needle.length > 0}
                  />
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}

/** Renders one element row and, for an expanded sequence, its nested
 * item and child rows. Each sequence row toggles its own open state;
 * when `expandAll` is set (an active filter) every row is forced open. */
function ElementRows({
  element,
  depth,
  rowKey,
  expandAll,
}: {
  element: DicomElement;
  depth: number;
  rowKey: string;
  expandAll: boolean;
}) {
  const [open, setOpen] = useState(false);
  const isSequence = !!element.items;
  const expanded = expandAll || open;
  // 1rem of indent per level, applied as left padding on the Name cell.
  const indent = { paddingLeft: `${depth * 1 + 0.75}rem` };

  return (
    <>
      <tr className="border-t border-slate-800 align-top hover:bg-slate-900/40">
        <td className="whitespace-nowrap px-3 py-1.5 font-mono text-xs text-slate-400">
          {element.tag}
        </td>
        <td className="py-1.5 pr-3 text-slate-200" style={indent}>
          {isSequence ? (
            <button
              type="button"
              onClick={() => setOpen((v) => !v)}
              disabled={expandAll}
              className="flex items-center gap-1 text-left hover:text-white disabled:cursor-default"
            >
              {expanded ? (
                <ChevronDown className="size-3.5 shrink-0 text-slate-500" />
              ) : (
                <ChevronRight className="size-3.5 shrink-0 text-slate-500" />
              )}
              {element.name}
            </button>
          ) : (
            element.name
          )}
        </td>
        <td className="px-3 py-1.5 font-mono text-xs text-slate-400">
          {element.vr}
        </td>
        <td className="px-3 py-1.5 text-right font-mono text-xs text-slate-500">
          {element.length ?? "—"}
        </td>
        <td className="px-3 py-1.5 font-mono text-xs text-slate-300 break-all">
          {element.value}
        </td>
      </tr>

      {isSequence &&
        expanded &&
        element.items!.map((item, itemIndex) => (
          <ItemRows
            key={`${rowKey}-item${itemIndex}`}
            item={item}
            itemIndex={itemIndex}
            depth={depth}
            rowKey={`${rowKey}-item${itemIndex}`}
            expandAll={expandAll}
          />
        ))}
    </>
  );
}

/** Renders a sequence item's label row followed by its element rows. */
function ItemRows({
  item,
  itemIndex,
  depth,
  rowKey,
  expandAll,
}: {
  item: { elements: DicomElement[] };
  itemIndex: number;
  depth: number;
  rowKey: string;
  expandAll: boolean;
}) {
  return (
    <>
      <tr className="border-t border-slate-800/60 bg-slate-900/30">
        <td className="px-3 py-1 text-xs text-slate-600" />
        <td
          className="py-1 pr-3 text-xs uppercase tracking-wide text-slate-500"
          style={{ paddingLeft: `${(depth + 1) * 1 + 0.75}rem` }}
          colSpan={4}
        >
          Item {itemIndex + 1}
        </td>
      </tr>
      {item.elements.map((child, i) => (
        <ElementRows
          key={`${rowKey}-${child.tag}-${i}`}
          element={child}
          depth={depth + 2}
          rowKey={`${rowKey}-${child.tag}-${i}`}
          expandAll={expandAll}
        />
      ))}
    </>
  );
}

function MetaRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="flex gap-2">
      <dt className="shrink-0 text-slate-500">{label}:</dt>
      <dd
        className={
          "min-w-0 break-all text-slate-200 " + (mono ? "font-mono text-xs" : "")
        }
      >
        {value || "—"}
      </dd>
    </div>
  );
}
