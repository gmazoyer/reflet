import type { RpkiStatus } from "../api/types";

const config: Record<RpkiStatus, { label: string; className: string }> = {
  valid: {
    label: "Valid",
    className: "bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400",
  },
  invalid: {
    label: "Invalid",
    className: "bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400",
  },
  not_found: {
    label: "Unknown",
    className: "bg-gray-100 text-gray-600 dark:bg-gray-700 dark:text-gray-400",
  },
};

export default function RpkiBadge({ status }: { status?: RpkiStatus }) {
  if (!status) return null;
  const { label, className } = config[status];
  return (
    <span className={`inline-block px-2 py-0.5 rounded text-xs font-medium ${className}`}>
      {label}
    </span>
  );
}
