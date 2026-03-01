use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

use reflet_core::asn::AsnStore;
use reflet_core::community::CommunityStore;
use reflet_core::config::BgpConfig;
use reflet_core::event_log::EventLog;
use reflet_core::peer::{PeerInfo, PeerState};
use reflet_core::rib::RibStore;
use reflet_core::route::BgpRoute;
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
    pub command_channels:
        Arc<RwLock<HashMap<String, mpsc::Sender<reflet_bgp::refresh::SessionCommand>>>>,
    pub event_log: EventLog,
    pub event_notify: Arc<tokio::sync::Notify>,
    pub shutdown: Arc<AtomicBool>,
    pub rpki_store: Arc<RwLock<RpkiStore>>,
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
        command_channels: Arc<
            RwLock<HashMap<String, mpsc::Sender<reflet_bgp::refresh::SessionCommand>>>,
        >,
        event_log: EventLog,
        event_notify: Arc<tokio::sync::Notify>,
        rpki_store: Arc<RwLock<RpkiStore>>,
    ) -> Self {
        Self {
            rib_store,
            peers,
            bgp_config,
            community_store,
            asn_store,
            title,
            hide_peer_addresses,
            command_channels,
            event_log,
            event_notify,
            shutdown: Arc::new(AtomicBool::new(false)),
            rpki_store,
        }
    }

    /// Get a snapshot of all peer infos.
    pub fn peer_infos(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().unwrap();
        peers
            .values()
            .filter_map(|p| p.read().ok().map(|info| self.sanitize_peer(info.clone())))
            .collect()
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
        self.peer_info(id)
            .ok_or_else(|| ApiError::NotFound(format!("peer {id} not found")))
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
        let channels = self.command_channels.read().unwrap();
        let tx = channels
            .get(peer_id)
            .ok_or_else(|| ApiError::NotFound(format!("peer {peer_id} has no active session")))?;
        tx.try_send(reflet_bgp::refresh::SessionCommand::RouteRefresh)
            .map_err(|_| ApiError::Internal("failed to send refresh command".into()))
    }

    /// Mask sensitive IP fields on a peer info when hiding is enabled.
    fn sanitize_peer(&self, mut peer: PeerInfo) -> PeerInfo {
        if *self.hide_peer_addresses.read().unwrap() {
            peer.address = match peer.address {
                IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            };
            peer.router_id = Ipv4Addr::UNSPECIFIED;
        }
        peer
    }

    /// Apply address-hiding policy to a route (currently a no-op;
    /// next-hop is intentionally left visible).
    pub fn sanitize_route(&self, route: BgpRoute) -> BgpRoute {
        route
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
