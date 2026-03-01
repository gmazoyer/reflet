import type { AsPathSegment, LookupResult } from "../api/types";

// --- Types ---

/** A single hop in a flattened AS path: either a single ASN or an AS_SET (array). */
export type Hop = number | number[];

/** A peer's flattened path through the AS graph. */
export interface PeerPath {
  peerId: string;
  hops: Hop[];
}

/** A node in the DAG, representing one or more ASNs at a specific depth. */
export interface DagNode {
  id: string;
  column: number;
  row: number;
  asns: number[];
  x: number;
  y: number;
}

/** An edge between two DAG nodes, attributed to one or more peers. */
export interface DagEdge {
  sourceId: string;
  targetId: string;
  peerIds: string[];
}

/** The complete layout ready for SVG rendering. */
export interface DagLayout {
  nodes: DagNode[];
  edges: DagEdge[];
  width: number;
  height: number;
}

// --- Constants ---

export const NODE_WIDTH = 100;
export const NODE_HEIGHT = 40;
export const COL_GAP = 60;
export const ROW_GAP = 20;
export const PADDING = 40;

// --- Color palette ---

const PEER_COLORS = [
  "#2563eb", // blue
  "#dc2626", // red
  "#16a34a", // green
  "#d97706", // amber
  "#7c3aed", // violet
  "#db2777", // pink
  "#0891b2", // cyan
  "#ea580c", // orange
  "#0d9488", // teal
  "#4f46e5", // indigo
  "#65a30d", // lime
  "#e11d48", // rose
];

/** Get a color for a peer by index, cycling through the palette. */
export function peerColor(index: number): string {
  return PEER_COLORS[((index % PEER_COLORS.length) + PEER_COLORS.length) % PEER_COLORS.length];
}

// --- Algorithm functions ---

/** Flatten AS path segments into hops. Sequence ASNs become individual hops; AS_SET becomes one hop. */
export function flattenAsPath(segments: AsPathSegment[]): Hop[] {
  const hops: Hop[] = [];
  for (const seg of segments) {
    if (seg.type === "Sequence") {
      for (const asn of seg.asns) {
        hops.push(asn);
      }
    } else {
      // AS_SET — sorted for canonical ordering
      hops.push([...seg.asns].sort((a, b) => a - b));
    }
  }
  return hops;
}

/** Serialize a hop for deduplication/comparison. */
function hopKey(hop: Hop): string {
  return Array.isArray(hop) ? `{${hop.join(",")}}` : String(hop);
}

/** Serialize a full path for deduplication. */
function pathKey(hops: Hop[]): string {
  return hops.map(hopKey).join("|");
}

/** Extract and deduplicate peer paths from lookup results. */
export function extractPeerPaths(results: LookupResult[]): PeerPath[] {
  const paths: PeerPath[] = [];
  for (const result of results) {
    const seen = new Set<string>();
    for (const route of result.routes) {
      const hops = flattenAsPath(route.as_path);
      if (hops.length === 0) continue;
      const key = pathKey(hops);
      if (seen.has(key)) continue;
      seen.add(key);
      paths.push({ peerId: result.peer_id, hops });
    }
  }
  return paths;
}

/** Canonical node ID: includes column so prepended ASNs get separate nodes. */
function nodeId(column: number, hop: Hop): string {
  return Array.isArray(hop)
    ? `set-${column}-${hop.join(",")}`
    : `asn-${column}-${hop}`;
}

/** Build a DAG by merging overlapping nodes across peer paths. */
export function buildDag(
  peerPaths: PeerPath[],
): { nodes: Map<string, { column: number; asns: number[] }>; edges: Map<string, { sourceId: string; targetId: string; peerIds: string[] }> } {
  const nodes = new Map<string, { column: number; asns: number[] }>();
  const edges = new Map<string, { sourceId: string; targetId: string; peerIds: string[] }>();

  for (const { peerId, hops } of peerPaths) {
    let prevId: string | null = null;
    for (let col = 0; col < hops.length; col++) {
      const hop = hops[col];
      const id = nodeId(col, hop);

      if (!nodes.has(id)) {
        const asns = Array.isArray(hop) ? hop : [hop];
        nodes.set(id, { column: col, asns });
      }

      if (prevId !== null) {
        const edgeKey = `${prevId}->${id}`;
        const existing = edges.get(edgeKey);
        if (existing) {
          if (!existing.peerIds.includes(peerId)) {
            existing.peerIds.push(peerId);
          }
        } else {
          edges.set(edgeKey, { sourceId: prevId, targetId: id, peerIds: [peerId] });
        }
      }

      prevId = id;
    }
  }

  return { nodes, edges };
}

/** Assign row positions using barycenter ordering to reduce edge crossings. */
export function layoutDag(
  nodeMap: Map<string, { column: number; asns: number[] }>,
  edgeMap: Map<string, { sourceId: string; targetId: string; peerIds: string[] }>,
): DagLayout {
  // Group nodes by column
  const columns = new Map<number, string[]>();
  for (const [id, node] of nodeMap) {
    const list = columns.get(node.column) ?? [];
    list.push(id);
    columns.set(node.column, list);
  }

  const maxColumn = columns.size > 0 ? Math.max(...columns.keys()) : 0;

  // Build adjacency: for each node, track predecessor rows
  const nodeRows = new Map<string, number>();

  // Initial row assignment: arbitrary order within each column
  for (const [, ids] of columns) {
    ids.forEach((id, i) => nodeRows.set(id, i));
  }

  // Build reverse adjacency (target -> sources)
  const predecessors = new Map<string, string[]>();
  for (const edge of edgeMap.values()) {
    const preds = predecessors.get(edge.targetId) ?? [];
    preds.push(edge.sourceId);
    predecessors.set(edge.targetId, preds);
  }

  // Barycenter ordering: iterate a few times to stabilize
  for (let iter = 0; iter < 4; iter++) {
    for (let col = 1; col <= maxColumn; col++) {
      const ids = columns.get(col);
      if (!ids || ids.length <= 1) continue;

      // Compute barycenter for each node
      const barycenters = ids.map((id) => {
        const preds = predecessors.get(id) ?? [];
        if (preds.length === 0) return { id, bc: 0 };
        const sum = preds.reduce((s, p) => s + (nodeRows.get(p) ?? 0), 0);
        return { id, bc: sum / preds.length };
      });

      barycenters.sort((a, b) => a.bc - b.bc);
      barycenters.forEach(({ id }, i) => nodeRows.set(id, i));
      columns.set(col, barycenters.map(({ id }) => id));
    }
  }

  // Find max rows per column for centering
  const maxRows = new Map<number, number>();
  for (const [col, ids] of columns) {
    maxRows.set(col, ids.length);
  }
  const globalMaxRows = Math.max(1, ...maxRows.values());

  // Compute pixel positions
  const dagNodes: DagNode[] = [];
  for (const [id, node] of nodeMap) {
    const row = nodeRows.get(id) ?? 0;
    const colSize = maxRows.get(node.column) ?? 1;
    const colHeight = colSize * NODE_HEIGHT + (colSize - 1) * ROW_GAP;
    const totalHeight = globalMaxRows * NODE_HEIGHT + (globalMaxRows - 1) * ROW_GAP;
    const yOffset = (totalHeight - colHeight) / 2;

    dagNodes.push({
      id,
      column: node.column,
      row,
      asns: node.asns,
      x: PADDING + node.column * (NODE_WIDTH + COL_GAP),
      y: PADDING + yOffset + row * (NODE_HEIGHT + ROW_GAP),
    });
  }

  const dagEdges: DagEdge[] = Array.from(edgeMap.values());

  const width =
    dagNodes.length > 0
      ? Math.max(...dagNodes.map((n) => n.x + NODE_WIDTH)) + PADDING
      : PADDING * 2;
  const height =
    dagNodes.length > 0
      ? Math.max(...dagNodes.map((n) => n.y + NODE_HEIGHT)) + PADDING
      : PADDING * 2;

  return { nodes: dagNodes, edges: dagEdges, width, height };
}

/** Compute an SVG cubic bezier path between two nodes, with fan-out offset for overlapping edges. */
export function edgePath(
  source: DagNode,
  target: DagNode,
  edgeIndex: number,
  totalEdges: number,
): string {
  const spread = 3;
  const offset = (edgeIndex - (totalEdges - 1) / 2) * spread;

  const x1 = source.x + NODE_WIDTH;
  const y1 = source.y + NODE_HEIGHT / 2 + offset;
  const x2 = target.x;
  const y2 = target.y + NODE_HEIGHT / 2 + offset;

  const cx1 = x1 + (x2 - x1) * 0.4;
  const cx2 = x2 - (x2 - x1) * 0.4;

  return `M ${x1} ${y1} C ${cx1} ${y1}, ${cx2} ${y2}, ${x2} ${y2}`;
}

/** Public entry point: compute a full DAG layout from lookup results. */
export function computeDagLayout(results: LookupResult[]): DagLayout {
  const peerPaths = extractPeerPaths(results);
  if (peerPaths.length === 0) {
    return { nodes: [], edges: [], width: PADDING * 2, height: PADDING * 2 };
  }
  const { nodes, edges } = buildDag(peerPaths);
  return layoutDag(nodes, edges);
}
