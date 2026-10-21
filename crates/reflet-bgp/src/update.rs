use std::net::{IpAddr, Ipv6Addr};
use std::sync::RwLock;

use chrono::Utc;
use ipnet::{Ipv4Net, Ipv6Net};
use tracing::{debug, trace, warn};
use zettabgp::prelude::*;

use reflet_core::event_log::{self, EventLog};
use reflet_core::peer::PeerId;
use reflet_core::prefix::Prefix;
use reflet_core::rib::PeerRib;
use reflet_core::route::{
    AsPathSegment, BgpRoute, Community, ExtCommunity, LargeCommunity, Origin,
};

use crate::nexthop::{self, MpNextHop};

/// The zettabgp transport mode for every session, IPv6 sessions included.
///
/// zettabgp reads the classic UPDATE fields (withdrawn routes, NLRI,
/// NEXT_HOP, CLUSTER_LIST) in the format of this mode. RFC 4271 and
/// RFC 4456 define these fields as IPv4 on every transport. In IPv6 mode,
/// zettabgp rejects a 4-byte NEXT_HOP and the session resets. IPv6 routes
/// come in MP_REACH_NLRI and MP_UNREACH_NLRI, which do not use this mode.
pub const CLASSIC_FIELDS_MODE: BgpTransportMode = BgpTransportMode::IPv4;

/// Decode an UPDATE body and apply it to the peer's RIB.
///
/// Reflet reads some path attributes from the raw body, because zettabgp
/// 0.3 decodes them wrongly (see [`crate::nexthop`]).
///
/// A NEXT_HOP that is not 4 bytes long is malformed (RFC 7606 section 7.3).
/// When the UPDATE has routes in its NLRI field, Reflet withdraws all the
/// routes of the UPDATE ("treat-as-withdraw"). Otherwise the NEXT_HOP has
/// no use, and Reflet ignores it (RFC 4760 section 3). The session stays up
/// in both cases.
pub fn decode_and_process(
    body: &[u8],
    params: &BgpSessionParams,
    peer_id: &PeerId,
    rib: &RwLock<PeerRib>,
    has_ipv4: bool,
    has_ipv6: bool,
    event_log: &EventLog,
) -> Result<(), BgpError> {
    let raw = nexthop::scan_attributes(body);
    let mut update = BgpUpdateMessage::new();
    match &raw.malformed_next_hop {
        // zettabgp rejects a NEXT_HOP shorter than 4 bytes, so it must not
        // see the attribute.
        Some(next_hop) => update.decode_from(params, &next_hop.remove_from(body))?,
        None => update.decode_from(params, body)?,
    }

    if let Some(next_hop) = &raw.malformed_next_hop {
        if has_classic_nlri(&update) {
            let count = treat_as_withdraw(&update, peer_id, rib, has_ipv4, has_ipv6, event_log);
            warn!(
                peer = %peer_id,
                length = next_hop.len,
                withdrawn = count,
                "NEXT_HOP is not 4 bytes long, withdrew the routes of the UPDATE (RFC 7606)"
            );
            return Ok(());
        }
        debug!(
            peer = %peer_id,
            length = next_hop.len,
            "ignored a NEXT_HOP that is not 4 bytes long in an UPDATE with no NLRI field"
        );
    }

    process_update(
        &update,
        raw.mp_next_hop,
        params,
        peer_id,
        rib,
        has_ipv4,
        has_ipv6,
        event_log,
    );
    Ok(())
}

fn has_classic_nlri(update: &BgpUpdateMessage) -> bool {
    match &update.updates {
        BgpAddrs::IPV4U(addrs) => !addrs.is_empty(),
        BgpAddrs::IPV4UP(addrs) => !addrs.is_empty(),
        _ => false,
    }
}

/// Withdraw all the routes of an UPDATE, as if they were in its withdrawn
/// routes field ("treat-as-withdraw", RFC 7606 section 2). Returns the
/// number of withdrawn routes.
fn treat_as_withdraw(
    update: &BgpUpdateMessage,
    peer_id: &PeerId,
    rib: &RwLock<PeerRib>,
    has_ipv4: bool,
    has_ipv6: bool,
    event_log: &EventLog,
) -> usize {
    let mut fields = vec![&update.withdraws, &update.updates];
    if let Some(mp_updates) = update.get_mpupdates() {
        fields.push(&mp_updates.addrs);
    }
    if let Some(mp_withdraws) = update.get_mpwithdraws() {
        fields.push(&mp_withdraws.addrs);
    }

    let mut rib = rib.write().unwrap();
    let mut count = 0;
    for addrs in fields {
        let routes: Vec<(Prefix, Option<u32>)> = match addrs {
            BgpAddrs::IPV4U(addrs) if has_ipv4 => addrs
                .iter()
                .map(|addr| (bgp_addr_v4_to_prefix(addr), None))
                .collect(),
            BgpAddrs::IPV4UP(addrs) if has_ipv4 => addrs
                .iter()
                .map(|wp| (bgp_addr_v4_to_prefix(&wp.nlri), Some(wp.pathid)))
                .collect(),
            BgpAddrs::IPV6U(addrs) if has_ipv6 => addrs
                .iter()
                .map(|addr| (bgp_addr_v6_to_prefix(addr), None))
                .collect(),
            BgpAddrs::IPV6UP(addrs) if has_ipv6 => addrs
                .iter()
                .map(|wp| (bgp_addr_v6_to_prefix(&wp.nlri), Some(wp.pathid)))
                .collect(),
            _ => continue,
        };
        for (prefix, path_id) in routes {
            trace!(peer = %peer_id, prefix = %prefix, ?path_id, "withdrawing route (treat-as-withdraw)");
            rib.remove(&prefix, path_id);
            event_log.push_withdraw(peer_id.clone(), prefix, path_id);
            count += 1;
        }
    }
    count
}

/// Process a decoded BGP UPDATE message and apply changes to the peer's RIB.
///
/// Only processes routes for address families that are enabled via `has_ipv4` / `has_ipv6`.
///
/// `mp_next_hop` is the MP_REACH_NLRI next hop read from the raw message
/// (see [`crate::nexthop`]). It replaces the next hop that zettabgp decodes.
#[allow(clippy::too_many_arguments)]
fn process_update(
    update: &BgpUpdateMessage,
    mp_next_hop: Option<MpNextHop>,
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
        mp_next_hop,
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
    next_hop_link_local: Option<Ipv6Addr>,
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
        next_hop_link_local,
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
                    None,
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
                    None,
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
    mp_next_hop: Option<MpNextHop>,
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
        let (next_hop, next_hop_link_local) = match mp_next_hop {
            Some(nh) => (nh.address, nh.link_local),
            None => (mp_nexthop_to_ipaddr(&mp_updates.nexthop), None),
        };
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
                        next_hop_link_local,
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
                        next_hop_link_local,
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
                        next_hop_link_local,
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
                        next_hop_link_local,
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
    use crate::refresh::{AFI_IPV4, AFI_IPV6};
    use crate::test_util::{NLRI_V4, NLRI_V6, UpdateBody, v6};

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

    /// A peer on an IPv6 session, with its own RIB.
    struct TestPeer {
        params: BgpSessionParams,
        peer_id: PeerId,
        rib: RwLock<PeerRib>,
        event_log: EventLog,
    }

    impl TestPeer {
        fn new() -> Self {
            Self {
                params: BgpSessionParams::new(
                    65000,
                    90,
                    CLASSIC_FIELDS_MODE,
                    std::net::Ipv4Addr::new(192, 0, 2, 1),
                    vec![],
                ),
                peer_id: "2001:db8::ffff".to_string(),
                rib: RwLock::new(PeerRib::new()),
                event_log: EventLog::new(10, None).unwrap(),
            }
        }

        /// Process `body` as the session does.
        fn receive(&self, body: &[u8]) -> Result<(), BgpError> {
            decode_and_process(
                body,
                &self.params,
                &self.peer_id,
                &self.rib,
                true,
                true,
                &self.event_log,
            )
        }

        /// The routes in the RIB for `prefix`.
        fn routes(&self, prefix: &str) -> Vec<BgpRoute> {
            let rib = self.rib.read().unwrap();
            rib.get(&prefix.parse().unwrap())
                .cloned()
                .unwrap_or_default()
        }
    }

    /// Process `body` on a new peer and return the routes for `prefix`.
    fn process_raw_update(body: &[u8], prefix: &str) -> Vec<BgpRoute> {
        let peer = TestPeer::new();
        peer.receive(body).unwrap();
        peer.routes(prefix)
    }

    /// IPv4 routes with a classic 4-byte NEXT_HOP, as BIRD sends them over
    /// an IPv6 session when extended next hop is not negotiated.
    #[test]
    fn classic_ipv4_routes_over_ipv6_session() {
        let body = UpdateBody::new()
            .aspath(&[65001])
            .next_hop(&[172, 30, 42, 20])
            .nlri(&[24, 198, 51, 100]) // 198.51.100.0/24
            .nlri(&[24, 203, 0, 113]) // 203.0.113.0/24
            .build();
        for prefix in ["198.51.100.0/24", "203.0.113.0/24"] {
            let routes = process_raw_update(&body, prefix);
            assert_eq!(
                routes[0].next_hop,
                "172.30.42.20".parse::<IpAddr>().unwrap()
            );
            assert_eq!(routes[0].as_path_flat(), vec![65001]);
        }
    }

    #[test]
    fn ipv4_route_with_ipv6_next_hop() {
        let next_hop = [v6("2001:db8::1"), v6("fe80::1")].concat();
        let body = UpdateBody::new()
            .mp_reach(AFI_IPV4, &next_hop, &NLRI_V4)
            .build();
        let routes = process_raw_update(&body, "192.0.2.0/24");
        assert_eq!(routes[0].next_hop, "2001:db8::1".parse::<IpAddr>().unwrap());
        assert_eq!(
            routes[0].next_hop_link_local,
            Some("fe80::1".parse().unwrap())
        );
    }

    #[test]
    fn ipv4_route_with_ipv6_link_local_next_hop() {
        let body = UpdateBody::new()
            .mp_reach(AFI_IPV4, &v6("fe80::1"), &NLRI_V4)
            .build();
        let routes = process_raw_update(&body, "192.0.2.0/24");
        assert_eq!(routes[0].next_hop, "fe80::1".parse::<IpAddr>().unwrap());
        assert_eq!(routes[0].next_hop_link_local, None);
    }

    #[test]
    fn ipv6_route_with_link_local_only_next_hop() {
        let next_hop = [v6("::"), v6("fe80::1")].concat();
        let body = UpdateBody::new()
            .mp_reach(AFI_IPV6, &next_hop, &NLRI_V6)
            .build();
        let routes = process_raw_update(&body, "2001:db8::/32");
        assert_eq!(routes[0].next_hop, "fe80::1".parse::<IpAddr>().unwrap());
        assert_eq!(routes[0].next_hop_link_local, None);
    }

    /// RFC 7606 section 7.3: a NEXT_HOP that is not 4 bytes long is
    /// malformed, and the UPDATE is "treat-as-withdraw". The 16-byte case is
    /// a classic UPDATE with an IPv6 NEXT_HOP on an IPv6 session.
    #[test]
    fn malformed_next_hop_withdraws_all_routes() {
        let next_hops: [&[u8]; 4] = [&[], &[192, 0, 2], &[192, 0, 2, 1, 0], &v6("2001:db8::1")];
        for next_hop in next_hops {
            let peer = TestPeer::new();
            peer.receive(
                &UpdateBody::new()
                    .next_hop(&[192, 0, 2, 1])
                    .nlri(&NLRI_V4)
                    .mp_reach(AFI_IPV6, &v6("2001:db8::1"), &NLRI_V6)
                    .build(),
            )
            .unwrap();
            assert_eq!(peer.routes("192.0.2.0/24").len(), 1);
            assert_eq!(peer.routes("2001:db8::/32").len(), 1);

            let malformed = UpdateBody::new()
                .next_hop(next_hop)
                .nlri(&NLRI_V4)
                .mp_reach(AFI_IPV6, &v6("2001:db8::1"), &NLRI_V6)
                .build();
            let len = next_hop.len();
            assert!(peer.receive(&malformed).is_ok(), "length {len}");
            assert!(peer.routes("192.0.2.0/24").is_empty(), "length {len}");
            assert!(peer.routes("2001:db8::/32").is_empty(), "length {len}");
        }
    }

    /// RFC 4760 section 3: an UPDATE with no routes in the NLRI field
    /// ignores the NEXT_HOP, so a malformed one changes nothing.
    #[test]
    fn malformed_next_hop_without_nlri_is_ignored() {
        let body = UpdateBody::new()
            .next_hop(&[192, 0, 2])
            .mp_reach(AFI_IPV6, &v6("2001:db8::1"), &NLRI_V6)
            .build();
        let routes = process_raw_update(&body, "2001:db8::/32");
        assert_eq!(routes[0].next_hop, "2001:db8::1".parse::<IpAddr>().unwrap());
    }
}
