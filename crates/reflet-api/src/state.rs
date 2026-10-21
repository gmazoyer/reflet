use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use reflet_core::asn::AsnStore;
use reflet_core::community::CommunityStore;
use reflet_core::config::BgpConfig;
use reflet_core::event_log::{EventLog, RouteEvent};
use reflet_core::peer::{PeerInfo, PeerState};
use reflet_core::rib::{NextHopSearch, RibStore};
use reflet_core::route::{self, BgpRoute};
use reflet_core::rpki::RpkiStore;
use tokio::sync::mpsc;

use crate::error::ApiError;

/// Shared application state accessible from all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub rib_store: RibStore,
    pub peers: Arc<RwLock<HashMap<String, Arc<RwLock<PeerInfo>>>>>,
    pub bgp_config: BgpConfig,
    pub community_store: Arc<RwLock<CommunityStore>>,
    pub asn_store: Arc<RwLock<AsnStore>>,
    pub title: Arc<RwLock<String>>,
    pub hide_peer_addresses: Arc<RwLock<bool>>,
    pub disable_route_refresh: Arc<RwLock<bool>>,
    pub command_channels:
        Arc<RwLock<HashMap<String, mpsc::Sender<reflet_bgp::refresh::SessionCommand>>>>,
    pub event_log: EventLog,
    pub event_notify: Arc<tokio::sync::Notify>,
    pub shutdown: Arc<AtomicBool>,
    pub rpki_store: Arc<RwLock<RpkiStore>>,
    pub snapshot_data_dir: Option<String>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rib_store: RibStore,
        peers: Arc<RwLock<HashMap<String, Arc<RwLock<PeerInfo>>>>>,
        bgp_config: BgpConfig,
        community_store: Arc<RwLock<CommunityStore>>,
        asn_store: Arc<RwLock<AsnStore>>,
        title: Arc<RwLock<String>>,
        hide_peer_addresses: Arc<RwLock<bool>>,
        disable_route_refresh: Arc<RwLock<bool>>,
        command_channels: Arc<
            RwLock<HashMap<String, mpsc::Sender<reflet_bgp::refresh::SessionCommand>>>,
        >,
        event_log: EventLog,
        event_notify: Arc<tokio::sync::Notify>,
        rpki_store: Arc<RwLock<RpkiStore>>,
        snapshot_data_dir: Option<String>,
    ) -> Self {
        Self {
            rib_store,
            peers,
            bgp_config,
            community_store,
            asn_store,
            title,
            hide_peer_addresses,
            disable_route_refresh,
            command_channels,
            event_log,
            event_notify,
            shutdown: Arc::new(AtomicBool::new(false)),
            rpki_store,
            snapshot_data_dir,
        }
    }

    /// Get a snapshot of all peer infos, ordered by config file position.
    pub fn peer_infos(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().unwrap();
        let mut infos: Vec<PeerInfo> = peers
            .values()
            .filter_map(|p| p.read().ok().map(|info| self.sanitize_peer(info.clone())))
            .collect();
        infos.sort_by_key(|p| p.order);
        infos
    }

    /// Resolve a peer name (from the URL) to its internal peer ID.
    /// Returns 404 if no peer with that name exists.
    pub fn resolve_name(&self, name: &str) -> Result<String, ApiError> {
        let peers = self.peers.read().unwrap();
        for (id, info_lock) in peers.iter() {
            if let Ok(info) = info_lock.read()
                && info.name == name
            {
                return Ok(id.clone());
            }
        }
        Err(ApiError::NotFound(format!("peer '{name}' not found")))
    }

    /// Get a specific peer's info by ID, or return 404.
    pub fn peer_or_404(&self, id: &str) -> Result<PeerInfo, ApiError> {
        self.peer_info(id).ok_or_else(|| {
            ApiError::NotFound(format!("peer {} not found", self.public_peer_id(id)))
        })
    }

    /// Aggregate stats from all peers: (total_peers, established_count, total_ipv4, total_ipv6).
    pub fn peer_stats(&self) -> (usize, usize, usize, usize) {
        let peers = self.peer_infos();
        let total = peers.len();
        let established = peers
            .iter()
            .filter(|p| p.state == PeerState::Established)
            .count();
        let total_v4: usize = peers.iter().map(|p| p.prefixes.ipv4).sum();
        let total_v6: usize = peers.iter().map(|p| p.prefixes.ipv6).sum();
        (total, established, total_v4, total_v6)
    }

    /// Get a specific peer's info by ID.
    pub fn peer_info(&self, id: &str) -> Option<PeerInfo> {
        let peers = self.peers.read().unwrap();
        peers
            .get(id)
            .and_then(|p| p.read().ok().map(|info| self.sanitize_peer(info.clone())))
    }

    /// Request a route refresh for a specific peer.
    pub fn request_route_refresh(&self, peer_id: &str) -> Result<(), ApiError> {
        if *self.disable_route_refresh.read().unwrap() {
            return Err(ApiError::Forbidden("route refresh is disabled".to_string()));
        }
        let tx = self.command_channels.read().unwrap().get(peer_id).cloned();
        let tx = tx.ok_or_else(|| {
            ApiError::NotFound(format!(
                "peer {} has no active session",
                self.public_peer_id(peer_id)
            ))
        })?;
        tx.try_send(reflet_bgp::refresh::SessionCommand::RouteRefresh)
            .map_err(|_| ApiError::Internal("failed to send refresh command".into()))
    }

    fn hides_peer_addresses(&self) -> bool {
        *self.hide_peer_addresses.read().unwrap()
    }

    /// The ID of a peer in API responses. The internal ID is the peer
    /// address, so with hiding on the API shows the peer name. A peer that is
    /// not in the configuration, such as one with a saved RIB, has no name:
    /// its ID shows as the unspecified address.
    pub fn public_peer_id(&self, id: &str) -> String {
        if !self.hides_peer_addresses() {
            return id.to_string();
        }
        let peers = self.peers.read().unwrap();
        if let Some(info) = peers.get(id).and_then(|p| p.read().ok()) {
            return info.name.clone();
        }
        id.parse()
            .map_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED), unspecified)
            .to_string()
    }

    /// The internal ID of the peer that has this public ID, or `None` when
    /// no peer has it. See [`Self::public_peer_id`].
    pub fn internal_peer_id(&self, public_id: &str) -> Option<String> {
        if self.hides_peer_addresses() {
            self.resolve_name(public_id).ok()
        } else {
            Some(public_id.to_string())
        }
    }

    /// Mask sensitive IP fields on a peer info when hiding is enabled.
    fn sanitize_peer(&self, mut peer: PeerInfo) -> PeerInfo {
        if self.hides_peer_addresses() {
            peer.id = peer.name.clone();
            peer.address = unspecified(peer.address);
            peer.router_id = Ipv4Addr::UNSPECIFIED;
        }
        peer
    }

    /// Apply the address-hiding policy to a route. Global next hops stay
    /// visible. Link-local next hops become `::`, because on a session over
    /// link-local addresses they are the address of the peer.
    pub fn sanitize_route(&self, mut route: BgpRoute) -> BgpRoute {
        if self.hides_peer_addresses() {
            route.mask_link_local_next_hops();
        }
        route
    }

    /// Apply the address-hiding policy to a route event: its peer ID, as in
    /// [`Self::public_peer_id`], and its next hop, as in
    /// [`Self::sanitize_route`].
    pub fn sanitize_event(&self, mut event: RouteEvent) -> RouteEvent {
        if self.hides_peer_addresses() {
            event.peer_id = self.public_peer_id(&event.peer_id);
            event.next_hop = event.next_hop.map(route::mask_link_local);
        }
        event
    }

    /// The next hops that a `nexthop:` search can match. A search must not
    /// find a next hop that the address-hiding policy hides.
    pub fn next_hop_search(&self) -> NextHopSearch {
        if self.hides_peer_addresses() {
            NextHopSearch::NotLinkLocal
        } else {
            NextHopSearch::All
        }
    }

    /// Annotate a route with its RPKI validation status.
    pub fn annotate_route(&self, mut route: BgpRoute) -> BgpRoute {
        let rpki_store = self.rpki_store.read().unwrap();
        if !rpki_store.is_empty() {
            route.rpki_status = Some(rpki_store.validate(&route.prefix, route.origin_as));
        }
        route
    }
}

/// The unspecified address of the family of `addr`.
fn unspecified(addr: IpAddr) -> IpAddr {
    match addr {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    }
}
