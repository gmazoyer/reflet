import type { ReactNode } from "react";

interface TooltipProps {
  content: ReactNode;
  children: ReactNode;
}

/**
 * A hover/focus tooltip with an arrow, positioned above the trigger element.
 * Wrap the trigger element as children and pass tooltip content via `content`.
 */
export default function Tooltip({ content, children }: TooltipProps) {
  return (
    <span className="relative inline-flex items-center group" tabIndex={0}>
      {children}
      <span
        role="tooltip"
        className="pointer-events-none absolute bottom-full left-1/2 -translate-x-1/2 mb-1.5 px-2 py-1 rounded bg-gray-900 dark:bg-gray-100 text-white dark:text-gray-900 text-xs whitespace-nowrap opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 transition-opacity z-50"
      >
        {content}
        <span className="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-gray-900 dark:border-t-gray-100" />
      </span>
    </span>
  );
}
