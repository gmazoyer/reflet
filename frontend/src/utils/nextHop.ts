import type { BgpRoute } from "../api/types";

/** Returns true for an IPv6 link-local address (fe80::/10). */
export function isLinkLocal(addr: string): boolean {
  if (!addr.includes(":")) return false;
  const firstGroup = parseInt(addr.split(":")[0] || "0", 16);
  return (firstGroup & 0xffc0) === 0xfe80;
}

export interface NextHopInfo {
  /** Address family of the route, e.g. "IPv4 unicast (AFI 1, SAFI 1)". */
  afiSafi: string;
  /** Kind of next hop, e.g. "IPv6 global + link-local". */
  type: string;
  /** True when an IPv4 route has an IPv6 next hop (RFC 8950). */
  extendedNextHop: boolean;
}

/** Describe the address family of a route and the kind of its next hop. */
export function describeNextHop(
  route: Pick<BgpRoute, "prefix" | "next_hop" | "next_hop_link_local">,
): NextHopInfo {
  const ipv4Route = !route.prefix.includes(":");
  const ipv6NextHop = route.next_hop.includes(":");

  let type: string;
  if (!ipv6NextHop) type = "IPv4";
  else if (isLinkLocal(route.next_hop)) type = "IPv6 link-local";
  else if (route.next_hop_link_local) type = "IPv6 global + link-local";
  else type = "IPv6 global";

  const extendedNextHop = ipv4Route && ipv6NextHop;
  return {
    afiSafi: ipv4Route ? "IPv4 unicast (AFI 1, SAFI 1)" : "IPv6 unicast (AFI 2, SAFI 1)",
    type: extendedNextHop ? `${type} (RFC 8950)` : type,
    extendedNextHop,
  };
}
