use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};
use zettabgp::prelude::*;

use reflet_core::config::{BgpConfig, PeerConfig};
use reflet_core::event_log::EventLog;
use reflet_core::peer::{PeerInfo, PeerState};
use reflet_core::prefix::AddressFamily;
use reflet_core::rib::RibStore;

use crate::graceful_restart::PeerGrInfo;
use crate::refresh::CommandTx;
use crate::session::BgpSession;

/// The BGP speaker: listens for connections and manages per-peer sessions.
pub struct BgpSpeaker {
    bgp_config: BgpConfig,
    peer_configs: Vec<PeerConfig>,
    listen_addr: SocketAddr,
    rib_store: RibStore,
    peers: Arc<RwLock<HashMap<String, Arc<RwLock<PeerInfo>>>>>,
    command_channels: Arc<RwLock<HashMap<String, CommandTx>>>,
    /// Per-peer GR info learned from peer OPEN messages, persists across sessions.
    peer_gr_info: Arc<RwLock<HashMap<String, PeerGrInfo>>>,
    /// Whether this speaker is in the restarting state (R-bit in GR capability).
    is_restarting: Arc<AtomicBool>,
    event_log: EventLog,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl BgpSpeaker {
    pub fn new(
        bgp_config: BgpConfig,
        peer_configs: Vec<PeerConfig>,
        listen_addr: SocketAddr,
        rib_store: RibStore,
        is_restarting: bool,
        event_log: EventLog,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let mut peer_map = HashMap::new();

        // Initialize peer info for each configured peer
        for pc in &peer_configs {
            let info = PeerInfo::new(
                pc.address,
                pc.remote_asn,
                pc.name.clone(),
                pc.description.clone(),
                pc.location.clone(),
                pc.families.clone(),
            );
            peer_map.insert(info.id.clone(), Arc::new(RwLock::new(info)));
        }

        Self {
            bgp_config,
            peer_configs,
            listen_addr,
            rib_store,
            peers: Arc::new(RwLock::new(peer_map)),
            command_channels: Arc::new(RwLock::new(HashMap::new())),
            peer_gr_info: Arc::new(RwLock::new(HashMap::new())),
            is_restarting: Arc::new(AtomicBool::new(is_restarting)),
            event_log,
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Get the `is_restarting` flag (for the global restart timer in main).
    pub fn is_restarting(&self) -> Arc<AtomicBool> {
        self.is_restarting.clone()
    }

    /// Get a snapshot of all peer infos.
    pub fn peer_infos(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().unwrap();
        peers
            .values()
            .filter_map(|p| p.read().ok().map(|info| info.clone()))
            .collect()
    }

    /// Get the shared peers map (for the API).
    pub fn peers(&self) -> Arc<RwLock<HashMap<String, Arc<RwLock<PeerInfo>>>>> {
        self.peers.clone()
    }

    /// Get the shared command channels map (for the API).
    pub fn command_channels(&self) -> Arc<RwLock<HashMap<String, CommandTx>>> {
        self.command_channels.clone()
    }

    /// Get the event log.
    pub fn event_log(&self) -> EventLog {
        self.event_log.clone()
    }

    /// Signal all sessions to shut down.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Run the BGP speaker: listen for incoming connections and dispatch to sessions.
    pub async fn run(&self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.listen_addr).await?;
        info!(addr = %self.listen_addr, "BGP speaker listening");

        // Build a lookup of allowed peer addresses
        let allowed_peers: HashMap<String, &PeerConfig> = self
            .peer_configs
            .iter()
            .map(|pc| (format!("{}", pc.address), pc))
            .collect();

        let mut shutdown_rx = self.shutdown_rx.clone();

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        info!("BGP speaker shutting down");
                        return Ok(());
                    }
                }
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, addr)) => {
                            let peer_ip = format!("{}", addr.ip());
                            if let Some(peer_config) = allowed_peers.get(&peer_ip) {
                                info!(peer = %peer_ip, "accepted BGP connection");
                                self.spawn_session((*peer_config).clone(), stream);
                            } else {
                                warn!(addr = %addr, "rejected connection from unknown peer");
                                drop(stream);
                            }
                        }
                        Err(e) => {
                            error!(error = %e, "failed to accept connection");
                        }
                    }
                }
            }
        }
    }

    fn spawn_session(&self, peer_config: PeerConfig, stream: tokio::net::TcpStream) {
        let peer_id = format!("{}", peer_config.address);
        let params = self.build_session_params(&peer_config);
        let rib_store = self.rib_store.clone();
        let shutdown_rx = self.shutdown_rx.clone();
        let gr_enabled = self.bgp_config.graceful_restart.enabled;

        let peer_info = {
            let peers = self.peers.read().unwrap();
            peers.get(&peer_id).cloned()
        };

        let Some(peer_info) = peer_info else {
            error!(peer = %peer_id, "no peer info found");
            return;
        };

        // Check if peer has stale routes from a prior session (GR recovery)
        let has_stale_routes = if gr_enabled {
            rib_store
                .get(&peer_id)
                .and_then(|rib_arc| rib_arc.read().ok().map(|rib| rib.total_count() > 0))
                .unwrap_or(false)
        } else {
            false
        };

        // Create command channel for this session
        let (cmd_tx, cmd_rx) = mpsc::channel(4);
        {
            let mut channels = self.command_channels.write().unwrap();
            channels.insert(peer_id.clone(), cmd_tx);
        }

        let command_channels = self.command_channels.clone();
        let peer_gr_info = self.peer_gr_info.clone();
        let peers_map = self.peers.clone();
        let event_log = self.event_log.clone();

        tokio::spawn(async move {
            let mut session = BgpSession::new(
                peer_config,
                params,
                rib_store.clone(),
                peer_info.clone(),
                shutdown_rx,
                cmd_rx,
                gr_enabled,
                has_stale_routes,
                peer_gr_info.clone(),
                event_log.clone(),
            );

            let session_result = session.run_with_stream(stream).await;
            let reason = match &session_result {
                Ok(()) => {
                    info!(peer = %peer_id, "session ended cleanly");
                    "session ended cleanly".to_string()
                }
                Err(e) => {
                    warn!(peer = %peer_id, error = %e, "session ended with error");
                    e.to_string()
                }
            };
            event_log.push_session_down(peer_id.clone(), reason);

            // Remove command channel for this session
            {
                let mut channels = command_channels.write().unwrap();
                channels.remove(&peer_id);
            }

            // Determine if we should retain routes (helper mode)
            let peer_gr = peer_gr_info
                .read()
                .ok()
                .and_then(|map| map.get(&peer_id).cloned())
                .unwrap_or_default();

            if gr_enabled && peer_gr.supports_gr && peer_gr.restart_time > 0 {
                // Helper mode: mark routes stale and start a timer
                if let Some(rib_arc) = rib_store.get(&peer_id)
                    && let Ok(mut rib) = rib_arc.write()
                {
                    rib.mark_stale();
                    info!(
                        peer = %peer_id,
                        restart_time = peer_gr.restart_time,
                        "helper mode: marked routes stale, waiting for peer restart"
                    );
                }

                // Reset peer state but keep prefix counts (stale routes still visible)
                {
                    let mut info = peer_info.write().unwrap();
                    info.state = PeerState::Idle;
                    info.uptime = None;
                }

                // Spawn timer task to clear stale routes if peer doesn't reconnect
                let timer_peer_id = peer_id.clone();
                let timer_rib_store = rib_store.clone();
                let timer_peer_info = peer_info.clone();
                let timer_peers = peers_map;
                let restart_time = peer_gr.restart_time;

                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(restart_time as u64)).await;

                    // Check if peer is still Idle (hasn't reconnected)
                    let still_idle = timer_peers
                        .read()
                        .ok()
                        .and_then(|peers| peers.get(&timer_peer_id).cloned())
                        .and_then(|info_arc| {
                            info_arc
                                .read()
                                .ok()
                                .map(|info| info.state == PeerState::Idle)
                        })
                        .unwrap_or(false);

                    if still_idle {
                        if let Some(rib_arc) = timer_rib_store.get(&timer_peer_id)
                            && let Ok(mut rib) = rib_arc.write()
                        {
                            let cleared = rib.total_count();
                            rib.clear();
                            info!(
                                peer = %timer_peer_id,
                                cleared,
                                "GR timer expired, peer did not reconnect — cleared stale routes"
                            );
                        }
                        // Reset prefix counts
                        if let Ok(mut info) = timer_peer_info.write() {
                            info.prefixes = Default::default();
                        }
                    }
                });
            } else {
                // No GR: clear routes immediately (existing behavior)
                {
                    let mut info = peer_info.write().unwrap();
                    info.state = PeerState::Idle;
                    info.uptime = None;
                    info.prefixes = Default::default();
                }

                if let Some(rib_arc) = rib_store.get(&peer_id)
                    && let Ok(mut rib) = rib_arc.write()
                {
                    rib.clear();
                }
            }
        });
    }

    fn build_session_params(&self, peer_config: &PeerConfig) -> BgpSessionParams {
        let peer_mode = match peer_config.address {
            std::net::IpAddr::V4(_) => BgpTransportMode::IPv4,
            std::net::IpAddr::V6(_) => BgpTransportMode::IPv6,
        };

        let mut caps = Vec::new();

        for family in &peer_config.families {
            match family {
                AddressFamily::Ipv4Unicast => caps.push(BgpCapability::SafiIPv4u),
                AddressFamily::Ipv6Unicast => caps.push(BgpCapability::SafiIPv6u),
            }
        }

        // Advertise Route Refresh (RFC 2918) and Enhanced Route Refresh (RFC 7313)
        caps.push(BgpCapability::CapRR);
        caps.push(BgpCapability::CapEnhancedRR);

        // Advertise Add-Path Receive for each configured address family (RFC 7911)
        let addpath_caps: Vec<BgpCapAddPath> = peer_config
            .families
            .iter()
            .map(|f| match f {
                AddressFamily::Ipv4Unicast => BgpCapAddPath {
                    afi: 1,
                    safi: 1,
                    send: false,
                    receive: true,
                },
                AddressFamily::Ipv6Unicast => BgpCapAddPath {
                    afi: 2,
                    safi: 1,
                    send: false,
                    receive: true,
                },
            })
            .collect();
        caps.push(BgpCapability::CapAddPath(addpath_caps));

        // Advertise Graceful Restart (RFC 4724) when enabled
        if self.bgp_config.graceful_restart.enabled {
            let gr_afis: Vec<BgpCapGR> = peer_config
                .families
                .iter()
                .map(|f| match f {
                    AddressFamily::Ipv4Unicast => BgpCapGR {
                        afi: 1,
                        safi: 1,
                        forwarding_state: false, // looking glass doesn't forward
                    },
                    AddressFamily::Ipv6Unicast => BgpCapGR {
                        afi: 2,
                        safi: 1,
                        forwarding_state: false,
                    },
                })
                .collect();
            caps.push(BgpCapability::CapGR {
                restart_time: self.bgp_config.graceful_restart.restart_time,
                restart_state: self.is_restarting.load(Ordering::Relaxed),
                afis: gr_afis,
            });
        }

        // Always advertise 4-byte ASN capability
        caps.push(BgpCapability::CapASN32(self.bgp_config.local_asn));

        BgpSessionParams::new(
            self.bgp_config.local_asn,
            self.bgp_config.hold_time,
            peer_mode,
            self.bgp_config.router_id,
            caps,
        )
    }
}
