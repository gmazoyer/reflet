import Tooltip from "./Tooltip";

interface CommunityBadgeProps {
  value: string;
  description?: string | null;
  onClick?: () => void;
}

export default function CommunityBadge({ value, description, onClick }: CommunityBadgeProps) {
  const clickable = !!onClick;
  const pill = description
    ? "bg-blue-50 text-blue-700 border-blue-200 dark:bg-blue-900/30 dark:text-blue-300 dark:border-blue-700 cursor-help"
    : clickable
      ? "bg-gray-50 text-gray-600 border-gray-200 dark:bg-gray-800 dark:text-gray-400 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 cursor-pointer"
      : "bg-gray-50 text-gray-600 border-gray-200 dark:bg-gray-800 dark:text-gray-400 dark:border-gray-600";

  const badge = (
    <span
      className={`inline-block font-mono text-xs px-1.5 py-0.5 rounded-full border ${pill}`}
      role={clickable ? "button" : undefined}
      tabIndex={clickable && !description ? 0 : undefined}
      onClick={onClick}
      onKeyDown={onClick ? (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onClick(); } } : undefined}
    >
      {value}
    </span>
  );

  if (description) {
    return (
      <Tooltip content={description}>
        {badge}
      </Tooltip>
    );
  }

  return badge;
}
