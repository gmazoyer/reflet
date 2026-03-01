import { describe, it, expect } from "vitest";
import type { AsPathSegment, LookupResult } from "../api/types";
import {
  flattenAsPath,
  extractPeerPaths,
  buildDag,
  layoutDag,
  edgePath,
  peerColor,
  computeDagLayout,
  NODE_WIDTH,
  NODE_HEIGHT,
  COL_GAP,
  PADDING,
} from "./asPathDag";

// --- flattenAsPath ---

describe("flattenAsPath", () => {
  it("flattens Sequence-only segments into individual hops", () => {
    const segments: AsPathSegment[] = [
      { type: "Sequence", asns: [64496, 64497, 64498] },
    ];
    expect(flattenAsPath(segments)).toEqual([64496, 64497, 64498]);
  });

  it("flattens Set-only segments into a single hop array", () => {
    const segments: AsPathSegment[] = [
      { type: "Set", asns: [64498, 64496] },
    ];
    // Sorted
    expect(flattenAsPath(segments)).toEqual([[64496, 64498]]);
  });

  it("handles mixed Sequence and Set segments", () => {
    const segments: AsPathSegment[] = [
      { type: "Sequence", asns: [64496, 64497] },
      { type: "Set", asns: [64499, 64498] },
    ];
    expect(flattenAsPath(segments)).toEqual([64496, 64497, [64498, 64499]]);
  });

  it("returns empty array for empty segments", () => {
    expect(flattenAsPath([])).toEqual([]);
  });
});

// --- extractPeerPaths ---

function makeResult(peerId: string, paths: AsPathSegment[][]): LookupResult {
  return {
    peer_id: peerId,
    peer_name: peerId,
    matched_prefix: null,
    routes: paths.map((as_path) => ({
      prefix: "10.0.0.0/24",
      path_id: null,
      origin: "IGP" as const,
      as_path,
      next_hop: "10.0.0.1",
      med: null,
      local_pref: null,
      communities: [],
      ext_communities: [],
      large_communities: [],
      origin_as: null,
      received_at: "",
    })),
  };
}

describe("extractPeerPaths", () => {
  it("extracts paths from multiple peers", () => {
    const results = [
      makeResult("peer1", [[{ type: "Sequence", asns: [64496, 64497] }]]),
      makeResult("peer2", [[{ type: "Sequence", asns: [64496, 64498] }]]),
    ];
    const paths = extractPeerPaths(results);
    expect(paths).toHaveLength(2);
    expect(paths[0].peerId).toBe("peer1");
    expect(paths[1].peerId).toBe("peer2");
  });

  it("deduplicates identical paths from the same peer", () => {
    const seg: AsPathSegment[] = [{ type: "Sequence", asns: [64496, 64497] }];
    const results = [makeResult("peer1", [seg, seg])];
    const paths = extractPeerPaths(results);
    expect(paths).toHaveLength(1);
  });

  it("filters out routes with empty AS paths", () => {
    const results = [makeResult("peer1", [[]])];
    const paths = extractPeerPaths(results);
    expect(paths).toHaveLength(0);
  });
});

// --- buildDag ---

describe("buildDag", () => {
  it("merges shared nodes for two peers with the same path", () => {
    const paths = [
      { peerId: "peer1", hops: [64496, 64497] },
      { peerId: "peer2", hops: [64496, 64497] },
    ];
    const { nodes, edges } = buildDag(paths);

    // Same ASN at same column → shared node
    expect(nodes.size).toBe(2);
    expect(edges.size).toBe(1);

    const edge = Array.from(edges.values())[0];
    expect(edge.peerIds).toContain("peer1");
    expect(edge.peerIds).toContain("peer2");
  });

  it("creates separate nodes when paths diverge", () => {
    const paths = [
      { peerId: "peer1", hops: [64496, 64497] },
      { peerId: "peer2", hops: [64496, 64498] },
    ];
    const { nodes, edges } = buildDag(paths);

    // Column 0: one shared node (64496), Column 1: two separate nodes
    expect(nodes.size).toBe(3);
    expect(edges.size).toBe(2);
  });

  it("creates separate nodes for prepending (same ASN at different columns)", () => {
    const paths = [
      { peerId: "peer1", hops: [64496, 64496, 64497] },
    ];
    const { nodes } = buildDag(paths);

    // 64496 at col 0, 64496 at col 1, 64497 at col 2
    expect(nodes.size).toBe(3);
    expect(nodes.has("asn-0-64496")).toBe(true);
    expect(nodes.has("asn-1-64496")).toBe(true);
    expect(nodes.has("asn-2-64497")).toBe(true);
  });

  it("handles AS_SET hops", () => {
    const paths = [
      { peerId: "peer1", hops: [64496, [64497, 64498] as number[]] },
    ];
    const { nodes } = buildDag(paths);
    expect(nodes.has("set-1-64497,64498")).toBe(true);
  });
});

// --- layoutDag ---

describe("layoutDag", () => {
  it("assigns correct column positions", () => {
    const paths = [{ peerId: "peer1", hops: [64496, 64497, 64498] }];
    const { nodes: nodeMap, edges: edgeMap } = buildDag(paths);
    const layout = layoutDag(nodeMap, edgeMap);

    expect(layout.nodes).toHaveLength(3);

    const sorted = [...layout.nodes].sort((a, b) => a.column - b.column);
    expect(sorted[0].column).toBe(0);
    expect(sorted[1].column).toBe(1);
    expect(sorted[2].column).toBe(2);

    // x positions increase with column
    expect(sorted[0].x).toBe(PADDING);
    expect(sorted[1].x).toBe(PADDING + NODE_WIDTH + COL_GAP);
    expect(sorted[2].x).toBe(PADDING + 2 * (NODE_WIDTH + COL_GAP));
  });

  it("centers single-node columns vertically", () => {
    const paths = [
      { peerId: "peer1", hops: [64496, 64497] },
      { peerId: "peer2", hops: [64496, 64498] },
    ];
    const { nodes: nodeMap, edges: edgeMap } = buildDag(paths);
    const layout = layoutDag(nodeMap, edgeMap);

    // Column 0 has 1 node, column 1 has 2 nodes
    const col0 = layout.nodes.filter((n) => n.column === 0);
    const col1 = layout.nodes.filter((n) => n.column === 1);
    expect(col0).toHaveLength(1);
    expect(col1).toHaveLength(2);

    // Col0 node should be vertically centered relative to col1 nodes
    const col1MinY = Math.min(...col1.map((n) => n.y));
    const col1MaxY = Math.max(...col1.map((n) => n.y + NODE_HEIGHT));
    const col1Center = (col1MinY + col1MaxY) / 2;
    const col0Center = col0[0].y + NODE_HEIGHT / 2;
    expect(Math.abs(col0Center - col1Center)).toBeLessThan(1);
  });

  it("computes width and height with padding", () => {
    const paths = [{ peerId: "peer1", hops: [64496] }];
    const { nodes: nodeMap, edges: edgeMap } = buildDag(paths);
    const layout = layoutDag(nodeMap, edgeMap);

    expect(layout.width).toBe(PADDING + NODE_WIDTH + PADDING);
    expect(layout.height).toBe(PADDING + NODE_HEIGHT + PADDING);
  });
});

// --- edgePath ---

describe("edgePath", () => {
  it("returns a valid SVG path string", () => {
    const source: Parameters<typeof edgePath>[0] = {
      id: "a",
      column: 0,
      row: 0,
      asns: [64496],
      x: 40,
      y: 40,
    };
    const target: Parameters<typeof edgePath>[0] = {
      id: "b",
      column: 1,
      row: 0,
      asns: [64497],
      x: 200,
      y: 40,
    };
    const path = edgePath(source, target, 0, 1);
    expect(path).toMatch(/^M\s/);
    expect(path).toContain("C");
  });

  it("applies fan-out offset for multiple edges", () => {
    const source = { id: "a", column: 0, row: 0, asns: [64496], x: 40, y: 40 };
    const target = { id: "b", column: 1, row: 0, asns: [64497], x: 200, y: 40 };

    const path0 = edgePath(source, target, 0, 3);
    const path1 = edgePath(source, target, 1, 3);
    const path2 = edgePath(source, target, 2, 3);

    // All should be different
    expect(path0).not.toBe(path1);
    expect(path1).not.toBe(path2);
  });
});

// --- peerColor ---

describe("peerColor", () => {
  it("returns a valid hex color", () => {
    expect(peerColor(0)).toMatch(/^#[0-9a-f]{6}$/);
    expect(peerColor(5)).toMatch(/^#[0-9a-f]{6}$/);
  });

  it("cycles after 12 colors", () => {
    expect(peerColor(0)).toBe(peerColor(12));
    expect(peerColor(1)).toBe(peerColor(13));
  });
});

// --- computeDagLayout (integration) ---

describe("computeDagLayout", () => {
  it("returns empty layout for no results", () => {
    const layout = computeDagLayout([]);
    expect(layout.nodes).toHaveLength(0);
    expect(layout.edges).toHaveLength(0);
  });

  it("produces a valid layout for multiple peers", () => {
    const results = [
      makeResult("peer1", [[{ type: "Sequence", asns: [64496, 64497, 64498] }]]),
      makeResult("peer2", [[{ type: "Sequence", asns: [64496, 64499, 64498] }]]),
    ];
    const layout = computeDagLayout(results);

    // 4 unique nodes: 64496@0, 64497@1, 64499@1, 64498@2
    expect(layout.nodes).toHaveLength(4);
    // 4 edges: peer1 has 2, peer2 has 2 (some may overlap)
    expect(layout.edges.length).toBeGreaterThanOrEqual(3);
    expect(layout.width).toBeGreaterThan(0);
    expect(layout.height).toBeGreaterThan(0);
  });
});
