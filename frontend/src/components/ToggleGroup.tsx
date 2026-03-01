import type { ReactNode } from "react";

interface ToggleOption<T extends string> {
  value: T;
  label: ReactNode;
}

interface ToggleGroupProps<T extends string> {
  options: ToggleOption<T>[];
  value: T;
  onChange: (value: T) => void;
}

export default function ToggleGroup<T extends string>({
  options,
  value,
  onChange,
}: ToggleGroupProps<T>) {
  return (
    <div className="flex rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
      {options.map((option, i) => (
        <button
          key={option.value}
          onClick={() => onChange(option.value)}
          className={`px-4 py-1.5 text-sm font-medium ${i > 0 ? "border-l border-gray-200 dark:border-gray-700 " : ""}${
            value === option.value
              ? "bg-gray-900 text-white dark:bg-gray-100 dark:text-gray-900"
              : "bg-white text-gray-600 hover:bg-gray-50 dark:bg-gray-800 dark:text-gray-400 dark:hover:bg-gray-700"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
