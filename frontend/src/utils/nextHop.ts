import type { BgpRoute } from "../api/types";
import { isHiddenAddress } from "./hiddenAddress";

/** Returns true for an IPv6 link-local address (fe80::/10). */
export function isLinkLocal(addr: string): boolean {
  if (!addr.includes(":")) return false;
  const firstGroup = parseInt(addr.split(":")[0] || "0", 16);
  return (firstGroup & 0xffc0) === 0xfe80;
}

/** Label for an IPv4 route with an IPv6 next hop. */
export const EXTENDED_NEXT_HOP_LABEL = "Extended next hop (RFC 8950)";

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
  // The server hides only link-local next hops (hide_peer_addresses).
  else if (isLinkLocal(route.next_hop) || isHiddenAddress(route.next_hop)) type = "IPv6 link-local";
  else if (route.next_hop_link_local) type = "IPv6 global + link-local";
  else type = "IPv6 global";

  return {
    afiSafi: ipv4Route ? "IPv4 unicast (AFI 1, SAFI 1)" : "IPv6 unicast (AFI 2, SAFI 1)",
    type,
    extendedNextHop: ipv4Route && ipv6NextHop,
  };
}
