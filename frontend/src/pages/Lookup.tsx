import { useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useLookup, useAsnInfo, useCommunityDefinitions } from "../hooks/useApi";
import { formatOrigin } from "../utils/format";
import AsPath from "../components/AsPath";
import AsPathGraph from "../components/AsPathGraph";
import CommunityList from "../components/CommunityList";
import PrefixSearch from "../components/PrefixSearch";
import RpkiBadge from "../components/RpkiBadge";
import ToggleGroup from "../components/ToggleGroup";

type ViewMode = "table" | "graph";

const VALID_TYPES = ["exact", "longest-match", "subnets"] as const;
type LookupType = (typeof VALID_TYPES)[number];

function parseLookupType(value: string | null): LookupType {
  if (value && (VALID_TYPES as readonly string[]).includes(value))
    return value as LookupType;
  return "longest-match";
}

export default function Lookup() {
  const [searchParams, setSearchParams] = useSearchParams();

  const initialPrefix = searchParams.get("prefix") ?? "";
  const initialType = parseLookupType(searchParams.get("type"));

  const [query, setQuery] = useState(initialPrefix);
  const [lookupType, setLookupType] = useState<LookupType>(initialType);
  const [viewMode, setViewMode] = useState<ViewMode>("table");

  const { data: asnInfo } = useAsnInfo();
  const { data: communityDefs } = useCommunityDefinitions();
  const { data, isLoading, error } = useLookup(query, lookupType);

  function handleSearch(
    prefix: string,
    type: LookupType,
  ) {
    setQuery(prefix);
    setLookupType(type);
    setSearchParams({ prefix, type }, { replace: true });
  }

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">Prefix Lookup</h1>

      <PrefixSearch
        onSearch={handleSearch}
        isLoading={isLoading}
        initialPrefix={initialPrefix}
        initialType={initialType}
      />

      {error && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-4 text-red-700 dark:text-red-400 text-sm">
          Lookup failed: {(error as Error).message}
        </div>
      )}

      {data && (() => {
        const rows = data.results.flatMap((result) =>
          result.routes.map((route) => ({ ...result, route }))
        );
        const showPathId = rows.some((r) => r.route.path_id != null);
        const showRpki = rows.some((r) => r.route.rpki_status != null);
        const totalRoutes = data.results.reduce((sum, r) => sum + r.routes.length, 0);

        return (
          <div className="space-y-4">
            <div className="flex flex-col sm:flex-row sm:items-center gap-4">
              <div className="text-sm text-gray-500 dark:text-gray-400">
                Query: <span className="font-mono">{data.query}</span> | Type:{" "}
                {data.lookup_type} | Results: {totalRoutes}
              </div>

              {rows.length > 0 && (
                <ToggleGroup
                  options={[
                    { value: "table" as ViewMode, label: "Table" },
                    { value: "graph" as ViewMode, label: "Graph" },
                  ]}
                  value={viewMode}
                  onChange={setViewMode}
                />
              )}
            </div>

            {rows.length === 0 ? (
              <div className="bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg p-8 text-center text-gray-500 dark:text-gray-400">
                No matching routes found.
              </div>
            ) : viewMode === "graph" ? (
              <AsPathGraph results={data.results} asnInfo={asnInfo} />
            ) : (
              <div className="bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
                <table className="w-full text-left">
                  <thead className="bg-gray-50 dark:bg-gray-800">
                    <tr>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        Peer
                      </th>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        Matched Prefix
                      </th>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        Prefix
                      </th>
                      {showPathId && (
                        <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                          Path ID
                        </th>
                      )}
                      {showRpki && (
                        <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                          RPKI
                        </th>
                      )}
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        Next Hop
                      </th>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        AS Path
                      </th>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        Origin
                      </th>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        MED
                      </th>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        Local Pref
                      </th>
                      <th scope="col" className="px-4 py-3 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">
                        Communities
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-gray-100 dark:divide-gray-700">
                    {rows.map((row, i) => (
                      <tr key={i} className="hover:bg-gray-50 dark:hover:bg-gray-700">
                        <td className="px-4 py-3 text-sm text-gray-600 dark:text-gray-400">
                          {row.peer_name}
                        </td>
                        <td className="px-4 py-3 text-sm font-mono text-gray-600 dark:text-gray-400">
                          {row.matched_prefix ?? "-"}
                        </td>
                        <td className="px-4 py-3 text-sm font-mono text-gray-900 dark:text-gray-100">
                          {row.route.prefix}
                        </td>
                        {showPathId && (
                          <td className="px-4 py-3 text-sm font-mono text-gray-600 dark:text-gray-400">
                            {row.route.path_id ?? "-"}
                          </td>
                        )}
                        {showRpki && (
                          <td className="px-4 py-3">
                            <RpkiBadge status={row.route.rpki_status} />
                          </td>
                        )}
                        <td className="px-4 py-3 text-sm font-mono text-gray-600 dark:text-gray-400">
                          {row.route.next_hop}
                        </td>
                        <td className="px-4 py-3">
                          <AsPath segments={row.route.as_path} asnInfo={asnInfo} />
                        </td>
                        <td className="px-4 py-3 text-sm font-mono text-gray-600 dark:text-gray-400">
                          {formatOrigin(row.route.origin).label}
                        </td>
                        <td className="px-4 py-3 text-sm font-mono text-gray-600 dark:text-gray-400">
                          {row.route.med ?? "-"}
                        </td>
                        <td className="px-4 py-3 text-sm font-mono text-gray-600 dark:text-gray-400">
                          {row.route.local_pref ?? "-"}
                        </td>
                        <td className="px-4 py-3">
                          <CommunityList
                            communities={row.route.communities}
                            extCommunities={row.route.ext_communities}
                            largeCommunities={row.route.large_communities}
                            definitions={communityDefs}
                          />
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      })()}
    </div>
  );
}
