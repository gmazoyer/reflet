import { Link } from "react-router-dom";
import type { PeerInfo } from "../api/types";
import { stateColors, defaultStateColor } from "../utils/stateColors";
import { isHiddenAddress } from "../utils/hiddenAddress";

function formatUptime(uptime: string | null): string {
  if (!uptime) return "N/A";
  return uptime;
}

export default function PeerCard({ peer }: { peer: PeerInfo }) {
  const colorClass = stateColors[peer.state] ?? defaultStateColor;

  return (
    <Link
      to={`/peers/${encodeURIComponent(peer.name)}/routes`}
      className="block bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4 hover:shadow-md transition-shadow"
    >
      <div className="flex items-center justify-between mb-2">
        <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100 truncate">
          {peer.name || peer.address}
        </h3>
        <span
          className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium ${colorClass}`}
        >
          {peer.state}
        </span>
      </div>
      <div className="text-xs text-gray-500 dark:text-gray-400 space-y-1">
        {(peer.description || peer.location) && (
          <div className="text-gray-400 dark:text-gray-500 truncate">
            {[peer.description, peer.location].filter(Boolean).join(" · ")}
          </div>
        )}
        <div className="flex justify-between">
          <span>AS {peer.remote_asn}</span>
          {!isHiddenAddress(peer.address) && <span>{peer.address}</span>}
        </div>
        <div className="flex justify-between">
          <span>
            IPv4: {peer.prefixes.ipv4.toLocaleString()} | IPv6:{" "}
            {peer.prefixes.ipv6.toLocaleString()}
          </span>
        </div>
        <div className="text-gray-400 dark:text-gray-500">
          Uptime: {formatUptime(peer.uptime)}
        </div>
      </div>
    </Link>
  );
}
