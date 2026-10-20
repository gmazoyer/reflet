import { describe, it, expect } from "vitest";
import { describeNextHop, isLinkLocal } from "./nextHop";

describe("isLinkLocal", () => {
  it("matches fe80::/10", () => {
    expect(isLinkLocal("fe80::1")).toBe(true);
    expect(isLinkLocal("FE80::1")).toBe(true);
    expect(isLinkLocal("febf::1")).toBe(true);
  });

  it("rejects other addresses", () => {
    expect(isLinkLocal("fec0::1")).toBe(false);
    expect(isLinkLocal("2001:db8::1")).toBe(false);
    expect(isLinkLocal("::")).toBe(false);
    expect(isLinkLocal("10.0.0.1")).toBe(false);
  });
});

describe("describeNextHop", () => {
  it("describes an IPv4 route with an IPv4 next hop", () => {
    const info = describeNextHop({ prefix: "10.0.0.0/24", next_hop: "10.0.0.1" });
    expect(info).toEqual({
      afiSafi: "IPv4 unicast (AFI 1, SAFI 1)",
      type: "IPv4",
      extendedNextHop: false,
    });
  });

  it("describes an IPv4 route with an IPv6 next hop (RFC 8950)", () => {
    const info = describeNextHop({
      prefix: "10.0.0.0/24",
      next_hop: "2001:db8::1",
      next_hop_link_local: "fe80::1",
    });
    expect(info).toEqual({
      afiSafi: "IPv4 unicast (AFI 1, SAFI 1)",
      type: "IPv6 global + link-local (RFC 8950)",
      extendedNextHop: true,
    });
  });

  it("describes an IPv4 route with a link-local next hop", () => {
    const info = describeNextHop({ prefix: "10.0.0.0/24", next_hop: "fe80::1" });
    expect(info.type).toBe("IPv6 link-local (RFC 8950)");
  });

  it("describes IPv6 routes", () => {
    expect(describeNextHop({ prefix: "2001:db8::/32", next_hop: "2001:db8::1" })).toEqual({
      afiSafi: "IPv6 unicast (AFI 2, SAFI 1)",
      type: "IPv6 global",
      extendedNextHop: false,
    });
    expect(describeNextHop({ prefix: "2001:db8::/32", next_hop: "fe80::1" }).type).toBe(
      "IPv6 link-local",
    );
  });
});
