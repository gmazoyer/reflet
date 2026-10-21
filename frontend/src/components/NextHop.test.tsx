import { describe, it, expect } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { BgpRoute } from "../api/types";
import { EXTENDED_NEXT_HOP_LABEL } from "../utils/nextHop";
import NextHop from "./NextHop";

function route(prefix: string, next_hop: string, next_hop_link_local?: string): BgpRoute {
  return { prefix, next_hop, next_hop_link_local } as BgpRoute;
}

describe("NextHop", () => {
  it("labels an IPv4 route with an IPv6 next hop", () => {
    for (const showDetails of [true, false]) {
      const html = renderToStaticMarkup(
        <NextHop route={route("10.0.0.0/24", "2001:db8::1", "fe80::1")} showDetails={showDetails} />,
      );
      expect(html).toContain(EXTENDED_NEXT_HOP_LABEL);
      expect(html).toContain("fe80::1");
    }
  });

  it("does not label other routes", () => {
    for (const r of [route("10.0.0.0/24", "10.0.0.1"), route("2001:db8::/32", "2001:db8::1")]) {
      expect(renderToStaticMarkup(<NextHop route={r} showDetails />)).not.toContain(
        EXTENDED_NEXT_HOP_LABEL,
      );
    }
  });

  it("shows a hidden next hop as Hidden", () => {
    const html = renderToStaticMarkup(<NextHop route={route("2001:db8::/32", "::")} showDetails />);
    expect(html).toContain("Hidden");
  });
});
