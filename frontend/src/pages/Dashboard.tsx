import { useNavigate } from "react-router-dom";
import { useSummary, usePeers } from "../hooks/useApi";
import PeerCard from "../components/PeerCard";
import PrefixSearch from "../components/PrefixSearch";

export default function Dashboard() {
  const navigate = useNavigate();
  const { data: summary, isLoading: summaryLoading } = useSummary();
  const { data: peers, isLoading: peersLoading } = usePeers();

  function handleLookup(prefix: string, type: "exact" | "longest-match" | "subnets") {
    navigate(`/lookup?prefix=${encodeURIComponent(prefix)}&type=${encodeURIComponent(type)}`);
  }

  if (summaryLoading || peersLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-gray-500 dark:text-gray-400">Loading...</div>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">Dashboard</h1>

      {summary && (
        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4">
          <StatCard label="ASN" value={summary.local_asn.toString()} />
          <StatCard
            label="Peers"
            value={`${summary.established_peers} / ${summary.peer_count}`}
          />
          <StatCard
            label="IPv4 Prefixes"
            value={summary.total_ipv4_prefixes.toLocaleString()}
          />
          <StatCard
            label="IPv6 Prefixes"
            value={summary.total_ipv6_prefixes.toLocaleString()}
          />
          {summary.rpki && (
            <StatCard
              label="RPKI VRPs"
              value={summary.rpki.vrp_count.toLocaleString()}
            />
          )}
        </div>
      )}

      <div>
        <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Prefix Lookup</h2>
        <PrefixSearch onSearch={handleLookup} />
      </div>

      <div>
        <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100 mb-4">Peers</h2>
        {peers && peers.length > 0 ? (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {peers.map((peer) => (
              <PeerCard key={peer.id} peer={peer} />
            ))}
          </div>
        ) : (
          <p className="text-gray-500 dark:text-gray-400">No peers configured.</p>
        )}
      </div>
    </div>
  );
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
      <dt className="text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
        {label}
      </dt>
      <dd className="mt-1 text-lg font-semibold text-gray-900 dark:text-gray-100">{value}</dd>
    </div>
  );
}
