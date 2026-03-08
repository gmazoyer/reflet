export interface PeerInfo {
  id: string;
  address: string;
  remote_asn: number;
  router_id: string;
  name: string;
  description: string;
  location: string | null;
  state: PeerState;
  uptime: string | null;
  prefixes: PrefixCounts;
}

export type PeerState =
  | "Idle"
  | "Connect"
  | "Active"
  | "OpenSent"
  | "OpenConfirm"
  | "Established";

export interface PrefixCounts {
  ipv4: number;
  ipv6: number;
}

export interface AsPathSegment {
  type: "Sequence" | "Set";
  asns: number[];
}

export interface Community {
  asn: number;
  value: number;
}

export interface ExtCommunity {
  type_high: number;
  type_low: number;
  value: number[];
}

export interface LargeCommunity {
  global_admin: number;
  local_data1: number;
  local_data2: number;
}

export type RpkiStatus = "valid" | "invalid" | "not_found";

export interface BgpRoute {
  prefix: string;
  path_id: number | null;
  origin: "IGP" | "EGP" | "INCOMPLETE";
  as_path: AsPathSegment[];
  next_hop: string;
  med: number | null;
  local_pref: number | null;
  communities: Community[];
  ext_communities: ExtCommunity[];
  large_communities: LargeCommunity[];
  origin_as: number | null;
  received_at: string;
  rpki_status?: RpkiStatus;
}

export interface PaginatedRoutes {
  data: BgpRoute[];
  meta: PaginationMeta;
}

export interface PaginationMeta {
  total: number;
  page: number;
  per_page: number;
}

export interface RpkiSummary {
  vrp_count: number;
}

export interface SummaryResponse {
  title: string;
  local_asn: number;
  router_id: string;
  peer_count: number;
  established_peers: number;
  total_ipv4_prefixes: number;
  total_ipv6_prefixes: number;
  route_refresh_enabled: boolean;
  rpki?: RpkiSummary | null;
}

export interface LookupResult {
  peer_id: string;
  peer_name: string;
  matched_prefix: string | null;
  routes: BgpRoute[];
}

export interface LookupResponse {
  query: string;
  lookup_type: string;
  results: LookupResult[];
}

export interface CommunityPattern {
  pattern: string;
  description: string;
  type: "standard" | "large";
}

export interface WhoamiResponse {
  ip: string;
}

export interface AsnInfo {
  name: string;
  as_domain: string;
}

export type AsnMap = Record<string, AsnInfo>;

export type SegmentMatcher =
  | { type: "exact"; value: string }
  | { type: "range"; start: number; end: number }
  | { type: "wildcard"; pattern: string };

export interface CommunityRange {
  segments: SegmentMatcher[];
  description: string;
  type: "standard" | "large";
}

export interface CommunityDefinitions {
  standard: Record<string, string>;
  large: Record<string, string>;
  patterns: CommunityPattern[];
  ranges?: CommunityRange[];
}
