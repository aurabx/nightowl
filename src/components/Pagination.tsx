import { ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight } from "lucide-react";

interface PaginationProps {
  /** Total number of matching rows. */
  total: number;
  /** Number of rows per page. */
  pageSize: number;
  /** Zero-based page index. */
  page: number;
  /** Called when the user navigates to a new page. */
  onPageChange: (page: number) => void;
}

/**
 * Numeric-pager control. Renders First / Prev / Page X of Y / Next /
 * Last buttons plus a row-range readout ("21–40 of 137").
 *
 * Hidden when there is at most one page.
 */
export function Pagination({ total, pageSize, page, onPageChange }: PaginationProps) {
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const safePage = Math.min(Math.max(0, page), pageCount - 1);
  const start = total === 0 ? 0 : safePage * pageSize + 1;
  const end = Math.min(total, safePage * pageSize + pageSize);

  if (pageCount <= 1) {
    return (
      <div className="flex items-center justify-end text-xs text-slate-500">
        {total === 0 ? "0 rows" : `${total} row${total === 1 ? "" : "s"}`}
      </div>
    );
  }

  const buttonClass =
    "rounded border border-slate-700 bg-slate-900 px-2 py-1 text-slate-300 " +
    "hover:border-slate-500 disabled:cursor-not-allowed disabled:opacity-40";

  return (
    <div className="flex items-center justify-between gap-3 text-xs text-slate-400">
      <span>
        {start.toLocaleString()}–{end.toLocaleString()} of {total.toLocaleString()}
      </span>
      <div className="flex items-center gap-1">
        <button
          type="button"
          className={buttonClass}
          onClick={() => onPageChange(0)}
          disabled={safePage === 0}
          title="First page"
        >
          <ChevronsLeft className="size-3.5" />
        </button>
        <button
          type="button"
          className={buttonClass}
          onClick={() => onPageChange(safePage - 1)}
          disabled={safePage === 0}
          title="Previous page"
        >
          <ChevronLeft className="size-3.5" />
        </button>
        <span className="px-2 text-slate-300">
          Page {safePage + 1} of {pageCount}
        </span>
        <button
          type="button"
          className={buttonClass}
          onClick={() => onPageChange(safePage + 1)}
          disabled={safePage >= pageCount - 1}
          title="Next page"
        >
          <ChevronRight className="size-3.5" />
        </button>
        <button
          type="button"
          className={buttonClass}
          onClick={() => onPageChange(pageCount - 1)}
          disabled={safePage >= pageCount - 1}
          title="Last page"
        >
          <ChevronsRight className="size-3.5" />
        </button>
      </div>
    </div>
  );
}
