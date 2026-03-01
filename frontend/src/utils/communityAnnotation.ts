import type {
  Community,
  LargeCommunity,
  CommunityDefinitions,
  CommunityRange,
} from "../api/types";

/**
 * Check whether every character in a string is the same wildcard letter.
 */
function isFullyWildcard(seg: string): boolean {
  return seg.length > 0 && (seg === "x".repeat(seg.length) || seg === "n".repeat(seg.length));
}

/**
 * Match a single segment value against a mixed (literal+wildcard) segment pattern.
 * Returns captured groups on match, null on mismatch.
 * Requires equal length — only used for segments that mix literals and wildcards.
 */
function matchMixedSegment(val: string, pat: string): string[] | null {
  if (val.length !== pat.length) return null;

  const captures: string[] = [];
  let i = 0;

  while (i < pat.length) {
    const p = pat[i];
    if (p === "x" || p === "n") {
      let captured = "";
      while (i < pat.length && pat[i] === p) {
        if (val[i] < "0" || val[i] > "9") return null;
        captured += val[i];
        i++;
      }
      captures.push(captured);
    } else {
      if (val[i] !== p) return null;
      i++;
    }
  }

  return captures;
}

/**
 * Match a community key against a wildcard pattern, returning captured groups.
 *
 * Matching is done **segment-by-segment** (split on `:`).
 * - A fully-wildcard segment (`nnn`, `xxx`) matches any sequence of 1+ digits
 *   regardless of length and produces one capture.
 * - A mixed segment (`20xxx`) does character-by-character matching and requires
 *   equal length; consecutive identical wildcards form one capture.
 * - A literal segment must match exactly.
 */
function matchPatternWithCaptures(
  key: string,
  pattern: string,
): string[] | null {
  const keySegs = key.split(":");
  const patSegs = pattern.split(":");
  if (keySegs.length !== patSegs.length) return null;

  const captures: string[] = [];

  for (let s = 0; s < patSegs.length; s++) {
    const pat = patSegs[s];
    const val = keySegs[s];

    if (isFullyWildcard(pat)) {
      // Fully-wildcard segment: match any digits (variable length)
      if (val.length === 0 || !/^\d+$/.test(val)) return null;
      captures.push(val);
    } else if (pat.includes("x") || pat.includes("n")) {
      // Mixed segment: character-by-character (requires equal length)
      const segCaptures = matchMixedSegment(val, pat);
      if (segCaptures === null) return null;
      captures.push(...segCaptures);
    } else {
      // Literal segment: exact match
      if (val !== pat) return null;
    }
  }

  return captures;
}

/**
 * Match a community key against a CommunityRange definition.
 * Returns captured values (from range and wildcard segments) on match, or null.
 */
function matchRange(key: string, range: CommunityRange): string[] | null {
  const keySegments = key.split(":");
  if (keySegments.length !== range.segments.length) return null;

  const captures: string[] = [];

  for (let i = 0; i < range.segments.length; i++) {
    const seg = range.segments[i];
    const val = keySegments[i];

    if (seg.type === "exact") {
      if (val !== seg.value) return null;
    } else if (seg.type === "range") {
      const num = parseInt(val, 10);
      if (isNaN(num) || num < seg.start || num > seg.end) return null;
      captures.push(val);
    } else {
      // wildcard segment
      if (isFullyWildcard(seg.pattern)) {
        if (val.length === 0 || !/^\d+$/.test(val)) return null;
        captures.push(val);
      } else {
        const segCaptures = matchMixedSegment(val, seg.pattern);
        if (segCaptures === null) return null;
        captures.push(...segCaptures);
      }
    }
  }

  return captures;
}

/**
 * Replace `$0`, `$1`, etc. in a description template with captured values.
 */
function substituteDescription(desc: string, captures: string[]): string {
  let result = desc;
  for (let i = 0; i < captures.length; i++) {
    result = result.split(`$${i}`).join(captures[i]);
  }
  return result;
}

/**
 * Annotate a standard community (asn:value) with a human-readable description.
 * Returns null if no matching definition is found.
 */
export function annotateStandardCommunity(
  c: Community,
  defs: CommunityDefinitions,
): string | null {
  const key = `${c.asn}:${c.value}`;

  // 1. Exact match
  const exact = defs.standard[key];
  if (exact) return exact;

  // 2. Wildcard patterns (with substitution)
  for (const p of defs.patterns) {
    if (p.type === "standard") {
      const captures = matchPatternWithCaptures(key, p.pattern);
      if (captures) return substituteDescription(p.description, captures);
    }
  }

  // 3. Range definitions (with substitution)
  if (defs.ranges) {
    for (const r of defs.ranges) {
      if (r.type === "standard") {
        const captures = matchRange(key, r);
        if (captures) return substituteDescription(r.description, captures);
      }
    }
  }

  return null;
}

/**
 * Annotate a large community (global_admin:local_data1:local_data2) with a description.
 * Returns null if no matching definition is found.
 */
export function annotateLargeCommunity(
  c: LargeCommunity,
  defs: CommunityDefinitions,
): string | null {
  const key = `${c.global_admin}:${c.local_data1}:${c.local_data2}`;

  // 1. Exact match
  const exact = defs.large[key];
  if (exact) return exact;

  // 2. Wildcard patterns (with substitution)
  for (const p of defs.patterns) {
    if (p.type === "large") {
      const captures = matchPatternWithCaptures(key, p.pattern);
      if (captures) return substituteDescription(p.description, captures);
    }
  }

  // 3. Range definitions (with substitution)
  if (defs.ranges) {
    for (const r of defs.ranges) {
      if (r.type === "large") {
        const captures = matchRange(key, r);
        if (captures) return substituteDescription(r.description, captures);
      }
    }
  }

  return null;
}
