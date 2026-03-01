import { useState, useEffect } from "react";
import { useParams, Link } from "react-router-dom";
import { usePeer, usePeerRoutes, useAsnInfo, useCommunityDefinitions, useRefreshPeer } from "../hooks/useApi";
import { isHiddenAddress } from "../utils/hiddenAddress";
import ClearButton from "../components/ClearButton";
import Pagination from "../components/Pagination";
import RouteTable from "../components/RouteTable";
import ToggleGroup from "../components/ToggleGroup";

export default function PeerRoutes() {
  const { id } = useParams<{ id: string }>();
  const [af, setAf] = useState<"ipv4" | "ipv6">("ipv4");
  const [page, setPage] = useState(1);
  const [search, setSearch] = useState("");
  const [searchInput, setSearchInput] = useState("");
  const [perPage, setPerPage] = useState(100);
  const [showFilterHelp, setShowFilterHelp] = useState(false);

  const [refreshMessage, setRefreshMessage] = useState<{ text: string; type: "success" | "error" } | null>(null);

  const { data: asnInfo } = useAsnInfo();
  const { data: communityDefs } = useCommunityDefinitions();
  const { data: peer, isLoading: peerLoading } = usePeer(id!);
  const refreshMutation = useRefreshPeer();
  const { data: routes, isLoading: routesLoading } = usePeerRoutes(
    id!,
    af,
    page,
    perPage,
    search || undefined,
  );

  useEffect(() => {
    if (refreshMessage) {
      const timer = setTimeout(() => setRefreshMessage(null), 3000);
      return () => clearTimeout(timer);
    }
  }, [refreshMessage]);

  function handleRefresh() {
    if (!id) return;
    refreshMutation.mutate(id, {
      onSuccess: () => setRefreshMessage({ text: "Route refresh requested", type: "success" }),
      onError: () => setRefreshMessage({ text: "Failed to request route refresh", type: "error" }),
    });
  }

  function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    setSearch(searchInput);
    setPage(1);
  }

  function handleAfChange(newAf: "ipv4" | "ipv6") {
    setAf(newAf);
    setPage(1);
    setSearch("");
    setSearchInput("");
  }

  const totalPages = routes ? Math.ceil(routes.meta.total / perPage) : 0;

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-2 text-sm text-gray-500 dark:text-gray-400">
        <Link to="/peers" className="hover:text-gray-700 dark:hover:text-gray-300">
          Peers
        </Link>
        <span>/</span>
        <span className="text-gray-900 dark:text-gray-100 font-medium">
          {peerLoading ? "..." : peer?.name || peer?.address || id}
        </span>
      </div>

      {peer && (
        <div className="bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg p-4 flex flex-wrap items-center gap-6 text-sm text-gray-900 dark:text-gray-100">
          {!isHiddenAddress(peer.address) && (
            <div>
              <span className="text-gray-500 dark:text-gray-400">Address:</span>{" "}
              <span className="font-mono">{peer.address}</span>
            </div>
          )}
          <div>
            <span className="text-gray-500 dark:text-gray-400">ASN:</span> {peer.remote_asn}
          </div>
          <div>
            <span className="text-gray-500 dark:text-gray-400">State:</span> {peer.state}
          </div>
          {!isHiddenAddress(peer.router_id) && (
            <div>
              <span className="text-gray-500 dark:text-gray-400">Router ID:</span>{" "}
              <span className="font-mono">{peer.router_id}</span>
            </div>
          )}
          {peer.location && (
            <div>
              <span className="text-gray-500 dark:text-gray-400">Location:</span> {peer.location}
            </div>
          )}
          <div className="ml-auto flex items-center gap-2">
            <button
              onClick={handleRefresh}
              disabled={refreshMutation.isPending || peer.state !== "Established"}
              className="px-3 py-1.5 text-sm font-medium rounded-lg border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              title={peer.state !== "Established" ? "Peer must be established to refresh" : "Request route refresh from peer"}
            >
              {refreshMutation.isPending ? (
                <span className="flex items-center gap-1.5">
                  <svg className="animate-spin h-3.5 w-3.5" viewBox="0 0 24 24" fill="none">
                    <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                    <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                  </svg>
                  Refreshing...
                </span>
              ) : (
                "Refresh Routes"
              )}
            </button>
            {refreshMessage && (
              <span className={`text-xs ${refreshMessage.type === "success" ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}`}>
                {refreshMessage.text}
              </span>
            )}
          </div>
        </div>
      )}

      <div className="flex flex-col sm:flex-row items-start sm:items-center gap-4">
        <ToggleGroup
          options={[
            {
              value: "ipv4" as const,
              label: <>IPv4{peer && <span className="ml-1 text-xs opacity-75">({peer.prefixes.ipv4.toLocaleString()})</span>}</>,
            },
            {
              value: "ipv6" as const,
              label: <>IPv6{peer && <span className="ml-1 text-xs opacity-75">({peer.prefixes.ipv6.toLocaleString()})</span>}</>,
            },
          ]}
          value={af}
          onChange={handleAfChange}
        />

        <form onSubmit={handleSearch} className="flex gap-2 flex-1">
          <div className="relative flex-1">
            <label htmlFor="route-filter" className="sr-only">Filter routes</label>
            <input
              id="route-filter"
              type="text"
              value={searchInput}
              onChange={(e) => setSearchInput(e.target.value)}
              placeholder="Filter: prefix, AS65001, community:65000:100, origin:igp, med:>100..."
              className="w-full px-3 py-2 pr-8 border border-gray-300 dark:border-gray-600 rounded-lg text-sm focus:ring-2 focus:ring-blue-500 focus:border-blue-500 outline-none bg-white dark:bg-gray-800 dark:text-gray-100"
            />
            {(searchInput || search) && (
              <ClearButton
                onClick={() => {
                  setSearchInput("");
                  setSearch("");
                  setPage(1);
                }}
                label="Clear search"
              />
            )}
          </div>
          <button
            type="submit"
            className="px-4 py-2 bg-blue-600 dark:bg-blue-500 text-white rounded-lg text-sm font-medium hover:bg-blue-700 dark:hover:bg-blue-600 transition-colors"
          >
            Filter
          </button>
          <button
            type="button"
            onClick={() => setShowFilterHelp((v) => !v)}
            className={`px-2.5 py-2 border rounded-lg text-sm font-medium transition-colors ${
              showFilterHelp
                ? "bg-gray-200 dark:bg-gray-600 border-gray-300 dark:border-gray-500 text-gray-700 dark:text-gray-200"
                : "border-gray-300 dark:border-gray-600 text-gray-500 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-700"
            }`}
            aria-label="Filter syntax help"
            title="Filter syntax help"
          >
            ?
          </button>
        </form>
      </div>

      {showFilterHelp && (
        <div className="bg-gray-50 dark:bg-gray-800/50 border border-gray-200 dark:border-gray-700 rounded-lg p-4 text-sm text-gray-700 dark:text-gray-300">
          <div className="font-medium mb-2 text-gray-900 dark:text-gray-100">Filter syntax</div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-6 gap-y-1">
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">10.0.0</span> prefix substring</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">AS65001</span> ASN in path</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">65000 65001</span> AS path subsequence</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">community:65000:100</span> standard community</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">community:65000:*</span> wildcard match</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">lc:65000:1:2</span> large community</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">origin:igp</span> origin (igp/egp/incomplete)</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">med:&gt;100</span> MED with comparison</div>
            <div><span className="font-mono text-xs bg-gray-200 dark:bg-gray-700 px-1 rounded">localpref:&gt;=200</span> local-pref with comparison</div>
            <div className="sm:col-span-2 mt-1 text-gray-500 dark:text-gray-400">Combine multiple filters with spaces for AND logic.</div>
          </div>
        </div>
      )}

      {routesLoading ? (
        <div className="flex items-center justify-center h-32">
          <div className="text-gray-500 dark:text-gray-400">Loading routes...</div>
        </div>
      ) : routes ? (
        <>
          <div className="text-sm text-gray-500 dark:text-gray-400">
            {routes.meta.total.toLocaleString()} routes total
            {search && (
              <span>
                {" "}
                (filtered by &quot;{search}&quot;)
              </span>
            )}
          </div>
          <RouteTable
            data={routes.data}
            communityDefinitions={communityDefs}
            asnInfo={asnInfo}
            onFilterByCommunity={(filter) => {
              setSearchInput(filter);
              setSearch(filter);
              setPage(1);
            }}
          />
          <Pagination
            page={page}
            totalPages={totalPages}
            perPage={perPage}
            onPageChange={setPage}
            onPerPageChange={(n) => {
              setPerPage(n);
              setPage(1);
            }}
          />
        </>
      ) : null}
    </div>
  );
}
