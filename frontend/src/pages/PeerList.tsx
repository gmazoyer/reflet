import { Link } from "react-router-dom";
import { usePeers } from "../hooks/useApi";
import type { PeerInfo } from "../api/types";
import { stateColors, defaultStateColor } from "../utils/stateColors";
import { isHiddenAddress } from "../utils/hiddenAddress";

export default function PeerList() {
  const { data: peers, isLoading, error } = usePeers();

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-gray-500 dark:text-gray-400">Loading peers...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-4 text-red-700 dark:text-red-400">
        Failed to load peers: {(error as Error).message}
      </div>
    );
  }

  const showAddress = peers ? peers.some((p) => !isHiddenAddress(p.address)) : true;

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">BGP Peers</h1>
      <div className="bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
        <table className="w-full text-left">
          <thead className="bg-gray-50 dark:bg-gray-800">
            <tr>
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                Name
              </th>
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                Description
              </th>
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                Location
              </th>
              {showAddress && (
                <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                  Address
                </th>
              )}
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                ASN
              </th>
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                State
              </th>
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                IPv4
              </th>
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                IPv6
              </th>
              <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                Uptime
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
            {peers && peers.length > 0 ? (
              peers.map((peer: PeerInfo) => (
                <tr key={peer.id} className="hover:bg-gray-50 dark:hover:bg-gray-700">
                  <td className="px-4 py-3 text-sm font-medium text-gray-900 dark:text-gray-100">
                    <Link
                      to={`/peers/${encodeURIComponent(peer.name)}/routes`}
                      className="text-blue-600 dark:text-blue-400 hover:underline"
                    >
                      {peer.name || peer.id}
                    </Link>
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-600 dark:text-gray-400">
                    {peer.description || "-"}
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-600 dark:text-gray-400">
                    {peer.location ?? "-"}
                  </td>
                  {showAddress && (
                    <td className="px-4 py-3 text-sm font-mono text-gray-600 dark:text-gray-400">
                      {peer.address}
                    </td>
                  )}
                  <td className="px-4 py-3 text-sm text-gray-600 dark:text-gray-400">
                    {peer.remote_asn}
                  </td>
                  <td className="px-4 py-3">
                    <span
                      className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium ${stateColors[peer.state] ?? defaultStateColor}`}
                    >
                      {peer.state}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-600 dark:text-gray-400">
                    {peer.prefixes.ipv4.toLocaleString()}
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-600 dark:text-gray-400">
                    {peer.prefixes.ipv6.toLocaleString()}
                  </td>
                  <td className="px-4 py-3 text-sm text-gray-500 dark:text-gray-400">
                    {peer.uptime ?? "N/A"}
                  </td>
                </tr>
              ))
            ) : (
              <tr>
                <td
                  colSpan={showAddress ? 9 : 8}
                  className="px-4 py-8 text-center text-gray-500 dark:text-gray-400 text-sm"
                >
                  No peers found.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
