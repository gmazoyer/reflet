import { describe, it, expect } from "vitest";
import type { CommunityDefinitions } from "../api/types";
import {
  annotateStandardCommunity,
  annotateLargeCommunity,
} from "./communityAnnotation";

function emptyDefs(): CommunityDefinitions {
  return { standard: {}, large: {}, patterns: [], ranges: [] };
}

// --- Exact match ---

describe("exact match", () => {
  it("matches a standard community exactly", () => {
    const defs = emptyDefs();
    defs.standard["1299:20500"] = "Amsterdam (Peer)";
    expect(
      annotateStandardCommunity({ asn: 1299, value: 20500 }, defs),
    ).toBe("Amsterdam (Peer)");
  });

  it("matches a large community exactly", () => {
    const defs = emptyDefs();
    defs.large["6695:1914:150"] = "Continent: Europe";
    expect(
      annotateLargeCommunity(
        { global_admin: 6695, local_data1: 1914, local_data2: 150 },
        defs,
      ),
    ).toBe("Continent: Europe");
  });

  it("returns null when no match", () => {
    const defs = emptyDefs();
    expect(
      annotateStandardCommunity({ asn: 9999, value: 1 }, defs),
    ).toBeNull();
  });
});

// --- Wildcard matching with captures ---

describe("wildcard matching with captures", () => {
  it("matches x wildcards and substitutes $0", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "1299:20xxx",
      description: "EU Peers $0",
      type: "standard",
    });
    expect(
      annotateStandardCommunity({ asn: 1299, value: 20500 }, defs),
    ).toBe("EU Peers 500");
  });

  it("matches mixed n wildcards (character-by-character) and substitutes $0", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "300:1nnnn",
      description: "Region $0",
      type: "standard",
    });
    expect(
      annotateStandardCommunity({ asn: 300, value: 12345 }, defs),
    ).toBe("Region 2345");
  });

  it("fully-wildcard segment matches variable-length numbers", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "201281:100:nnn",
      description: "Received from ASN$0",
      type: "large",
    });
    expect(
      annotateLargeCommunity(
        { global_admin: 201281, local_data1: 100, local_data2: 204092 },
        defs,
      ),
    ).toBe("Received from ASN204092");
  });

  it("fully-wildcard segment matches short numbers too", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "201281:100:nnn",
      description: "Received from ASN$0",
      type: "large",
    });
    expect(
      annotateLargeCommunity(
        { global_admin: 201281, local_data1: 100, local_data2: 42 },
        defs,
      ),
    ).toBe("Received from ASN42");
  });

  it("captures multiple wildcard groups with $0 and $1", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "1299:1xx0nn",
      description: "Type $0, Region $1",
      type: "standard",
    });
    expect(
      annotateStandardCommunity({ asn: 1299, value: 123045 }, defs),
    ).toBe("Type 23, Region 45");
  });

  it("rejects wrong-length value for mixed wildcard segment", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "100:1xx",
      description: "Test $0",
      type: "standard",
    });
    // "1xx" is a mixed segment (literal '1' + wildcards) so requires equal length.
    // value=1 → segment "1" (1 char) vs pattern "1xx" (3 chars) → no match
    expect(
      annotateStandardCommunity({ asn: 100, value: 1 }, defs),
    ).toBeNull();
  });
});

// --- Description substitution ---

describe("description substitution", () => {
  it("replaces $0 in pattern description", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "15169:1100x",
      description: "Product Differentiator $0",
      type: "standard",
    });
    expect(
      annotateStandardCommunity({ asn: 15169, value: 11005 }, defs),
    ).toBe("Product Differentiator 5");
  });

  it("replaces multiple substitution variables", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "65000:xxnn",
      description: "Group $0, Sub $1",
      type: "standard",
    });
    expect(
      annotateStandardCommunity({ asn: 65000, value: 4217 }, defs),
    ).toBe("Group 42, Sub 17");
  });
});

// --- Range matching ---

describe("range matching", () => {
  it("matches a range in value position", () => {
    const defs = emptyDefs();
    defs.ranges = [
      {
        segments: [
          { type: "exact", value: "15169" },
          { type: "range", start: 13001, end: 13099 },
        ],
        description: "Range description",
        type: "standard",
      },
    ];
    expect(
      annotateStandardCommunity({ asn: 15169, value: 13050 }, defs),
    ).toBe("Range description");
  });

  it("rejects value outside range", () => {
    const defs = emptyDefs();
    defs.ranges = [
      {
        segments: [
          { type: "exact", value: "15169" },
          { type: "range", start: 13001, end: 13099 },
        ],
        description: "Range description",
        type: "standard",
      },
    ];
    expect(
      annotateStandardCommunity({ asn: 15169, value: 14000 }, defs),
    ).toBeNull();
  });

  it("matches a range in ASN position", () => {
    const defs = emptyDefs();
    defs.ranges = [
      {
        segments: [
          { type: "range", start: 65001, end: 65004 },
          { type: "exact", value: "6509" },
        ],
        description: "Range in ASN",
        type: "standard",
      },
    ];
    expect(
      annotateStandardCommunity({ asn: 65002, value: 6509 }, defs),
    ).toBe("Range in ASN");
  });

  it("substitutes $0 from range capture", () => {
    const defs = emptyDefs();
    defs.ranges = [
      {
        segments: [
          { type: "exact", value: "15169" },
          { type: "range", start: 13001, end: 13099 },
        ],
        description: "Region $0",
        type: "standard",
      },
    ];
    expect(
      annotateStandardCommunity({ asn: 15169, value: 13042 }, defs),
    ).toBe("Region 13042");
  });

  it("matches large community range", () => {
    const defs = emptyDefs();
    defs.ranges = [
      {
        segments: [
          { type: "exact", value: "6695" },
          { type: "exact", value: "1914" },
          { type: "range", start: 100, end: 200 },
        ],
        description: "Location $0",
        type: "large",
      },
    ];
    expect(
      annotateLargeCommunity(
        { global_admin: 6695, local_data1: 1914, local_data2: 150 },
        defs,
      ),
    ).toBe("Location 150");
  });

  it("matches range + wildcard segments", () => {
    const defs = emptyDefs();
    defs.ranges = [
      {
        segments: [
          { type: "range", start: 65511, end: 65513 },
          { type: "wildcard", pattern: "nnn" },
        ],
        description: "AS $0 code $1",
        type: "standard",
      },
    ];
    expect(
      annotateStandardCommunity({ asn: 65512, value: 456 }, defs),
    ).toBe("AS 65512 code 456");
  });
});

// --- Priority ---

describe("priority", () => {
  it("exact match takes priority over pattern", () => {
    const defs = emptyDefs();
    defs.standard["1299:20500"] = "Exact Match";
    defs.patterns.push({
      pattern: "1299:20xxx",
      description: "Pattern Match",
      type: "standard",
    });
    expect(
      annotateStandardCommunity({ asn: 1299, value: 20500 }, defs),
    ).toBe("Exact Match");
  });

  it("exact match takes priority over range", () => {
    const defs = emptyDefs();
    defs.standard["15169:13050"] = "Exact Match";
    defs.ranges = [
      {
        segments: [
          { type: "exact", value: "15169" },
          { type: "range", start: 13001, end: 13099 },
        ],
        description: "Range Match",
        type: "standard",
      },
    ];
    expect(
      annotateStandardCommunity({ asn: 15169, value: 13050 }, defs),
    ).toBe("Exact Match");
  });

  it("pattern takes priority over range", () => {
    const defs = emptyDefs();
    defs.patterns.push({
      pattern: "15169:130xx",
      description: "Pattern Match",
      type: "standard",
    });
    defs.ranges = [
      {
        segments: [
          { type: "exact", value: "15169" },
          { type: "range", start: 13001, end: 13099 },
        ],
        description: "Range Match",
        type: "standard",
      },
    ];
    expect(
      annotateStandardCommunity({ asn: 15169, value: 13050 }, defs),
    ).toBe("Pattern Match");
  });
});

// --- Backward compatibility ---

describe("backward compatibility", () => {
  it("works without ranges field", () => {
    const defs: CommunityDefinitions = {
      standard: { "100:1": "Test" },
      large: {},
      patterns: [],
    };
    expect(annotateStandardCommunity({ asn: 100, value: 1 }, defs)).toBe(
      "Test",
    );
    expect(
      annotateStandardCommunity({ asn: 100, value: 2 }, defs),
    ).toBeNull();
  });
});
