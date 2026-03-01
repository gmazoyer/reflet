import { useRef, useMemo } from "react";
import {
  useReactTable,
  getCoreRowModel,
  flexRender,
  createColumnHelper,
} from "@tanstack/react-table";
import { useVirtualizer } from "@tanstack/react-virtual";
import type { AsnMap, BgpRoute, CommunityDefinitions } from "../api/types";
import { formatOrigin } from "../utils/format";
import AsPath from "./AsPath";
import CommunityList from "./CommunityList";
import RpkiBadge from "./RpkiBadge";
import Tooltip from "./Tooltip";

const columnHelper = createColumnHelper<BgpRoute>();

function buildColumns(communityDefinitions?: CommunityDefinitions, asnInfo?: AsnMap, showPathId?: boolean, showRpki?: boolean, onFilterByCommunity?: (filter: string) => void) {
  return [
    columnHelper.accessor("prefix", {
      header: "Prefix",
      cell: (info) => (
        <span className="font-mono text-sm text-gray-900 dark:text-gray-100">{info.getValue()}</span>
      ),
    }),
    ...(showPathId
      ? [
          columnHelper.accessor("path_id", {
            header: "Path ID",
            cell: (info) => (
              <span className="font-mono text-sm text-gray-600 dark:text-gray-400">
                {info.getValue() ?? "-"}
              </span>
            ),
          }),
        ]
      : []),
    ...(showRpki
      ? [
          columnHelper.accessor("rpki_status", {
            header: "RPKI",
            cell: (info) => <RpkiBadge status={info.getValue()} />,
          }),
        ]
      : []),
    columnHelper.accessor("next_hop", {
      header: "Next Hop",
      cell: (info) => (
        <span className="font-mono text-sm text-gray-600 dark:text-gray-400">{info.getValue()}</span>
      ),
    }),
    columnHelper.accessor("as_path", {
      header: "AS Path",
      cell: (info) => <AsPath segments={info.getValue()} asnInfo={asnInfo} />,
    }),
    columnHelper.accessor("origin", {
      header: "Origin",
      cell: (info) => {
        const { label, description } = formatOrigin(info.getValue());
        return (
          <Tooltip content={description}>
            <span className="font-mono text-sm text-gray-600 dark:text-gray-400 cursor-help">
              {label}
            </span>
          </Tooltip>
        );
      },
    }),
    columnHelper.accessor("med", {
      header: "MED",
      cell: (info) => (
        <span className="font-mono text-sm text-gray-600 dark:text-gray-400">{info.getValue() ?? "-"}</span>
      ),
    }),
    columnHelper.accessor("local_pref", {
      header: "Local Pref",
      cell: (info) => (
        <span className="font-mono text-sm text-gray-600 dark:text-gray-400">{info.getValue() ?? "-"}</span>
      ),
    }),
    columnHelper.display({
      id: "communities",
      header: "Communities",
      cell: (info) => {
        const route = info.row.original;
        return (
          <CommunityList
            communities={route.communities}
            extCommunities={route.ext_communities}
            largeCommunities={route.large_communities}
            definitions={communityDefinitions}
            onFilterByCommunity={onFilterByCommunity}
          />
        );
      },
    }),
    columnHelper.accessor("received_at", {
      header: "Age",
      cell: (info) => {
        const date = new Date(info.getValue());
        const now = new Date();
        const diffMs = now.getTime() - date.getTime();
        const diffSec = Math.floor(diffMs / 1000);
        let label: string;
        if (diffSec < 60) label = `${diffSec}s`;
        else {
          const diffMin = Math.floor(diffSec / 60);
          if (diffMin < 60) label = `${diffMin}m`;
          else {
            const diffHr = Math.floor(diffMin / 60);
            if (diffHr < 24) label = `${diffHr}h`;
            else label = `${Math.floor(diffHr / 24)}d`;
          }
        }
        return <span className="text-sm text-gray-600 dark:text-gray-400">{label}</span>;
      },
    }),
  ];
}

interface RouteTableProps {
  data: BgpRoute[];
  communityDefinitions?: CommunityDefinitions;
  asnInfo?: AsnMap;
  onFilterByCommunity?: (filter: string) => void;
}

export default function RouteTable({ data, communityDefinitions, asnInfo, onFilterByCommunity }: RouteTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const showPathId = useMemo(() => data.some((r) => r.path_id != null), [data]);
  const showRpki = useMemo(() => data.some((r) => r.rpki_status != null), [data]);
  const columns = useMemo(() => buildColumns(communityDefinitions, asnInfo, showPathId, showRpki, onFilterByCommunity), [communityDefinitions, asnInfo, showPathId, showRpki, onFilterByCommunity]);

  const table = useReactTable({
    data,
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  const { rows } = table.getRowModel();

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 40,
    overscan: 20,
  });

  const virtualItems = virtualizer.getVirtualItems();
  const totalSize = virtualizer.getTotalSize();

  // Spacer heights keep the scroll container at the correct total size
  // so the scrollbar position stays accurate.
  const paddingTop = virtualItems.length > 0 ? virtualItems[0].start : 0;
  const paddingBottom =
    virtualItems.length > 0
      ? totalSize - virtualItems[virtualItems.length - 1].end
      : 0;

  return (
    <div
      ref={parentRef}
      className="overflow-auto border border-gray-200 dark:border-gray-700 rounded-lg"
      style={{ maxHeight: "70vh" }}
    >
      <table className="w-full text-left">
        <thead className="bg-gray-50 dark:bg-gray-800 sticky top-0 z-10">
          {table.getHeaderGroups().map((headerGroup) => (
            <tr key={headerGroup.id}>
              {headerGroup.headers.map((header) => (
                <th
                  key={header.id}
                  scope="col"
                  className="px-3 py-2 text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider border-b border-gray-200 dark:border-gray-700"
                >
                  {header.isPlaceholder
                    ? null
                    : flexRender(
                        header.column.columnDef.header,
                        header.getContext(),
                      )}
                </th>
              ))}
            </tr>
          ))}
        </thead>
        <tbody>
          {virtualItems.length === 0 ? (
            <tr>
              <td
                colSpan={columns.length}
                className="px-3 py-8 text-center text-gray-500 dark:text-gray-400 text-sm"
              >
                No routes to display
              </td>
            </tr>
          ) : (
            <>
              {paddingTop > 0 && (
                <tr>
                  <td colSpan={columns.length} style={{ height: `${paddingTop}px`, padding: 0 }} />
                </tr>
              )}
              {virtualItems.map((virtualRow) => {
                const row = rows[virtualRow.index];
                return (
                  <tr
                    key={row.id}
                    className="hover:bg-gray-50 dark:hover:bg-gray-700 border-b border-gray-100 dark:border-gray-700"
                  >
                    {row.getVisibleCells().map((cell) => (
                      <td key={cell.id} className="px-3 py-2 whitespace-nowrap">
                        {flexRender(
                          cell.column.columnDef.cell,
                          cell.getContext(),
                        )}
                      </td>
                    ))}
                  </tr>
                );
              })}
              {paddingBottom > 0 && (
                <tr>
                  <td colSpan={columns.length} style={{ height: `${paddingBottom}px`, padding: 0 }} />
                </tr>
              )}
            </>
          )}
        </tbody>
      </table>
    </div>
  );
}
