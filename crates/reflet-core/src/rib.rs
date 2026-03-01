use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use ipnet::{Ipv4Net, Ipv6Net};
use prefix_trie::PrefixMap;
use rayon::prelude::*;

use crate::peer::PeerId;
use crate::prefix::Prefix;
use crate::route::{BgpRoute, Origin};

/// Numeric comparison for MED and local-pref filters.
#[derive(Debug, PartialEq)]
enum NumericComparison {
    Eq(u32),
    Gt(u32),
    Lt(u32),
    Gte(u32),
    Lte(u32),
}

impl NumericComparison {
    fn matches(&self, value: u32) -> bool {
        match self {
            NumericComparison::Eq(v) => value == *v,
            NumericComparison::Gt(v) => value > *v,
            NumericComparison::Lt(v) => value < *v,
            NumericComparison::Gte(v) => value >= *v,
            NumericComparison::Lte(v) => value <= *v,
        }
    }
}

fn parse_numeric_comparison(s: &str) -> Option<NumericComparison> {
    if let Some(rest) = s.strip_prefix(">=") {
        rest.parse().ok().map(NumericComparison::Gte)
    } else if let Some(rest) = s.strip_prefix("<=") {
        rest.parse().ok().map(NumericComparison::Lte)
    } else if let Some(rest) = s.strip_prefix('>') {
        rest.parse().ok().map(NumericComparison::Gt)
    } else if let Some(rest) = s.strip_prefix('<') {
        rest.parse().ok().map(NumericComparison::Lt)
    } else {
        s.parse().ok().map(NumericComparison::Eq)
    }
}

/// Parsed search filter for route pagination.
#[derive(Debug)]
enum SearchFilter<'a> {
    /// Substring match on the prefix string (e.g. "10.0.0").
    Prefix(&'a str),
    /// Match routes where any ASN in the AS path equals this value (e.g. "AS65001").
    Asn(u32),
    /// Match routes whose flattened AS path contains this contiguous subsequence (e.g. "65000 65001").
    AsPath(Vec<u32>),
    /// Match routes with a specific standard community (e.g. "community:65000:100", "community:65000:*").
    Community {
        asn: Option<u16>,
        value: Option<u16>,
    },
    /// Match routes with a specific large community (e.g. "lc:65000:1:2").
    LargeCommunity {
        global_admin: Option<u32>,
        local_data1: Option<u32>,
        local_data2: Option<u32>,
    },
    /// Match routes by origin attribute (e.g. "origin:igp").
    Origin(Origin),
    /// Match routes by MED with comparison (e.g. "med:>100").
    Med(NumericComparison),
    /// Match routes by local-pref with comparison (e.g. "localpref:>=200").
    LocalPref(NumericComparison),
}

/// Known structured filter keywords (lowercase).
const STRUCTURED_KEYWORDS: &[&str] = &["community:", "lc:", "origin:", "med:", "localpref:"];

/// Try to parse a single token as a structured `key:value` filter.
/// Returns `None` if the token doesn't start with a known keyword or parsing fails.
fn parse_structured_token(token: &str) -> Option<SearchFilter<'_>> {
    let lower = token.to_ascii_lowercase();

    if let Some(val) = lower.strip_prefix("community:") {
        let parts: Vec<&str> = val.split(':').collect();
        if parts.len() == 2 {
            let asn = if parts[0] == "*" {
                None
            } else {
                Some(parts[0].parse::<u16>().ok()?)
            };
            let value = if parts[1] == "*" {
                None
            } else {
                Some(parts[1].parse::<u16>().ok()?)
            };
            return Some(SearchFilter::Community { asn, value });
        }
        return None;
    }

    if let Some(val) = lower.strip_prefix("lc:") {
        let parts: Vec<&str> = val.split(':').collect();
        if parts.len() == 3 {
            let global_admin = if parts[0] == "*" {
                None
            } else {
                Some(parts[0].parse::<u32>().ok()?)
            };
            let local_data1 = if parts[1] == "*" {
                None
            } else {
                Some(parts[1].parse::<u32>().ok()?)
            };
            let local_data2 = if parts[2] == "*" {
                None
            } else {
                Some(parts[2].parse::<u32>().ok()?)
            };
            return Some(SearchFilter::LargeCommunity {
                global_admin,
                local_data1,
                local_data2,
            });
        }
        return None;
    }

    if let Some(val) = lower.strip_prefix("origin:") {
        let origin = match val {
            "igp" => Origin::Igp,
            "egp" => Origin::Egp,
            "incomplete" => Origin::Incomplete,
            _ => return None,
        };
        return Some(SearchFilter::Origin(origin));
    }

    if let Some(val) = lower.strip_prefix("med:") {
        return parse_numeric_comparison(val).map(SearchFilter::Med);
    }

    if let Some(val) = lower.strip_prefix("localpref:") {
        return parse_numeric_comparison(val).map(SearchFilter::LocalPref);
    }

    None
}

/// Parse a search string into a list of filters (AND semantics).
///
/// Tokens starting with a known keyword (`community:`, `lc:`, `origin:`, `med:`,
/// `localpref:`) are parsed as structured filters. Remaining tokens are rejoined
/// and parsed with the legacy heuristic (ASN → AS path → prefix fallback).
/// If a structured token fails to parse, it falls back to the remainder.
fn parse_search(input: &str) -> Vec<SearchFilter<'_>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return vec![];
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let mut filters: Vec<SearchFilter<'_>> = Vec::new();
    let mut remainder_tokens: Vec<&str> = Vec::new();

    for token in &tokens {
        let lower = token.to_ascii_lowercase();
        let is_structured = STRUCTURED_KEYWORDS.iter().any(|kw| lower.starts_with(kw));

        if is_structured {
            if let Some(f) = parse_structured_token(token) {
                filters.push(f);
            } else {
                // Parse failure → treat as remainder
                remainder_tokens.push(token);
            }
        } else {
            remainder_tokens.push(token);
        }
    }

    if !remainder_tokens.is_empty() {
        // Apply the legacy heuristic on the remainder

        // Check for ASN pattern: AS<digits> or as<digits>
        if remainder_tokens.len() == 1 {
            let t = remainder_tokens[0];
            if (t.starts_with("AS") || t.starts_with("as"))
                && t.len() > 2
                && t[2..].chars().all(|c| c.is_ascii_digit())
                && let Ok(asn) = t[2..].parse::<u32>()
            {
                filters.push(SearchFilter::Asn(asn));
                return filters;
            }
        }

        // Check for AS path pattern: space-separated numbers (at least 2)
        if remainder_tokens.len() >= 2 {
            let parsed: Vec<u32> = remainder_tokens
                .iter()
                .filter_map(|p| p.parse::<u32>().ok())
                .collect();
            if parsed.len() == remainder_tokens.len() {
                filters.push(SearchFilter::AsPath(parsed));
                return filters;
            }
        }

        // Fallback: prefix substring on the full remainder
        // We need a &str that lives as long as the input. Find the substring in input.
        let start = input.find(remainder_tokens[0]).unwrap_or(0);
        let end = {
            let last = remainder_tokens[remainder_tokens.len() - 1];
            let last_start = input[start..].find(last).unwrap_or(0) + start;
            last_start + last.len()
        };
        filters.push(SearchFilter::Prefix(input[start..end].trim()));
    }

    filters
}

/// Check if `path` contains `pattern` as a contiguous subsequence (sliding window).
fn as_path_contains_subsequence(path: &[u32], pattern: &[u32]) -> bool {
    if pattern.is_empty() {
        return true;
    }
    path.windows(pattern.len()).any(|w| w == pattern)
}

/// Per-peer RIB containing both IPv4 and IPv6 routes.
///
/// Each prefix maps to a `Vec<BgpRoute>` to support Add-Path (RFC 7911).
/// Non-Add-Path peers (where `path_id` is `None`) always overwrite the
/// entire Vec for backward compatibility.
#[derive(Debug, Default)]
pub struct PeerRib {
    pub ipv4: PrefixMap<Ipv4Net, Vec<BgpRoute>>,
    pub ipv6: PrefixMap<Ipv6Net, Vec<BgpRoute>>,
}

impl PeerRib {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a route into the appropriate address family table.
    ///
    /// When `path_id` is `Some`, finds and replaces an existing path with the
    /// same ID, or appends. When `path_id` is `None`, overwrites the entire
    /// Vec (backward compat for non-Add-Path peers).
    pub fn insert(&mut self, route: BgpRoute) {
        match &route.prefix {
            Prefix::V4(net) => Self::insert_into_trie(&mut self.ipv4, *net, route),
            Prefix::V6(net) => Self::insert_into_trie(&mut self.ipv6, *net, route),
        }
    }

    fn insert_into_trie<P: prefix_trie::Prefix>(
        trie: &mut PrefixMap<P, Vec<BgpRoute>>,
        net: P,
        route: BgpRoute,
    ) {
        if let Some(path_id) = route.path_id {
            // Add-Path: replace existing path with same ID, or append
            if let Some(routes) = trie.get_mut(&net) {
                if let Some(existing) = routes.iter_mut().find(|r| r.path_id == Some(path_id)) {
                    *existing = route;
                } else {
                    routes.push(route);
                }
            } else {
                trie.insert(net, vec![route]);
            }
        } else {
            // Non-Add-Path: overwrite the entire Vec
            trie.insert(net, vec![route]);
        }
    }

    /// Remove a route by prefix and optional path ID.
    ///
    /// When `path_id` is `Some`, removes only that path (and the prefix entry
    /// if the Vec becomes empty). When `None`, removes the whole prefix.
    pub fn remove(&mut self, prefix: &Prefix, path_id: Option<u32>) {
        match prefix {
            Prefix::V4(net) => Self::remove_from_trie(&mut self.ipv4, net, path_id),
            Prefix::V6(net) => Self::remove_from_trie(&mut self.ipv6, net, path_id),
        }
    }

    fn remove_from_trie<P: prefix_trie::Prefix>(
        trie: &mut PrefixMap<P, Vec<BgpRoute>>,
        net: &P,
        path_id: Option<u32>,
    ) {
        if let Some(pid) = path_id {
            // Add-Path: remove just the matching path
            if let Some(routes) = trie.get_mut(net) {
                routes.retain(|r| r.path_id != Some(pid));
                if routes.is_empty() {
                    trie.remove(net);
                }
            }
        } else {
            trie.remove(net);
        }
    }

    /// Exact lookup by prefix. Returns all paths for that prefix.
    pub fn get(&self, prefix: &Prefix) -> Option<&Vec<BgpRoute>> {
        match prefix {
            Prefix::V4(net) => self.ipv4.get(net),
            Prefix::V6(net) => self.ipv6.get(net),
        }
    }

    /// Longest-prefix-match lookup for an IPv4 prefix.
    pub fn lpm_v4(&self, prefix: &Ipv4Net) -> Option<(&Ipv4Net, &Vec<BgpRoute>)> {
        self.ipv4.get_lpm(prefix)
    }

    /// Longest-prefix-match lookup for an IPv6 prefix.
    pub fn lpm_v6(&self, prefix: &Ipv6Net) -> Option<(&Ipv6Net, &Vec<BgpRoute>)> {
        self.ipv6.get_lpm(prefix)
    }

    /// Number of IPv4 paths (total across all prefixes).
    pub fn ipv4_count(&self) -> usize {
        self.ipv4.iter().map(|(_, routes)| routes.len()).sum()
    }

    /// Number of IPv6 paths (total across all prefixes).
    pub fn ipv6_count(&self) -> usize {
        self.ipv6.iter().map(|(_, routes)| routes.len()).sum()
    }

    /// Total number of paths.
    pub fn total_count(&self) -> usize {
        self.ipv4_count() + self.ipv6_count()
    }

    /// Paginate IPv4 routes, optionally filtering by prefix search string.
    /// Returns (page_data, total_count) — only clones the requested page.
    pub fn paginate_ipv4(
        &self,
        page: usize,
        per_page: usize,
        search: Option<&str>,
    ) -> (Vec<BgpRoute>, usize) {
        Self::paginate_trie(&self.ipv4, page, per_page, search)
    }

    /// Paginate IPv6 routes, optionally filtering by prefix search string.
    /// Returns (page_data, total_count) — only clones the requested page.
    pub fn paginate_ipv6(
        &self,
        page: usize,
        per_page: usize,
        search: Option<&str>,
    ) -> (Vec<BgpRoute>, usize) {
        Self::paginate_trie(&self.ipv6, page, per_page, search)
    }

    fn paginate_trie<P>(
        trie: &PrefixMap<P, Vec<BgpRoute>>,
        page: usize,
        per_page: usize,
        search: Option<&str>,
    ) -> (Vec<BgpRoute>, usize)
    where
        P: prefix_trie::Prefix + std::fmt::Display,
    {
        let offset = (page - 1) * per_page;

        if let Some(search) = search {
            let filters = parse_search(search);
            if filters.is_empty() {
                // Empty search string → no filter
                let total: usize = trie.iter().map(|(_, routes)| routes.len()).sum();
                let data = trie
                    .iter()
                    .flat_map(|(_, routes)| routes.iter())
                    .skip(offset)
                    .take(per_page)
                    .cloned()
                    .collect();
                return (data, total);
            }
            // With search: flat_map over routes, filter per-path (AND all filters)
            let filtered: Vec<&BgpRoute> = trie
                .iter()
                .flat_map(|(net, routes)| {
                    let filters = &filters;
                    routes.iter().filter(move |route| {
                        let net_str = net.to_string();
                        filters.iter().all(|filter| match filter {
                            SearchFilter::Prefix(s) => net_str.contains(s),
                            SearchFilter::Asn(asn) => route.as_path_flat().contains(asn),
                            SearchFilter::AsPath(pattern) => {
                                as_path_contains_subsequence(&route.as_path_flat(), pattern)
                            }
                            SearchFilter::Community { asn, value } => {
                                route.communities.iter().any(|c| {
                                    asn.is_none_or(|a| c.asn == a)
                                        && value.is_none_or(|v| c.value == v)
                                })
                            }
                            SearchFilter::LargeCommunity {
                                global_admin,
                                local_data1,
                                local_data2,
                            } => route.large_communities.iter().any(|lc| {
                                global_admin.is_none_or(|g| lc.global_admin == g)
                                    && local_data1.is_none_or(|d1| lc.local_data1 == d1)
                                    && local_data2.is_none_or(|d2| lc.local_data2 == d2)
                            }),
                            SearchFilter::Origin(o) => route.origin == *o,
                            SearchFilter::Med(cmp) => route.med.is_some_and(|m| cmp.matches(m)),
                            SearchFilter::LocalPref(cmp) => {
                                route.local_pref.is_some_and(|lp| cmp.matches(lp))
                            }
                        })
                    })
                })
                .collect();
            let total = filtered.len();
            let data = filtered
                .into_iter()
                .skip(offset)
                .take(per_page)
                .cloned()
                .collect();
            (data, total)
        } else {
            // Without search: flat_map to count total paths, then paginate
            let total: usize = trie.iter().map(|(_, routes)| routes.len()).sum();
            let data = trie
                .iter()
                .flat_map(|(_, routes)| routes.iter())
                .skip(offset)
                .take(per_page)
                .cloned()
                .collect();
            (data, total)
        }
    }

    /// Mark all routes as stale (for Enhanced Route Refresh, RFC 7313).
    pub fn mark_stale(&mut self) {
        for (_prefix, routes) in self.ipv4.iter_mut() {
            for route in routes.iter_mut() {
                route.stale = true;
            }
        }
        for (_prefix, routes) in self.ipv6.iter_mut() {
            for route in routes.iter_mut() {
                route.stale = true;
            }
        }
    }

    /// Remove routes still marked as stale (after refresh completes).
    /// Returns the number of routes removed.
    pub fn sweep_stale(&mut self) -> usize {
        let mut removed = 0;
        removed += Self::sweep_stale_trie(&mut self.ipv4);
        removed += Self::sweep_stale_trie(&mut self.ipv6);
        removed
    }

    fn sweep_stale_trie<P: prefix_trie::Prefix + Copy>(
        trie: &mut PrefixMap<P, Vec<BgpRoute>>,
    ) -> usize {
        let mut removed = 0;
        let mut empty_prefixes: Vec<P> = Vec::new();

        for (prefix, routes) in trie.iter_mut() {
            let before = routes.len();
            routes.retain(|r| !r.stale);
            removed += before - routes.len();
            if routes.is_empty() {
                empty_prefixes.push(*prefix);
            }
        }

        for prefix in empty_prefixes {
            trie.remove(&prefix);
        }

        removed
    }

    /// Clear all routes.
    pub fn clear(&mut self) {
        self.ipv4.clear();
        self.ipv6.clear();
    }
}

/// Thread-safe store of all peer RIBs.
#[derive(Debug, Clone, Default)]
pub struct RibStore {
    ribs: Arc<DashMap<PeerId, Arc<RwLock<PeerRib>>>>,
}

impl RibStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create the RIB for a peer.
    /// Returns a cloned Arc — the DashMap shard lock is released immediately.
    pub fn get_or_create(&self, peer_id: &str) -> Arc<RwLock<PeerRib>> {
        self.ribs
            .entry(peer_id.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(PeerRib::new())))
            .value()
            .clone()
    }

    /// Get a reference to a peer's RIB.
    /// Returns a cloned Arc — the DashMap shard lock is released immediately.
    pub fn get(&self, peer_id: &str) -> Option<Arc<RwLock<PeerRib>>> {
        self.ribs.get(peer_id).map(|entry| entry.value().clone())
    }

    /// Remove a peer's RIB entirely.
    pub fn remove(&self, peer_id: &str) -> Option<(PeerId, Arc<RwLock<PeerRib>>)> {
        self.ribs.remove(peer_id)
    }

    /// List all peer IDs.
    pub fn peer_ids(&self) -> Vec<PeerId> {
        self.ribs.iter().map(|entry| entry.key().clone()).collect()
    }

    /// Perform a lookup across all peers' RIBs. Returns (peer_id, routes) pairs.
    pub fn lookup_exact(&self, prefix: &Prefix) -> Vec<(PeerId, Vec<BgpRoute>)> {
        let peer_ids: Vec<PeerId> = self.peer_ids();
        peer_ids
            .par_iter()
            .filter_map(|peer_id| {
                let rib_arc = self.get(peer_id)?;
                let rib = rib_arc.read().ok()?;
                let routes = rib.get(prefix)?;
                Some((peer_id.clone(), routes.clone()))
            })
            .collect()
    }

    /// Perform a longest-prefix-match lookup across all peers' RIBs.
    pub fn lookup_lpm_v4(&self, prefix: &Ipv4Net) -> Vec<(PeerId, Ipv4Net, Vec<BgpRoute>)> {
        let peer_ids: Vec<PeerId> = self.peer_ids();
        peer_ids
            .par_iter()
            .filter_map(|peer_id| {
                let rib_arc = self.get(peer_id)?;
                let rib = rib_arc.read().ok()?;
                let (matched_prefix, routes) = rib.lpm_v4(prefix)?;
                Some((peer_id.clone(), *matched_prefix, routes.clone()))
            })
            .collect()
    }

    /// Perform a longest-prefix-match lookup across all peers' RIBs (IPv6).
    pub fn lookup_lpm_v6(&self, prefix: &Ipv6Net) -> Vec<(PeerId, Ipv6Net, Vec<BgpRoute>)> {
        let peer_ids: Vec<PeerId> = self.peer_ids();
        peer_ids
            .par_iter()
            .filter_map(|peer_id| {
                let rib_arc = self.get(peer_id)?;
                let rib = rib_arc.read().ok()?;
                let (matched_prefix, routes) = rib.lpm_v6(prefix)?;
                Some((peer_id.clone(), *matched_prefix, routes.clone()))
            })
            .collect()
    }

    /// Find all more-specific routes (subnets) of a given prefix across all peers.
    pub fn lookup_subnets_v4(&self, prefix: &Ipv4Net) -> Vec<(PeerId, Vec<BgpRoute>)> {
        let peer_ids: Vec<PeerId> = self.peer_ids();
        peer_ids
            .par_iter()
            .filter_map(|peer_id| {
                let rib_arc = self.get(peer_id)?;
                let rib = rib_arc.read().ok()?;
                let routes: Vec<BgpRoute> = rib
                    .ipv4
                    .children(prefix)
                    .filter(|(net, _)| *net != prefix)
                    .flat_map(|(_, routes)| routes.iter().cloned())
                    .collect();
                if routes.is_empty() {
                    None
                } else {
                    Some((peer_id.clone(), routes))
                }
            })
            .collect()
    }

    /// Find all more-specific routes (subnets) of a given prefix across all peers (IPv6).
    pub fn lookup_subnets_v6(&self, prefix: &Ipv6Net) -> Vec<(PeerId, Vec<BgpRoute>)> {
        let peer_ids: Vec<PeerId> = self.peer_ids();
        peer_ids
            .par_iter()
            .filter_map(|peer_id| {
                let rib_arc = self.get(peer_id)?;
                let rib = rib_arc.read().ok()?;
                let routes: Vec<BgpRoute> = rib
                    .ipv6
                    .children(prefix)
                    .filter(|(net, _)| *net != prefix)
                    .flat_map(|(_, routes)| routes.iter().cloned())
                    .collect();
                if routes.is_empty() {
                    None
                } else {
                    Some((peer_id.clone(), routes))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use chrono::Utc;

    use super::*;
    use crate::route::{AsPathSegment, Origin};

    fn make_route(prefix_str: &str, next_hop: &str, as_path: Vec<u32>) -> BgpRoute {
        make_route_with_path_id(prefix_str, next_hop, as_path, None)
    }

    fn make_route_with_path_id(
        prefix_str: &str,
        next_hop: &str,
        as_path: Vec<u32>,
        path_id: Option<u32>,
    ) -> BgpRoute {
        let prefix: Prefix = prefix_str.parse().unwrap();
        let origin_as = as_path.last().copied();
        BgpRoute {
            prefix,
            path_id,
            origin: Origin::Igp,
            as_path: vec![AsPathSegment::Sequence(as_path)],
            next_hop: next_hop.parse().unwrap(),
            med: None,
            local_pref: Some(100),
            communities: vec![],
            ext_communities: vec![],
            large_communities: vec![],
            origin_as,
            received_at: Utc::now(),
            stale: false,
            rpki_status: None,
        }
    }

    #[test]
    fn peer_rib_insert_and_get() {
        let mut rib = PeerRib::new();
        let route = make_route("10.0.0.0/24", "10.0.0.1", vec![65000, 65001]);
        rib.insert(route.clone());

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        let routes = rib.get(&prefix);
        assert!(routes.is_some());
        assert_eq!(routes.unwrap().len(), 1);
        assert_eq!(rib.ipv4_count(), 1);
        assert_eq!(rib.ipv6_count(), 0);
    }

    #[test]
    fn peer_rib_remove() {
        let mut rib = PeerRib::new();
        let route = make_route("10.0.0.0/24", "10.0.0.1", vec![65000]);
        rib.insert(route);

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        rib.remove(&prefix, None);
        assert_eq!(rib.ipv4_count(), 0);
    }

    #[test]
    fn peer_rib_lpm() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/8", "10.0.0.1", vec![65000]));
        rib.insert(make_route("10.0.0.0/24", "10.0.0.2", vec![65001]));

        let lookup: Ipv4Net = "10.0.0.128/32".parse().unwrap();
        let result = rib.lpm_v4(&lookup);
        assert!(result.is_some());
        let (matched, routes) = result.unwrap();
        assert_eq!(*matched, "10.0.0.0/24".parse::<Ipv4Net>().unwrap());
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].next_hop, "10.0.0.2".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn peer_rib_ipv6() {
        let mut rib = PeerRib::new();
        let route = make_route("2001:db8::/32", "::1", vec![65000]);
        rib.insert(route);

        assert_eq!(rib.ipv6_count(), 1);
        assert_eq!(rib.ipv4_count(), 0);

        let prefix: Prefix = "2001:db8::/32".parse().unwrap();
        assert!(rib.get(&prefix).is_some());
    }

    #[test]
    fn rib_store_multi_peer() {
        let store = RibStore::new();

        // Peer A
        {
            let rib_ref = store.get_or_create("peer-a");
            let mut rib = rib_ref.write().unwrap();
            rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));
            rib.insert(make_route("10.0.1.0/24", "10.0.0.1", vec![65000, 65001]));
        }

        // Peer B
        {
            let rib_ref = store.get_or_create("peer-b");
            let mut rib = rib_ref.write().unwrap();
            rib.insert(make_route("10.0.0.0/24", "10.0.0.2", vec![65002]));
        }

        // Lookup should find the route from both peers
        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        let results = store.lookup_exact(&prefix);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn rib_store_lpm_across_peers() {
        let store = RibStore::new();

        {
            let rib_ref = store.get_or_create("peer-a");
            let mut rib = rib_ref.write().unwrap();
            rib.insert(make_route("10.0.0.0/8", "10.0.0.1", vec![65000]));
        }
        {
            let rib_ref = store.get_or_create("peer-b");
            let mut rib = rib_ref.write().unwrap();
            rib.insert(make_route("10.0.0.0/24", "10.0.0.2", vec![65001]));
        }

        let lookup: Ipv4Net = "10.0.0.1/32".parse().unwrap();
        let results = store.lookup_lpm_v4(&lookup);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn rib_store_peer_ids() {
        let store = RibStore::new();
        store.get_or_create("peer-a");
        store.get_or_create("peer-b");

        let mut ids = store.peer_ids();
        ids.sort();
        assert_eq!(ids, vec!["peer-a", "peer-b"]);
    }

    #[test]
    fn peer_rib_clear() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));
        rib.insert(make_route("2001:db8::/32", "::1", vec![65000]));
        assert_eq!(rib.total_count(), 2);

        rib.clear();
        assert_eq!(rib.total_count(), 0);
    }

    // --- Add-Path tests ---

    #[test]
    fn peer_rib_add_path_insert_multiple() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Some(1),
        ));
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.2",
            vec![65001],
            Some(2),
        ));

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        let routes = rib.get(&prefix).unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(rib.ipv4_count(), 2);
    }

    #[test]
    fn peer_rib_add_path_remove_by_path_id() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Some(1),
        ));
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.2",
            vec![65001],
            Some(2),
        ));

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        rib.remove(&prefix, Some(1));

        let routes = rib.get(&prefix).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].path_id, Some(2));
        assert_eq!(rib.ipv4_count(), 1);
    }

    #[test]
    fn peer_rib_add_path_replace_by_path_id() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Some(1),
        ));
        // Replace path_id=1 with a different next_hop
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.99",
            vec![65099],
            Some(1),
        ));

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        let routes = rib.get(&prefix).unwrap();
        assert_eq!(routes.len(), 1); // replaced, not appended
        assert_eq!(routes[0].next_hop, "10.0.0.99".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn peer_rib_non_addpath_overwrites() {
        let mut rib = PeerRib::new();
        // Insert two Add-Path routes
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Some(1),
        ));
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.2",
            vec![65001],
            Some(2),
        ));
        assert_eq!(rib.ipv4_count(), 2);

        // Non-Add-Path insert overwrites the entire Vec
        rib.insert(make_route("10.0.0.0/24", "10.0.0.3", vec![65002]));

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        let routes = rib.get(&prefix).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].path_id, None);
    }

    // --- parse_search tests ---

    #[test]
    fn parse_search_prefix() {
        let filters = parse_search("10.0.0");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Prefix("10.0.0")));

        let filters = parse_search("2001:db8");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Prefix("2001:db8")));

        let filters = parse_search("hello");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Prefix("hello")));
    }

    #[test]
    fn parse_search_asn_uppercase() {
        let filters = parse_search("AS65001");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Asn(asn) => assert_eq!(*asn, 65001),
            _ => panic!("expected Asn variant"),
        }
    }

    #[test]
    fn parse_search_asn_lowercase() {
        let filters = parse_search("as65001");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Asn(asn) => assert_eq!(*asn, 65001),
            _ => panic!("expected Asn variant"),
        }
    }

    #[test]
    fn parse_search_asn_bare_prefix() {
        // "AS" alone without digits → prefix
        let filters = parse_search("AS");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Prefix("AS")));
    }

    #[test]
    fn parse_search_as_path() {
        let filters = parse_search("65000 65001");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::AsPath(path) => assert_eq!(*path, vec![65000, 65001]),
            _ => panic!("expected AsPath variant"),
        }
    }

    #[test]
    fn parse_search_single_number_is_prefix() {
        // A single number without "AS" prefix is treated as prefix search
        let filters = parse_search("65001");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Prefix("65001")));
    }

    #[test]
    fn parse_search_mixed_tokens_is_prefix() {
        // Mixed tokens (not all numbers) → prefix
        let filters = parse_search("65000 abc");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Prefix("65000 abc")));
    }

    #[test]
    fn parse_search_empty() {
        let filters = parse_search("");
        assert!(filters.is_empty());
        let filters = parse_search("   ");
        assert!(filters.is_empty());
    }

    // --- Structured filter parse tests ---

    #[test]
    fn parse_search_community_exact() {
        let filters = parse_search("community:65000:100");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Community { asn, value } => {
                assert_eq!(*asn, Some(65000));
                assert_eq!(*value, Some(100));
            }
            _ => panic!("expected Community variant"),
        }
    }

    #[test]
    fn parse_search_community_wildcard_value() {
        let filters = parse_search("community:65000:*");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Community { asn, value } => {
                assert_eq!(*asn, Some(65000));
                assert_eq!(*value, None);
            }
            _ => panic!("expected Community variant"),
        }
    }

    #[test]
    fn parse_search_community_wildcard_asn() {
        let filters = parse_search("community:*:100");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Community { asn, value } => {
                assert_eq!(*asn, None);
                assert_eq!(*value, Some(100));
            }
            _ => panic!("expected Community variant"),
        }
    }

    #[test]
    fn parse_search_large_community() {
        let filters = parse_search("lc:65000:1:2");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::LargeCommunity {
                global_admin,
                local_data1,
                local_data2,
            } => {
                assert_eq!(*global_admin, Some(65000));
                assert_eq!(*local_data1, Some(1));
                assert_eq!(*local_data2, Some(2));
            }
            _ => panic!("expected LargeCommunity variant"),
        }
    }

    #[test]
    fn parse_search_large_community_wildcard() {
        let filters = parse_search("lc:65000:*:*");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::LargeCommunity {
                global_admin,
                local_data1,
                local_data2,
            } => {
                assert_eq!(*global_admin, Some(65000));
                assert_eq!(*local_data1, None);
                assert_eq!(*local_data2, None);
            }
            _ => panic!("expected LargeCommunity variant"),
        }
    }

    #[test]
    fn parse_search_origin_igp() {
        let filters = parse_search("origin:igp");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Origin(Origin::Igp)));
    }

    #[test]
    fn parse_search_origin_egp() {
        let filters = parse_search("origin:egp");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Origin(Origin::Egp)));
    }

    #[test]
    fn parse_search_origin_incomplete() {
        let filters = parse_search("origin:incomplete");
        assert_eq!(filters.len(), 1);
        assert!(matches!(
            filters[0],
            SearchFilter::Origin(Origin::Incomplete)
        ));
    }

    #[test]
    fn parse_search_origin_case_insensitive() {
        let filters = parse_search("Origin:IGP");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Origin(Origin::Igp)));
    }

    #[test]
    fn parse_search_med_eq() {
        let filters = parse_search("med:100");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Med(cmp) => assert_eq!(*cmp, NumericComparison::Eq(100)),
            _ => panic!("expected Med variant"),
        }
    }

    #[test]
    fn parse_search_med_gt() {
        let filters = parse_search("med:>100");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Med(cmp) => assert_eq!(*cmp, NumericComparison::Gt(100)),
            _ => panic!("expected Med variant"),
        }
    }

    #[test]
    fn parse_search_med_lt() {
        let filters = parse_search("med:<50");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Med(cmp) => assert_eq!(*cmp, NumericComparison::Lt(50)),
            _ => panic!("expected Med variant"),
        }
    }

    #[test]
    fn parse_search_med_gte() {
        let filters = parse_search("med:>=200");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Med(cmp) => assert_eq!(*cmp, NumericComparison::Gte(200)),
            _ => panic!("expected Med variant"),
        }
    }

    #[test]
    fn parse_search_med_lte() {
        let filters = parse_search("med:<=300");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::Med(cmp) => assert_eq!(*cmp, NumericComparison::Lte(300)),
            _ => panic!("expected Med variant"),
        }
    }

    #[test]
    fn parse_search_localpref() {
        let filters = parse_search("localpref:>=200");
        assert_eq!(filters.len(), 1);
        match &filters[0] {
            SearchFilter::LocalPref(cmp) => assert_eq!(*cmp, NumericComparison::Gte(200)),
            _ => panic!("expected LocalPref variant"),
        }
    }

    #[test]
    fn parse_search_multiple_filters_combined() {
        let filters = parse_search("community:65000:100 origin:igp");
        assert_eq!(filters.len(), 2);
        assert!(matches!(
            filters[0],
            SearchFilter::Community {
                asn: Some(65000),
                value: Some(100)
            }
        ));
        assert!(matches!(filters[1], SearchFilter::Origin(Origin::Igp)));
    }

    #[test]
    fn parse_search_structured_plus_prefix() {
        let filters = parse_search("origin:igp 10.0.0");
        assert_eq!(filters.len(), 2);
        assert!(matches!(filters[0], SearchFilter::Origin(Origin::Igp)));
        assert!(matches!(filters[1], SearchFilter::Prefix("10.0.0")));
    }

    #[test]
    fn parse_search_invalid_structured_falls_back_to_prefix() {
        // Invalid community format → treated as prefix
        let filters = parse_search("community:bad");
        assert_eq!(filters.len(), 1);
        assert!(matches!(filters[0], SearchFilter::Prefix("community:bad")));
    }

    // --- NumericComparison tests ---

    #[test]
    fn numeric_comparison_matches() {
        assert!(NumericComparison::Eq(100).matches(100));
        assert!(!NumericComparison::Eq(100).matches(101));

        assert!(NumericComparison::Gt(100).matches(101));
        assert!(!NumericComparison::Gt(100).matches(100));

        assert!(NumericComparison::Lt(100).matches(99));
        assert!(!NumericComparison::Lt(100).matches(100));

        assert!(NumericComparison::Gte(100).matches(100));
        assert!(NumericComparison::Gte(100).matches(101));
        assert!(!NumericComparison::Gte(100).matches(99));

        assert!(NumericComparison::Lte(100).matches(100));
        assert!(NumericComparison::Lte(100).matches(99));
        assert!(!NumericComparison::Lte(100).matches(101));
    }

    // --- as_path_contains_subsequence tests ---

    #[test]
    fn subsequence_found() {
        assert!(as_path_contains_subsequence(
            &[65000, 65001, 65002],
            &[65001, 65002]
        ));
    }

    #[test]
    fn subsequence_not_found() {
        assert!(!as_path_contains_subsequence(
            &[65000, 65001, 65002],
            &[65000, 65002]
        ));
    }

    #[test]
    fn subsequence_empty_pattern() {
        assert!(as_path_contains_subsequence(&[65000, 65001], &[]));
    }

    #[test]
    fn subsequence_single_element() {
        assert!(as_path_contains_subsequence(&[65000, 65001], &[65001]));
        assert!(!as_path_contains_subsequence(&[65000, 65001], &[65002]));
    }

    #[test]
    fn subsequence_pattern_longer_than_path() {
        assert!(!as_path_contains_subsequence(&[65000], &[65000, 65001]));
    }

    // --- Pagination with ASN/AS path filters ---

    #[test]
    fn paginate_ipv4_search_by_asn() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000, 65010]));
        rib.insert(make_route("192.168.1.0/24", "10.0.0.1", vec![65000, 65020]));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("AS65010"));
        assert_eq!(total, 1);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.0.0/24");
    }

    #[test]
    fn paginate_ipv4_search_by_as_path() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000, 65010]));
        rib.insert(make_route("192.168.1.0/24", "10.0.0.1", vec![65000, 65020]));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("65000 65010"));
        assert_eq!(total, 1);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.0.0/24");
    }

    #[test]
    fn paginate_ipv4_search_by_prefix_still_works() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));
        rib.insert(make_route("192.168.1.0/24", "10.0.0.1", vec![65000]));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("192.168"));
        assert_eq!(total, 1);
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].prefix.to_string(), "192.168.1.0/24");
    }

    // --- Pagination with advanced filters ---

    use crate::route::{Community, LargeCommunity};

    fn make_route_with_attrs(
        prefix_str: &str,
        next_hop: &str,
        as_path: Vec<u32>,
        origin: Origin,
        med: Option<u32>,
        local_pref: Option<u32>,
        communities: Vec<Community>,
        large_communities: Vec<LargeCommunity>,
    ) -> BgpRoute {
        let prefix: Prefix = prefix_str.parse().unwrap();
        let origin_as = as_path.last().copied();
        BgpRoute {
            prefix,
            path_id: None,
            origin,
            as_path: vec![AsPathSegment::Sequence(as_path)],
            next_hop: next_hop.parse().unwrap(),
            med,
            local_pref,
            communities,
            ext_communities: vec![],
            large_communities,
            origin_as,
            received_at: Utc::now(),
            stale: false,
            rpki_status: None,
        }
    }

    #[test]
    fn paginate_filter_by_community() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![Community {
                asn: 65000,
                value: 100,
            }],
            vec![],
        ));
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![Community {
                asn: 65000,
                value: 200,
            }],
            vec![],
        ));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("community:65000:100"));
        assert_eq!(total, 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.0.0/24");
    }

    #[test]
    fn paginate_filter_by_community_wildcard() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![Community {
                asn: 65000,
                value: 100,
            }],
            vec![],
        ));
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![Community {
                asn: 65001,
                value: 200,
            }],
            vec![],
        ));

        // Wildcard value: any community from ASN 65000
        let (data, total) = rib.paginate_ipv4(1, 100, Some("community:65000:*"));
        assert_eq!(total, 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.0.0/24");
    }

    #[test]
    fn paginate_filter_by_origin() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![],
            vec![],
        ));
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Incomplete,
            None,
            Some(100),
            vec![],
            vec![],
        ));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("origin:incomplete"));
        assert_eq!(total, 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.1.0/24");
    }

    #[test]
    fn paginate_filter_by_med() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            Some(50),
            Some(100),
            vec![],
            vec![],
        ));
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            Some(200),
            Some(100),
            vec![],
            vec![],
        ));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("med:>100"));
        assert_eq!(total, 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.1.0/24");
    }

    #[test]
    fn paginate_filter_by_localpref() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![],
            vec![],
        ));
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(200),
            vec![],
            vec![],
        ));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("localpref:>=200"));
        assert_eq!(total, 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.1.0/24");
    }

    #[test]
    fn paginate_filter_multiple_and() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(200),
            vec![Community {
                asn: 65000,
                value: 100,
            }],
            vec![],
        ));
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![Community {
                asn: 65000,
                value: 100,
            }],
            vec![],
        ));

        // Both have community:65000:100, but only first has localpref>=200
        let (data, total) = rib.paginate_ipv4(1, 100, Some("community:65000:100 localpref:>=200"));
        assert_eq!(total, 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.0.0/24");
    }

    #[test]
    fn paginate_filter_med_none_not_matched() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None, // No MED
            Some(100),
            vec![],
            vec![],
        ));

        // med:0 should not match routes with no MED
        let (_, total) = rib.paginate_ipv4(1, 100, Some("med:0"));
        assert_eq!(total, 0);
    }

    #[test]
    fn paginate_filter_by_large_community() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![],
            vec![LargeCommunity {
                global_admin: 65000,
                local_data1: 1,
                local_data2: 2,
            }],
        ));
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65000],
            Origin::Igp,
            None,
            Some(100),
            vec![],
            vec![LargeCommunity {
                global_admin: 65001,
                local_data1: 3,
                local_data2: 4,
            }],
        ));

        let (data, total) = rib.paginate_ipv4(1, 100, Some("lc:65000:1:2"));
        assert_eq!(total, 1);
        assert_eq!(data[0].prefix.to_string(), "10.0.0.0/24");
    }

    // --- Stale route tests (Enhanced Route Refresh) ---

    #[test]
    fn mark_stale_marks_all_routes() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));
        rib.insert(make_route("10.0.1.0/24", "10.0.0.1", vec![65001]));
        rib.insert(make_route("2001:db8::/32", "::1", vec![65000]));

        rib.mark_stale();

        // Verify all routes are stale
        for (_prefix, routes) in rib.ipv4.iter() {
            for route in routes {
                assert!(route.stale);
            }
        }
        for (_prefix, routes) in rib.ipv6.iter() {
            for route in routes {
                assert!(route.stale);
            }
        }
    }

    #[test]
    fn sweep_stale_removes_stale_routes() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));
        rib.insert(make_route("10.0.1.0/24", "10.0.0.1", vec![65001]));
        rib.insert(make_route("2001:db8::/32", "::1", vec![65000]));

        // Mark all stale
        rib.mark_stale();

        // Re-insert one route (simulating refresh) — it gets stale=false
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));

        let removed = rib.sweep_stale();
        assert_eq!(removed, 2); // 10.0.1.0/24 and 2001:db8::/32
        assert_eq!(rib.ipv4_count(), 1);
        assert_eq!(rib.ipv6_count(), 0);

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        assert!(rib.get(&prefix).is_some());
    }

    #[test]
    fn sweep_stale_with_add_path() {
        let mut rib = PeerRib::new();
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Some(1),
        ));
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.2",
            vec![65001],
            Some(2),
        ));

        rib.mark_stale();

        // Re-insert only path_id=1
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65000],
            Some(1),
        ));

        let removed = rib.sweep_stale();
        assert_eq!(removed, 1); // path_id=2 removed
        assert_eq!(rib.ipv4_count(), 1);

        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        let routes = rib.get(&prefix).unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].path_id, Some(1));
    }

    #[test]
    fn sweep_stale_no_stale_routes() {
        let mut rib = PeerRib::new();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));

        let removed = rib.sweep_stale();
        assert_eq!(removed, 0);
        assert_eq!(rib.ipv4_count(), 1);
    }
}
