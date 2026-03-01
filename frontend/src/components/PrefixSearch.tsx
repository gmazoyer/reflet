import { useState } from "react";
import { useWhoami } from "../hooks/useApi";
import ClearButton from "./ClearButton";

interface PrefixSearchProps {
  onSearch: (prefix: string, type: "exact" | "longest-match" | "subnets") => void;
  isLoading?: boolean;
  initialPrefix?: string;
  initialType?: "exact" | "longest-match" | "subnets";
}

function isValidPrefix(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;

  // Simple validation: check IPv4 or IPv6 CIDR or plain address
  const ipv4Cidr = /^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(\/\d{1,2})?$/;
  const ipv6Cidr = /^[0-9a-fA-F:.]+(\/\d{1,3})?$/;

  return ipv4Cidr.test(trimmed) || ipv6Cidr.test(trimmed);
}

export default function PrefixSearch({ onSearch, isLoading, initialPrefix, initialType }: PrefixSearchProps) {
  const [prefix, setPrefix] = useState(initialPrefix ?? "");
  const [lookupType, setLookupType] = useState<"exact" | "longest-match" | "subnets">(
    initialType ?? "longest-match",
  );

  const { data: whoami } = useWhoami();

  const valid = isValidPrefix(prefix);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (valid) {
      onSearch(prefix.trim(), lookupType);
    }
  }

  function handleUseMyIp() {
    if (whoami) {
      setPrefix(whoami.ip);
    }
  }

  return (
    <div className="space-y-3">
      <form onSubmit={handleSubmit} className="flex flex-col sm:flex-row gap-3">
        <div className="relative flex-1">
          <label htmlFor="prefix-input" className="sr-only">Prefix</label>
          <input
            id="prefix-input"
            type="text"
            value={prefix}
            onChange={(e) => setPrefix(e.target.value)}
            placeholder="Enter prefix (e.g. 10.0.0.0/24 or 2001:db8::/32)"
            className="w-full px-4 py-2 pr-8 border border-gray-300 dark:border-gray-600 rounded-lg focus:ring-2 focus:ring-blue-500 focus:border-blue-500 outline-none text-sm bg-white dark:bg-gray-800 dark:text-gray-100"
          />
          {prefix && (
            <ClearButton onClick={() => setPrefix("")} label="Clear prefix" />
          )}
        </div>
        <label htmlFor="lookup-type" className="sr-only">Lookup type</label>
        <select
          id="lookup-type"
          value={lookupType}
          onChange={(e) =>
            setLookupType(e.target.value as "exact" | "longest-match" | "subnets")
          }
          className="px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg text-sm bg-white dark:bg-gray-800 dark:text-gray-100"
        >
          <option value="longest-match">Longest Match</option>
          <option value="exact">Exact</option>
          <option value="subnets">Subnets</option>
        </select>
        <button
          type="submit"
          disabled={!valid || isLoading}
          className="px-6 py-2 bg-blue-600 dark:bg-blue-500 text-white rounded-lg text-sm font-medium hover:bg-blue-700 dark:hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {isLoading ? "Searching..." : "Search"}
        </button>
      </form>
      {whoami && (
        <div className="flex items-center gap-2 text-sm text-gray-500 dark:text-gray-400">
          <span>Your IP:</span>
          <button
            type="button"
            onClick={handleUseMyIp}
            className="inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-xs font-mono font-medium bg-gray-100 text-gray-700 border border-gray-200 hover:bg-blue-50 hover:text-blue-700 hover:border-blue-200 dark:bg-gray-800 dark:text-gray-300 dark:border-gray-600 dark:hover:bg-blue-900/30 dark:hover:text-blue-300 dark:hover:border-blue-700 transition-colors cursor-pointer"
          >
            {whoami.ip}
          </button>
        </div>
      )}
    </div>
  );
}
