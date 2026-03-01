import axios from "axios";
import type {
  PeerInfo,
  SummaryResponse,
  PaginatedRoutes,
  LookupResponse,
  CommunityDefinitions,
  AsnMap,
  WhoamiResponse,
} from "./types";

const api = axios.create({
  baseURL: "/",
  headers: { "Content-Type": "application/json" },
});

export async function getSummary(): Promise<SummaryResponse> {
  const { data } = await api.get<SummaryResponse>("/api/v1/summary");
  return data;
}

export async function getPeers(): Promise<PeerInfo[]> {
  const { data } = await api.get<PeerInfo[]>("/api/v1/peers");
  return data;
}

export async function getPeer(id: string): Promise<PeerInfo> {
  const { data } = await api.get<PeerInfo>(`/api/v1/peers/${encodeURIComponent(id)}`);
  return data;
}

export async function getPeerRoutes(
  peerId: string,
  af: "ipv4" | "ipv6",
  page = 1,
  perPage = 100,
  search?: string,
): Promise<PaginatedRoutes> {
  const params: Record<string, string | number> = { page, per_page: perPage };
  if (search) params.search = search;
  const { data } = await api.get<PaginatedRoutes>(
    `/api/v1/peers/${encodeURIComponent(peerId)}/routes/${af}`,
    { params },
  );
  return data;
}

export async function lookup(
  prefix: string,
  type: "exact" | "longest-match" | "subnets" = "longest-match",
): Promise<LookupResponse> {
  const { data } = await api.get<LookupResponse>("/api/v1/lookup", {
    params: { prefix, type },
  });
  return data;
}

export async function getWhoami(): Promise<WhoamiResponse> {
  const { data } = await api.get<WhoamiResponse>("/api/v1/whoami");
  return data;
}

export async function getAsnInfo(): Promise<AsnMap> {
  const { data } = await api.get<AsnMap>("/api/v1/asns");
  return data;
}

export async function getCommunityDefinitions(): Promise<CommunityDefinitions> {
  const { data } = await api.get<CommunityDefinitions>(
    "/api/v1/communities/definitions",
  );
  return data;
}

export async function refreshPeer(
  peerId: string,
): Promise<{ message: string }> {
  const { data } = await api.post<{ message: string }>(
    `/api/v1/peers/${encodeURIComponent(peerId)}/refresh`,
  );
  return data;
}
