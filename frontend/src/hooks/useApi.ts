import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  getSummary,
  getPeers,
  getPeer,
  getPeerRoutes,
  lookup,
  getWhoami,
  getAsnInfo,
  getCommunityDefinitions,
  refreshPeer,
  getSnapshots,
  getSnapshotRoutes,
} from "../api/client";

export function useSummary() {
  return useQuery({
    queryKey: ["summary"],
    queryFn: getSummary,
  });
}

export function usePeers() {
  return useQuery({
    queryKey: ["peers"],
    queryFn: getPeers,
  });
}

export function usePeer(id: string) {
  return useQuery({
    queryKey: ["peer", id],
    queryFn: () => getPeer(id),
    enabled: !!id,
  });
}

export function usePeerRoutes(
  peerId: string,
  af: "ipv4" | "ipv6",
  page: number,
  perPage: number,
  search?: string,
) {
  return useQuery({
    queryKey: ["peerRoutes", peerId, af, page, perPage, search],
    queryFn: () => getPeerRoutes(peerId, af, page, perPage, search),
    enabled: !!peerId,
  });
}

export function useLookup(prefix: string, type: "exact" | "longest-match" | "subnets") {
  return useQuery({
    queryKey: ["lookup", prefix, type],
    queryFn: () => lookup(prefix, type),
    enabled: !!prefix,
  });
}

export function useWhoami() {
  return useQuery({
    queryKey: ["whoami"],
    queryFn: getWhoami,
    staleTime: Infinity,
  });
}

export function useAsnInfo() {
  return useQuery({
    queryKey: ["asnInfo"],
    queryFn: getAsnInfo,
    staleTime: Infinity,
  });
}

export function useCommunityDefinitions() {
  return useQuery({
    queryKey: ["communityDefinitions"],
    queryFn: getCommunityDefinitions,
    staleTime: Infinity,
  });
}

export function useSnapshots(peerId: string, enabled = true) {
  return useQuery({
    queryKey: ["snapshots", peerId],
    queryFn: () => getSnapshots(peerId),
    enabled: !!peerId && enabled,
  });
}

export function useSnapshotRoutes(
  peerId: string,
  timestamp: string,
  af: "ipv4" | "ipv6",
  page: number,
  perPage: number,
  search?: string,
) {
  return useQuery({
    queryKey: ["snapshotRoutes", peerId, timestamp, af, page, perPage, search],
    queryFn: () => getSnapshotRoutes(peerId, timestamp, af, page, perPage, search),
    enabled: !!peerId && !!timestamp,
  });
}

export function useRefreshPeer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (peerId: string) => refreshPeer(peerId),
    onSuccess: (_data, peerId) => {
      queryClient.invalidateQueries({ queryKey: ["peerRoutes", peerId] });
      queryClient.invalidateQueries({ queryKey: ["peer", peerId] });
    },
  });
}
