interface PaginationProps {
  page: number;
  totalPages: number;
  perPage: number;
  onPageChange: (page: number) => void;
  onPerPageChange: (perPage: number) => void;
}

/**
 * Build the list of page numbers to display.
 * Always shows first, last, current, and up to 2 neighbours.
 * Gaps are represented as null.
 */
function pageNumbers(current: number, total: number): (number | null)[] {
  if (total <= 7) {
    return Array.from({ length: total }, (_, i) => i + 1);
  }

  const pages = new Set<number>();
  pages.add(1);
  pages.add(total);
  for (let i = current - 1; i <= current + 1; i++) {
    if (i >= 1 && i <= total) pages.add(i);
  }

  const sorted = [...pages].sort((a, b) => a - b);
  const result: (number | null)[] = [];
  for (let i = 0; i < sorted.length; i++) {
    if (i > 0 && sorted[i] - sorted[i - 1] > 1) {
      result.push(null);
    }
    result.push(sorted[i]);
  }
  return result;
}

const btn =
  "px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 rounded-lg disabled:opacity-40 disabled:cursor-not-allowed hover:bg-gray-50 dark:hover:bg-gray-700 dark:text-gray-100 transition-colors";
const btnActive =
  "px-3 py-1.5 text-sm rounded-lg bg-gray-900 text-white dark:bg-gray-100 dark:text-gray-900 font-medium";

export default function Pagination({
  page,
  totalPages,
  perPage,
  onPageChange,
  onPerPageChange,
}: PaginationProps) {
  const pages = pageNumbers(page, totalPages);

  return (
    <div className="flex items-center justify-between">
      <div className="flex items-center gap-1">
        <button
          onClick={() => onPageChange(1)}
          disabled={page === 1}
          className={btn}
          aria-label="First page"
        >
          &laquo;
        </button>
        <button
          onClick={() => onPageChange(Math.max(1, page - 1))}
          disabled={page === 1}
          className={btn}
          aria-label="Previous page"
        >
          &lsaquo;
        </button>
        {pages.map((p, i) =>
          p === null ? (
            <span
              key={`gap-${i}`}
              className="px-1.5 text-sm text-gray-400 dark:text-gray-500 select-none"
            >
              &hellip;
            </span>
          ) : (
            <button
              key={p}
              onClick={() => onPageChange(p)}
              className={p === page ? btnActive : btn}
              aria-label={`Page ${p}`}
              aria-current={p === page ? "page" : undefined}
            >
              {p}
            </button>
          ),
        )}
        <button
          onClick={() => onPageChange(Math.min(totalPages, page + 1))}
          disabled={page >= totalPages}
          className={btn}
          aria-label="Next page"
        >
          &rsaquo;
        </button>
        <button
          onClick={() => onPageChange(totalPages)}
          disabled={page >= totalPages}
          className={btn}
          aria-label="Last page"
        >
          &raquo;
        </button>
      </div>
      <div className="flex items-center gap-2">
        <label htmlFor="page-size" className="sr-only">
          Rows per page
        </label>
        <select
          id="page-size"
          value={perPage}
          onChange={(e) => onPerPageChange(Number(e.target.value))}
          className="px-2 py-1.5 border border-gray-300 dark:border-gray-600 rounded-lg text-sm bg-white dark:bg-gray-800 dark:text-gray-100"
        >
          {[50, 100, 250, 500].map((n) => (
            <option key={n} value={n}>
              {n} / page
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
