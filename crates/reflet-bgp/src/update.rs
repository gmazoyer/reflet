use std::net::IpAddr;
use std::sync::RwLock;

use chrono::Utc;
use ipnet::{Ipv4Net, Ipv6Net};
use tracing::{debug, trace};
use zettabgp::prelude::*;

use reflet_core::event_log::{self, EventLog};
use reflet_core::peer::PeerId;
use reflet_core::prefix::Prefix;
use reflet_core::rib::PeerRib;
use reflet_core::route::{
    AsPathSegment, BgpRoute, Community, ExtCommunity, LargeCommunity, Origin,
};

/// Process a decoded BGP UPDATE message and apply changes to the peer's RIB.
///
/// Only processes routes for address families that are enabled via `has_ipv4` / `has_ipv6`.
pub fn process_update(
    update: &BgpUpdateMessage,
    params: &BgpSessionParams,
    peer_id: &PeerId,
    rib: &RwLock<PeerRib>,
    has_ipv4: bool,
    has_ipv6: bool,
    event_log: &EventLog,
) {
    // Extract path attributes that apply to all NLRI in this UPDATE
    let origin = extract_origin(update);
    let as_path = extract_as_path(update);
    let next_hop = extract_next_hop(update);
    let med = extract_med(update);
    let local_pref = extract_local_pref(update);
    let communities = extract_communities(update);
    let ext_communities = extract_ext_communities(update);
    let large_communities = extract_large_communities(update);
    let origin_as = BgpRoute::derive_origin_as(&as_path);
    let now = Utc::now();

    // Process IPv4 withdrawals (standard WITHDRAWN ROUTES field is always IPv4)
    if has_ipv4 {
        let withdrawn_count = process_withdrawals(update, peer_id, rib, event_log);
        if withdrawn_count > 0 {
            debug!(peer = %peer_id, count = withdrawn_count, "processed IPv4 withdrawals");
        }
    }

    // Process IPv4 NLRI announcements (standard NLRI field is always IPv4)
    if has_ipv4 {
        let announced_count = process_ipv4_nlri(
            update,
            peer_id,
            rib,
            &origin,
            &as_path,
            &next_hop,
            &med,
            &local_pref,
            &communities,
            &ext_communities,
            &large_communities,
            &origin_as,
            &now,
            event_log,
        );
        if announced_count > 0 {
            debug!(peer = %peer_id, count = announced_count, "processed IPv4 announcements");
        }
    }

    // Process MP_REACH_NLRI (may contain IPv4 or IPv6)
    let mp_announced = process_mp_reach(
        update,
        params,
        peer_id,
        rib,
        &origin,
        &as_path,
        &med,
        &local_pref,
        &communities,
        &ext_communities,
        &large_communities,
        &origin_as,
        &now,
        has_ipv4,
        has_ipv6,
        event_log,
    );
    if mp_announced > 0 {
        debug!(peer = %peer_id, count = mp_announced, "processed MP_REACH announcements");
    }

    // Process MP_UNREACH_NLRI (may contain IPv4 or IPv6)
    let mp_withdrawn = process_mp_unreach(update, peer_id, rib, has_ipv4, has_ipv6, event_log);
    if mp_withdrawn > 0 {
        debug!(peer = %peer_id, count = mp_withdrawn, "processed MP_UNREACH withdrawals");
    }
}

fn process_withdrawals(
    update: &BgpUpdateMessage,
    peer_id: &PeerId,
    rib: &RwLock<PeerRib>,
    event_log: &EventLog,
) -> usize {
    let mut count = 0;
    match &update.withdraws {
        BgpAddrs::IPV4U(addrs) => {
            let mut rib = rib.write().unwrap();
            for addr in addrs {
                let prefix = bgp_addr_v4_to_prefix(addr);
                trace!(peer = %peer_id, prefix = %prefix, "withdrawing route");
                rib.remove(&prefix, None);
                event_log.push_withdraw(peer_id.clone(), prefix, None);
                count += 1;
            }
        }
        BgpAddrs::IPV4UP(addrs) => {
            let mut rib = rib.write().unwrap();
            for wp in addrs {
                let prefix = bgp_addr_v4_to_prefix(&wp.nlri);
                trace!(peer = %peer_id, prefix = %prefix, path_id = wp.pathid, "withdrawing Add-Path route");
                rib.remove(&prefix, Some(wp.pathid));
                event_log.push_withdraw(peer_id.clone(), prefix, Some(wp.pathid));
                count += 1;
            }
        }
        _ => {}
    }
    count
}

#[allow(clippy::too_many_arguments)]
fn build_route(
    prefix: Prefix,
    path_id: Option<u32>,
    origin: &Origin,
    as_path: &[AsPathSegment],
    next_hop: IpAddr,
    med: &Option<u32>,
    local_pref: &Option<u32>,
    communities: &[Community],
    ext_communities: &[ExtCommunity],
    large_communities: &[LargeCommunity],
    origin_as: &Option<u32>,
    now: &chrono::DateTime<Utc>,
) -> BgpRoute {
    BgpRoute {
        prefix,
        path_id,
        origin: *origin,
        as_path: as_path.to_vec(),
        next_hop,
        med: *med,
        local_pref: *local_pref,
        communities: communities.to_vec(),
        ext_communities: ext_communities.to_vec(),
        large_communities: large_communities.to_vec(),
        origin_as: *origin_as,
        received_at: *now,
        stale: false,
        rpki_status: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn process_ipv4_nlri(
    update: &BgpUpdateMessage,
    peer_id: &PeerId,
    rib: &RwLock<PeerRib>,
    origin: &Origin,
    as_path: &[AsPathSegment],
    next_hop: &Option<IpAddr>,
    med: &Option<u32>,
    local_pref: &Option<u32>,
    communities: &[Community],
    ext_communities: &[ExtCommunity],
    large_communities: &[LargeCommunity],
    origin_as: &Option<u32>,
    now: &chrono::DateTime<Utc>,
    event_log: &EventLog,
) -> usize {
    let mut count = 0;
    let nh = next_hop.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    match &update.updates {
        BgpAddrs::IPV4U(addrs) => {
            let mut rib = rib.write().unwrap();
            for addr in addrs {
                let prefix = bgp_addr_v4_to_prefix(addr);
                let route = build_route(
                    prefix.clone(),
                    None,
                    origin,
                    as_path,
                    nh,
                    med,
                    local_pref,
                    communities,
                    ext_communities,
                    large_communities,
                    origin_as,
                    now,
                );
                trace!(peer = %peer_id, prefix = %prefix, "inserting IPv4 route");
                rib.insert(route);
                event_log.push_announce(
                    peer_id.clone(),
                    prefix,
                    None,
                    event_log::flatten_as_path(as_path),
                    nh,
                    *origin_as,
                );
                count += 1;
            }
        }
        BgpAddrs::IPV4UP(addrs) => {
            let mut rib = rib.write().unwrap();
            for wp in addrs {
                let prefix = bgp_addr_v4_to_prefix(&wp.nlri);
                let route = build_route(
                    prefix.clone(),
                    Some(wp.pathid),
                    origin,
                    as_path,
                    nh,
                    med,
                    local_pref,
                    communities,
                    ext_communities,
                    large_communities,
                    origin_as,
                    now,
                );
                trace!(peer = %peer_id, prefix = %prefix, path_id = wp.pathid, "inserting IPv4 Add-Path route");
                rib.insert(route);
                event_log.push_announce(
                    peer_id.clone(),
                    prefix,
                    Some(wp.pathid),
                    event_log::flatten_as_path(as_path),
                    nh,
                    *origin_as,
                );
                count += 1;
            }
        }
        _ => {}
    }
    count
}

#[allow(clippy::too_many_arguments)]
fn process_mp_reach(
    update: &BgpUpdateMessage,
    _params: &BgpSessionParams,
    peer_id: &PeerId,
    rib: &RwLock<PeerRib>,
    origin: &Origin,
    as_path: &[AsPathSegment],
    med: &Option<u32>,
    local_pref: &Option<u32>,
    communities: &[Community],
    ext_communities: &[ExtCommunity],
    large_communities: &[LargeCommunity],
    origin_as: &Option<u32>,
    now: &chrono::DateTime<Utc>,
    has_ipv4: bool,
    has_ipv6: bool,
    event_log: &EventLog,
) -> usize {
    let mut count = 0;
    if let Some(mp_updates) = update.get_mpupdates() {
        let next_hop = mp_nexthop_to_ipaddr(&mp_updates.nexthop);
        let flat_as_path = event_log::flatten_as_path(as_path);

        match &mp_updates.addrs {
            BgpAddrs::IPV6U(addrs) if has_ipv6 => {
                let mut rib = rib.write().unwrap();
                for addr in addrs {
                    let prefix = bgp_addr_v6_to_prefix(addr);
                    let route = build_route(
                        prefix.clone(),
                        None,
                        origin,
                        as_path,
                        next_hop,
                        med,
                        local_pref,
                        communities,
                        ext_communities,
                        large_communities,
                        origin_as,
                        now,
                    );
                    trace!(peer = %peer_id, prefix = %prefix, "inserting IPv6 route");
                    rib.insert(route);
                    event_log.push_announce(
                        peer_id.clone(),
                        prefix,
                        None,
                        flat_as_path.clone(),
                        next_hop,
                        *origin_as,
                    );
                    count += 1;
                }
            }
            BgpAddrs::IPV6UP(addrs) if has_ipv6 => {
                let mut rib = rib.write().unwrap();
                for wp in addrs {
                    let prefix = bgp_addr_v6_to_prefix(&wp.nlri);
                    let route = build_route(
                        prefix.clone(),
                        Some(wp.pathid),
                        origin,
                        as_path,
                        next_hop,
                        med,
                        local_pref,
                        communities,
                        ext_communities,
                        large_communities,
                        origin_as,
                        now,
                    );
                    trace!(peer = %peer_id, prefix = %prefix, path_id = wp.pathid, "inserting IPv6 Add-Path route");
                    rib.insert(route);
                    event_log.push_announce(
                        peer_id.clone(),
                        prefix,
                        Some(wp.pathid),
                        flat_as_path.clone(),
                        next_hop,
                        *origin_as,
                    );
                    count += 1;
                }
            }
            BgpAddrs::IPV4U(addrs) if has_ipv4 => {
                let mut rib = rib.write().unwrap();
                for addr in addrs {
                    let prefix = bgp_addr_v4_to_prefix(addr);
                    let route = build_route(
                        prefix.clone(),
                        None,
                        origin,
                        as_path,
                        next_hop,
                        med,
                        local_pref,
                        communities,
                        ext_communities,
                        large_communities,
                        origin_as,
                        now,
                    );
                    trace!(peer = %peer_id, prefix = %prefix, "inserting IPv4 MP route");
                    rib.insert(route);
                    event_log.push_announce(
                        peer_id.clone(),
                        prefix,
                        None,
                        flat_as_path.clone(),
                        next_hop,
                        *origin_as,
                    );
                    count += 1;
                }
            }
            BgpAddrs::IPV4UP(addrs) if has_ipv4 => {
                let mut rib = rib.write().unwrap();
                for wp in addrs {
                    let prefix = bgp_addr_v4_to_prefix(&wp.nlri);
                    let route = build_route(
                        prefix.clone(),
                        Some(wp.pathid),
                        origin,
                        as_path,
                        next_hop,
                        med,
                        local_pref,
                        communities,
                        ext_communities,
                        large_communities,
                        origin_as,
                        now,
                    );
                    trace!(peer = %peer_id, prefix = %prefix, path_id = wp.pathid, "inserting IPv4 MP Add-Path route");
                    rib.insert(route);
                    event_log.push_announce(
                        peer_id.clone(),
                        prefix,
                        Some(wp.pathid),
                        flat_as_path.clone(),
                        next_hop,
                        *origin_as,
                    );
                    count += 1;
                }
            }
            _ => {
                debug!(peer = %peer_id, "unsupported MP_REACH address family, skipping");
            }
        }
    }
    count
}

fn process_mp_unreach(
    update: &BgpUpdateMessage,
    peer_id: &PeerId,
    rib: &RwLock<PeerRib>,
    has_ipv4: bool,
    has_ipv6: bool,
    event_log: &EventLog,
) -> usize {
    let mut count = 0;
    if let Some(mp_withdraws) = update.get_mpwithdraws() {
        match &mp_withdraws.addrs {
            BgpAddrs::IPV6U(addrs) if has_ipv6 => {
                let mut rib = rib.write().unwrap();
                for addr in addrs {
                    let prefix = bgp_addr_v6_to_prefix(addr);
                    trace!(peer = %peer_id, prefix = %prefix, "withdrawing IPv6 route");
                    rib.remove(&prefix, None);
                    event_log.push_withdraw(peer_id.clone(), prefix, None);
                    count += 1;
                }
            }
            BgpAddrs::IPV6UP(addrs) if has_ipv6 => {
                let mut rib = rib.write().unwrap();
                for wp in addrs {
                    let prefix = bgp_addr_v6_to_prefix(&wp.nlri);
                    trace!(peer = %peer_id, prefix = %prefix, path_id = wp.pathid, "withdrawing IPv6 Add-Path route");
                    rib.remove(&prefix, Some(wp.pathid));
                    event_log.push_withdraw(peer_id.clone(), prefix, Some(wp.pathid));
                    count += 1;
                }
            }
            BgpAddrs::IPV4U(addrs) if has_ipv4 => {
                let mut rib = rib.write().unwrap();
                for addr in addrs {
                    let prefix = bgp_addr_v4_to_prefix(addr);
                    trace!(peer = %peer_id, prefix = %prefix, "withdrawing IPv4 MP route");
                    rib.remove(&prefix, None);
                    event_log.push_withdraw(peer_id.clone(), prefix, None);
                    count += 1;
                }
            }
            BgpAddrs::IPV4UP(addrs) if has_ipv4 => {
                let mut rib = rib.write().unwrap();
                for wp in addrs {
                    let prefix = bgp_addr_v4_to_prefix(&wp.nlri);
                    trace!(peer = %peer_id, prefix = %prefix, path_id = wp.pathid, "withdrawing IPv4 MP Add-Path route");
                    rib.remove(&prefix, Some(wp.pathid));
                    event_log.push_withdraw(peer_id.clone(), prefix, Some(wp.pathid));
                    count += 1;
                }
            }
            _ => {
                debug!(peer = %peer_id, "skipping MP_UNREACH (unsupported or unconfigured address family)");
            }
        }
    }
    count
}

// --- Conversion helpers ---

fn bgp_addr_v4_to_prefix(addr: &BgpAddrV4) -> Prefix {
    let net = Ipv4Net::new(addr.addr, addr.prefixlen)
        .unwrap_or_else(|_| Ipv4Net::new(addr.addr, 32).unwrap());
    Prefix::V4(net)
}

fn bgp_addr_v6_to_prefix(addr: &BgpAddrV6) -> Prefix {
    let net = Ipv6Net::new(addr.addr, addr.prefixlen)
        .unwrap_or_else(|_| Ipv6Net::new(addr.addr, 128).unwrap());
    Prefix::V6(net)
}

fn mp_nexthop_to_ipaddr(nexthop: &BgpAddr) -> IpAddr {
    match nexthop {
        BgpAddr::V4(addr) => IpAddr::V4(*addr),
        BgpAddr::V6(addr) => IpAddr::V6(*addr),
        _ => IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
    }
}

fn extract_origin(update: &BgpUpdateMessage) -> Origin {
    update
        .get_attr_origin()
        .map(|o| match o.value {
            BgpAttrOrigin::Igp => Origin::Igp,
            BgpAttrOrigin::Egp => Origin::Egp,
            BgpAttrOrigin::Incomplete => Origin::Incomplete,
        })
        .unwrap_or(Origin::Incomplete)
}

fn extract_as_path(update: &BgpUpdateMessage) -> Vec<AsPathSegment> {
    update
        .get_attr_aspath()
        .map(|asp| {
            asp.value
                .iter()
                .map(|item| match item {
                    BgpASitem::Seq(seq) => {
                        AsPathSegment::Sequence(seq.value.iter().map(|a| a.value).collect())
                    }
                    BgpASitem::Set(set) => {
                        AsPathSegment::Set(set.value.iter().map(|a| a.value).collect())
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_next_hop(update: &BgpUpdateMessage) -> Option<IpAddr> {
    update.get_attr_nexthop().map(|nh| nh.value)
}

fn extract_med(update: &BgpUpdateMessage) -> Option<u32> {
    for attr in &update.attrs {
        if let BgpAttrItem::MED(med) = attr {
            return Some(med.value);
        }
    }
    None
}

fn extract_local_pref(update: &BgpUpdateMessage) -> Option<u32> {
    for attr in &update.attrs {
        if let BgpAttrItem::LocalPref(lp) = attr {
            return Some(lp.value);
        }
    }
    None
}

fn extract_communities(update: &BgpUpdateMessage) -> Vec<Community> {
    update
        .get_attr_communitylist()
        .map(|cl| {
            cl.value
                .iter()
                .map(|c| Community {
                    asn: (c.value >> 16) as u16,
                    value: (c.value & 0xFFFF) as u16,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_ext_communities(update: &BgpUpdateMessage) -> Vec<ExtCommunity> {
    update
        .get_attr_extcommunitylist()
        .map(|ecl| {
            ecl.value
                .iter()
                .map(|ec| {
                    let a_bytes = ec.a.to_be_bytes();
                    let b_bytes = ec.b.to_be_bytes();
                    ExtCommunity {
                        type_high: ec.ctype,
                        type_low: ec.subtype,
                        value: [
                            a_bytes[0], a_bytes[1], b_bytes[0], b_bytes[1], b_bytes[2], b_bytes[3],
                        ],
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_large_communities(update: &BgpUpdateMessage) -> Vec<LargeCommunity> {
    update
        .get_attr_largecommunitylist()
        .map(|lcl| {
            lcl.value
                .iter()
                .map(|lc| LargeCommunity {
                    global_admin: lc.ga,
                    local_data1: lc.ldp1,
                    local_data2: lc.ldp2,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bgp_addr_v4_to_prefix() {
        let addr = BgpAddrV4::new(std::net::Ipv4Addr::new(10, 0, 0, 0), 24);
        let prefix = bgp_addr_v4_to_prefix(&addr);
        assert_eq!(prefix.to_string(), "10.0.0.0/24");
    }

    #[test]
    fn test_bgp_addr_v6_to_prefix() {
        let addr = BgpAddrV6::new("2001:db8::".parse().unwrap(), 32);
        let prefix = bgp_addr_v6_to_prefix(&addr);
        assert_eq!(prefix.to_string(), "2001:db8::/32");
    }

    #[test]
    fn test_origin_extraction() {
        let update = BgpUpdateMessage::new();
        // Empty update should return Incomplete
        assert_eq!(extract_origin(&update), Origin::Incomplete);
    }

    #[test]
    fn test_as_path_extraction_empty() {
        let update = BgpUpdateMessage::new();
        assert!(extract_as_path(&update).is_empty());
    }
}
