import { useMemo, useState, useCallback } from "react";
import type { LookupResult, AsnMap } from "../api/types";
import {
  computeDagLayout,
  edgePath,
  peerColor,
  NODE_WIDTH,
  NODE_HEIGHT,
} from "../utils/asPathDag";
import type { DagNode } from "../utils/asPathDag";

interface AsPathGraphProps {
  results: LookupResult[];
  asnInfo?: AsnMap;
}

export default function AsPathGraph({ results, asnInfo }: AsPathGraphProps) {
  const layout = useMemo(() => computeDagLayout(results), [results]);

  // Map peer IDs to display names
  const peerNameMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const r of results) {
      if (!map.has(r.peer_id)) {
        map.set(r.peer_id, r.peer_name);
      }
    }
    return map;
  }, [results]);

  // Ordered unique peer IDs from edges
  const peerIndex = useMemo(() => {
    const seen = new Set<string>();
    const ordered: string[] = [];
    for (const edge of layout.edges) {
      for (const pid of edge.peerIds) {
        if (!seen.has(pid)) {
          seen.add(pid);
          ordered.push(pid);
        }
      }
    }
    return ordered;
  }, [layout.edges]);

  const peerIndexMap = useMemo(
    () => new Map(peerIndex.map((pid, i) => [pid, i])),
    [peerIndex],
  );

  const [hoveredNode, setHoveredNode] = useState<DagNode | null>(null);
  const [highlightedPeer, setHighlightedPeer] = useState<string | null>(null);

  const nodeMap = useMemo(
    () => new Map(layout.nodes.map((n) => [n.id, n])),
    [layout.nodes],
  );

  const togglePeerHighlight = useCallback((peerId: string) => {
    setHighlightedPeer((prev) => (prev === peerId ? null : peerId));
  }, []);

  if (layout.nodes.length === 0) {
    return (
      <div className="bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg p-8 text-center text-gray-500 dark:text-gray-400">
        No AS paths to visualize.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {/* Peer legend */}
      <div className="flex flex-wrap gap-2">
        {peerIndex.map((peerId, i) => {
          const color = peerColor(i);
          const isActive = highlightedPeer === null || highlightedPeer === peerId;
          return (
            <button
              key={peerId}
              onClick={() => togglePeerHighlight(peerId)}
              className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium border transition-opacity ${isActive
                  ? "border-gray-300 dark:border-gray-600 text-gray-900 dark:text-gray-100 bg-white dark:bg-gray-800"
                  : "border-gray-200 dark:border-gray-700 text-gray-400 dark:text-gray-500 bg-gray-50 dark:bg-gray-800/50 opacity-50"
                }`}
            >
              <span
                className="inline-block w-3 h-3 rounded-sm flex-shrink-0"
                style={{ backgroundColor: color }}
              />
              {peerNameMap.get(peerId) ?? peerId}
            </button>
          );
        })}
      </div>

      {/* Graph container */}
      <div className="bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg overflow-x-auto max-h-[60vh] overflow-y-auto">
        <div className="relative" style={{ width: layout.width, height: layout.height }}>
          <svg
            width={layout.width}
            height={layout.height}
            className="block"
          >
            {/* Edges (behind nodes) */}
            {layout.edges.map((edge) => {
              const source = nodeMap.get(edge.sourceId);
              const target = nodeMap.get(edge.targetId);
              if (!source || !target) return null;

              return edge.peerIds.map((peerId, i) => {
                const colorIdx = peerIndexMap.get(peerId) ?? 0;
                const isHighlighted =
                  highlightedPeer === null || highlightedPeer === peerId;
                return (
                  <path
                    key={`${edge.sourceId}-${edge.targetId}-${peerId}`}
                    d={edgePath(source, target, i, edge.peerIds.length)}
                    fill="none"
                    stroke={peerColor(colorIdx)}
                    strokeWidth={isHighlighted ? 2 : 1}
                    opacity={isHighlighted ? 0.8 : 0.15}
                    className="transition-opacity"
                  />
                );
              });
            })}

            {/* Nodes */}
            {layout.nodes.map((node) => {
              const isSet = node.asns.length > 1;
              const label = isSet
                ? `{ ${node.asns.join(", ")} }`
                : String(node.asns[0]);
              const rectWidth = isSet
                ? Math.max(NODE_WIDTH, label.length * 9 + 16)
                : NODE_WIDTH;

              return (
                <g
                  key={node.id}
                  onMouseEnter={() => setHoveredNode(node)}
                  onMouseLeave={() => setHoveredNode(null)}
                  className="cursor-pointer"
                >
                  <a
                    href={
                      !isSet
                        ? `https://bgp.tools/as/${node.asns[0]}`
                        : undefined
                    }
                    target="_blank"
                    rel="noopener noreferrer"
                    tabIndex={0}
                    role="link"
                    aria-label={
                      isSet
                        ? `AS Set: ${node.asns.join(", ")}`
                        : `AS${node.asns[0]}`
                    }
                  >
                    <rect
                      x={node.x}
                      y={node.y}
                      width={rectWidth}
                      height={NODE_HEIGHT}
                      rx={6}
                      className="fill-white dark:fill-gray-700 stroke-gray-300 dark:stroke-gray-500"
                      strokeWidth={hoveredNode?.id === node.id ? 2 : 1}
                      stroke={
                        hoveredNode?.id === node.id ? "#3b82f6" : undefined
                      }
                    />
                    <text
                      x={node.x + rectWidth / 2}
                      y={node.y + NODE_HEIGHT / 2}
                      textAnchor="middle"
                      dominantBaseline="central"
                      className="text-xs font-mono fill-gray-900 dark:fill-gray-100 pointer-events-none select-none"
                    >
                      {label}
                    </text>
                  </a>
                </g>
              );
            })}
          </svg>

          {/* Tooltip (HTML overlay) */}
          {hoveredNode && (() => {
            const asn = hoveredNode.asns[0];
            const info = asnInfo?.[String(asn)];
            if (!info && hoveredNode.asns.length === 1) return null;

            const tooltipText = hoveredNode.asns.length > 1
              ? `AS Set: ${hoveredNode.asns.map((a) => {
                const i = asnInfo?.[String(a)];
                return i ? `${a} (${i.name})` : String(a);
              }).join(", ")}`
              : `AS${asn} \u2014 ${info!.name}`;

            const isSet = hoveredNode.asns.length > 1;
            const rectWidth = isSet
              ? Math.max(NODE_WIDTH, `{ ${hoveredNode.asns.join(", ")} }`.length * 9 + 16)
              : NODE_WIDTH;

            return (
              <div
                className="absolute pointer-events-none z-50"
                style={{
                  left: hoveredNode.x + rectWidth / 2,
                  top: hoveredNode.y - 4,
                  transform: "translate(-50%, -100%)",
                }}
              >
                <div className="px-2 py-1 rounded bg-gray-900 dark:bg-gray-100 text-white dark:text-gray-900 text-xs whitespace-nowrap">
                  {tooltipText}
                  <span className="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-gray-900 dark:border-t-gray-100" />
                </div>
              </div>
            );
          })()}
        </div>
      </div>
    </div>
  );
}
