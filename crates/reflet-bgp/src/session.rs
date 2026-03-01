use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::time::{Instant, interval, timeout};
use tracing::{debug, error, info, warn};
use zettabgp::prelude::*;

use reflet_core::config::PeerConfig;
use reflet_core::event_log::EventLog;
use reflet_core::peer::{PeerId, PeerInfo, PeerState, PrefixCounts};
use reflet_core::prefix::AddressFamily;
use reflet_core::rib::RibStore;

use crate::codec::{self, BGP_MAX_MSG_SIZE, RawMessageType};
use crate::error::BgpSessionError;
use crate::graceful_restart::{self, PeerGrInfo};
use crate::refresh::{
    AFI_IPV4, AFI_IPV6, CommandRx, ROUTE_REFRESH_BORR, ROUTE_REFRESH_EORR, ROUTE_REFRESH_NORMAL,
    RouteRefreshState, SAFI_UNICAST,
};
use crate::update;

/// A BGP session managing the FSM for a single peer.
pub struct BgpSession {
    peer_config: PeerConfig,
    local_params: BgpSessionParams,
    rib_store: RibStore,
    peer_info: Arc<RwLock<PeerInfo>>,
    shutdown_rx: watch::Receiver<bool>,
    command_rx: CommandRx,
    gr_enabled: bool,
    has_stale_routes: bool,
    peer_gr_info: Arc<RwLock<HashMap<String, PeerGrInfo>>>,
    event_log: EventLog,
}

impl BgpSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        peer_config: PeerConfig,
        local_params: BgpSessionParams,
        rib_store: RibStore,
        peer_info: Arc<RwLock<PeerInfo>>,
        shutdown_rx: watch::Receiver<bool>,
        command_rx: CommandRx,
        gr_enabled: bool,
        has_stale_routes: bool,
        peer_gr_info: Arc<RwLock<HashMap<String, PeerGrInfo>>>,
        event_log: EventLog,
    ) -> Self {
        Self {
            peer_config,
            local_params,
            rib_store,
            peer_info,
            shutdown_rx,
            command_rx,
            gr_enabled,
            has_stale_routes,
            peer_gr_info,
            event_log,
        }
    }

    /// Run the session, accepting a TCP stream from an already-connected peer.
    pub async fn run_with_stream(&mut self, mut stream: TcpStream) -> Result<(), BgpSessionError> {
        let peer_id = self.peer_id();
        let mut buf = vec![0u8; BGP_MAX_MSG_SIZE];
        let mut params = self.local_params.clone();

        info!(peer = %peer_id, "BGP session starting");

        // Transition: Idle -> Connect -> OpenSent
        self.set_state(PeerState::Connect);

        // Send OPEN
        codec::send_open(&mut stream, &params, &mut buf).await?;
        self.set_state(PeerState::OpenSent);
        debug!(peer = %peer_id, "sent OPEN message");

        // Receive OPEN from peer (use read_message_raw to handle type 5)
        let (msg_type, msg_len) = timeout(
            Duration::from_secs(30),
            codec::read_message_raw(&mut stream, &params, &mut buf),
        )
        .await
        .map_err(|_| BgpSessionError::Protocol("timeout waiting for OPEN".into()))??;

        if msg_type != RawMessageType::Standard(BgpMessageType::Open) {
            return Err(BgpSessionError::Protocol(format!(
                "expected OPEN, got {msg_type:?}"
            )));
        }

        let mut open_msg = BgpOpenMessage::new();
        open_msg.decode_from(&params, &buf[..msg_len])?;

        // Match capabilities and negotiate 4-byte ASN
        params.match_caps(&open_msg.caps);
        params.check_caps();

        // Extract the remote ASN: prefer 4-byte ASN from CapASN32 capability,
        // fall back to the 2-byte OPEN header value.
        let remote_asn = open_msg
            .caps
            .iter()
            .find_map(|cap| {
                if let BgpCapability::CapASN32(asn) = cap {
                    Some(*asn)
                } else {
                    None
                }
            })
            .unwrap_or(open_msg.as_num);

        // Check Route Refresh capabilities
        let mut rr_state = RouteRefreshState::default();
        let mut peer_gr = PeerGrInfo::default();
        for cap in &open_msg.caps {
            match cap {
                BgpCapability::CapRR => rr_state.supported = true,
                BgpCapability::CapEnhancedRR => rr_state.enhanced = true,
                BgpCapability::CapGR { restart_time, .. } => {
                    peer_gr.supports_gr = true;
                    peer_gr.restart_time = *restart_time;
                }
                _ => {}
            }
        }
        if rr_state.supported {
            debug!(peer = %peer_id, enhanced = rr_state.enhanced, "peer supports Route Refresh");
        }
        if peer_gr.supports_gr {
            debug!(peer = %peer_id, restart_time = peer_gr.restart_time, "peer supports Graceful Restart");
        }

        // Store peer GR info at the speaker level
        if let Ok(mut map) = self.peer_gr_info.write() {
            map.insert(peer_id.clone(), peer_gr.clone());
        }

        // Handle stale routes from prior session
        let rib_arc_pre = self.rib_store.get_or_create(&peer_id);
        if self.has_stale_routes && !peer_gr.supports_gr {
            // Peer reconnected without GR — clear stale routes immediately
            if let Ok(mut rib) = rib_arc_pre.write() {
                let removed = rib.sweep_stale();
                info!(peer = %peer_id, removed, "peer reconnected without GR, cleared stale routes");
            }
        }

        info!(
            peer = %peer_id,
            remote_as = remote_asn,
            router_id = %open_msg.router_id,
            hold_time = open_msg.hold_time,
            "received OPEN from peer"
        );

        // Update peer info with data from OPEN
        {
            let mut info = self.peer_info.write().unwrap();
            info.router_id = open_msg.router_id;
            info.remote_asn = remote_asn;
        }

        // Transition: OpenSent -> OpenConfirm
        self.set_state(PeerState::OpenConfirm);

        // Send KEEPALIVE to confirm
        codec::send_keepalive(&mut stream, &params, &mut buf).await?;
        debug!(peer = %peer_id, "sent KEEPALIVE (confirming OPEN)");

        // Wait for peer's KEEPALIVE
        let (msg_type, _msg_len) = timeout(
            Duration::from_secs(30),
            codec::read_message_raw(&mut stream, &params, &mut buf),
        )
        .await
        .map_err(|_| BgpSessionError::Protocol("timeout waiting for KEEPALIVE".into()))??;

        if msg_type != RawMessageType::Standard(BgpMessageType::Keepalive) {
            return Err(BgpSessionError::Protocol(format!(
                "expected KEEPALIVE, got {msg_type:?}"
            )));
        }

        // Transition: OpenConfirm -> Established
        self.set_state(PeerState::Established);
        {
            let mut info = self.peer_info.write().unwrap();
            info.uptime = Some(chrono::Utc::now());
        }
        info!(peer = %peer_id, "BGP session ESTABLISHED");
        self.event_log.push_session_up(peer_id.clone(), remote_asn);

        // Determine negotiated hold time
        let negotiated_hold = std::cmp::min(params.hold_time, open_msg.hold_time);
        let keepalive_interval = if negotiated_hold > 0 {
            Duration::from_secs((negotiated_hold / 3).max(1) as u64)
        } else {
            Duration::from_secs(u64::MAX) // effectively disabled
        };
        let hold_timeout = if negotiated_hold > 0 {
            Duration::from_secs(negotiated_hold as u64)
        } else {
            Duration::from_secs(u64::MAX)
        };

        // Determine negotiated address families
        let has_ipv4 = self
            .peer_config
            .families
            .contains(&AddressFamily::Ipv4Unicast);
        let has_ipv6 = self
            .peer_config
            .families
            .contains(&AddressFamily::Ipv6Unicast);

        // Get the RIB for this peer (Arc clone — DashMap shard lock released immediately)
        let rib_arc = self.rib_store.get_or_create(&peer_id);

        // Set up EoR tracking for Graceful Restart
        let mut awaiting_eor = self.gr_enabled && self.has_stale_routes && peer_gr.supports_gr;
        let mut expected_eor_afis: HashSet<(u16, u8)> = HashSet::new();
        let mut eor_received: HashSet<(u16, u8)> = HashSet::new();

        if awaiting_eor {
            for family in &self.peer_config.families {
                match family {
                    AddressFamily::Ipv4Unicast => {
                        expected_eor_afis.insert((1, 1));
                    }
                    AddressFamily::Ipv6Unicast => {
                        expected_eor_afis.insert((2, 1));
                    }
                }
            }
            info!(
                peer = %peer_id,
                afis = ?expected_eor_afis,
                "awaiting End-of-RIB markers for Graceful Restart convergence"
            );
        }

        // Main message loop
        let mut keepalive_timer = interval(keepalive_interval);
        keepalive_timer.tick().await; // consume first immediate tick
        let mut last_received = Instant::now();

        loop {
            tokio::select! {
                // Check for shutdown
                _ = self.shutdown_rx.changed() => {
                    if *self.shutdown_rx.borrow() {
                        info!(peer = %peer_id, "shutdown signal received");
                        let _ = codec::send_notification(
                            &mut stream, &params, 6, 2, &mut buf,
                        ).await;
                        return Err(BgpSessionError::Shutdown);
                    }
                }

                // Session command channel
                cmd = self.command_rx.recv() => {
                    match cmd {
                        Some(crate::refresh::SessionCommand::RouteRefresh) => {
                            if !rr_state.supported {
                                warn!(peer = %peer_id, "peer does not support Route Refresh, ignoring request");
                                continue;
                            }

                            if rr_state.enhanced {
                                // Enhanced Route Refresh (RFC 7313):
                                // 1. Mark all routes stale
                                // 2. Send BoRR for each AFI/SAFI
                                // 3. Send normal ROUTE_REFRESH for each AFI/SAFI
                                info!(peer = %peer_id, "initiating Enhanced Route Refresh");

                                if let Ok(mut rib) = rib_arc.write() {
                                    rib.mark_stale();
                                }

                                if has_ipv4 {
                                    codec::send_route_refresh(&mut stream, AFI_IPV4, SAFI_UNICAST, ROUTE_REFRESH_BORR, &mut buf).await?;
                                    codec::send_route_refresh(&mut stream, AFI_IPV4, SAFI_UNICAST, ROUTE_REFRESH_NORMAL, &mut buf).await?;
                                    rr_state.ipv4_refreshing = true;
                                    debug!(peer = %peer_id, "sent BoRR + ROUTE_REFRESH for IPv4");
                                }
                                if has_ipv6 {
                                    codec::send_route_refresh(&mut stream, AFI_IPV6, SAFI_UNICAST, ROUTE_REFRESH_BORR, &mut buf).await?;
                                    codec::send_route_refresh(&mut stream, AFI_IPV6, SAFI_UNICAST, ROUTE_REFRESH_NORMAL, &mut buf).await?;
                                    rr_state.ipv6_refreshing = true;
                                    debug!(peer = %peer_id, "sent BoRR + ROUTE_REFRESH for IPv6");
                                }
                            } else {
                                // Traditional Route Refresh (RFC 2918):
                                // Just send ROUTE_REFRESH. No stale marking.
                                info!(peer = %peer_id, "initiating Route Refresh (traditional)");

                                if has_ipv4 {
                                    codec::send_route_refresh(&mut stream, AFI_IPV4, SAFI_UNICAST, ROUTE_REFRESH_NORMAL, &mut buf).await?;
                                    debug!(peer = %peer_id, "sent ROUTE_REFRESH for IPv4");
                                }
                                if has_ipv6 {
                                    codec::send_route_refresh(&mut stream, AFI_IPV6, SAFI_UNICAST, ROUTE_REFRESH_NORMAL, &mut buf).await?;
                                    debug!(peer = %peer_id, "sent ROUTE_REFRESH for IPv6");
                                }
                            }
                        }
                        None => {
                            // Command channel closed — speaker dropped the sender
                            debug!(peer = %peer_id, "command channel closed");
                        }
                    }
                }

                // Keepalive timer
                _ = keepalive_timer.tick() => {
                    // Check hold timer
                    if negotiated_hold > 0 && last_received.elapsed() > hold_timeout {
                        error!(peer = %peer_id, "hold timer expired");
                        let _ = codec::send_notification(
                            &mut stream, &params, 4, 0, &mut buf,
                        ).await;
                        return Err(BgpSessionError::HoldTimerExpired);
                    }

                    codec::send_keepalive(&mut stream, &params, &mut buf).await?;
                }

                // Read incoming message
                result = codec::read_message_raw(&mut stream, &params, &mut buf) => {
                    let (msg_type, msg_len) = result?;
                    last_received = Instant::now();

                    match msg_type {
                        RawMessageType::Standard(BgpMessageType::Update) => {
                            // Check for End-of-RIB marker (RFC 4724)
                            if let Some((afi, safi)) = graceful_restart::detect_eor(&buf, msg_len) {
                                info!(peer = %peer_id, afi, safi, "received End-of-RIB");
                                eor_received.insert((afi, safi));

                                if awaiting_eor && expected_eor_afis.iter().all(|a| eor_received.contains(a)) {
                                    if let Ok(mut rib) = rib_arc.write() {
                                        let removed = rib.sweep_stale();
                                        info!(peer = %peer_id, removed, "swept stale routes after GR convergence");
                                    }
                                    awaiting_eor = false;
                                }
                            }

                            // Normal decode + process (EoR messages decode as empty updates)
                            let mut update_msg = BgpUpdateMessage::new();
                            update_msg.decode_from(&params, &buf[..msg_len])?;
                            update::process_update(&update_msg, &params, &peer_id, &rib_arc, has_ipv4, has_ipv6, &self.event_log);

                            // Update prefix counts
                            if let Ok(rib) = rib_arc.read() {
                                let mut info = self.peer_info.write().unwrap();
                                info.prefixes = PrefixCounts {
                                    ipv4: rib.ipv4_count(),
                                    ipv6: rib.ipv6_count(),
                                };
                            }
                        }
                        RawMessageType::Standard(BgpMessageType::Keepalive) => {
                            // Just reset the hold timer (already done above)
                        }
                        RawMessageType::Standard(BgpMessageType::Notification) => {
                            let mut notif = BgpNotificationMessage::new();
                            notif.decode_from(&params, &buf[..msg_len])?;
                            warn!(
                                peer = %peer_id,
                                error_code = notif.error_code,
                                error_subcode = notif.error_subcode,
                                message = %notif.error_text(),
                                "received NOTIFICATION"
                            );
                            return Err(BgpSessionError::Notification {
                                code: notif.error_code,
                                subcode: notif.error_subcode,
                                message: notif.error_text(),
                            });
                        }
                        RawMessageType::Standard(BgpMessageType::Open) => {
                            warn!(peer = %peer_id, "received unexpected OPEN in Established state");
                            let _ = codec::send_notification(
                                &mut stream, &params, 5, 0, &mut buf,
                            ).await;
                            return Err(BgpSessionError::Protocol(
                                "unexpected OPEN in Established state".into()
                            ));
                        }
                        RawMessageType::RouteRefresh => {
                            // Decode the Route Refresh message
                            match codec::decode_route_refresh(&buf, msg_len) {
                                Ok(rr_msg) => {
                                    match rr_msg.subtype {
                                        ROUTE_REFRESH_NORMAL => {
                                            // Peer requesting us to re-send routes —
                                            // we're a looking glass with no routes to send
                                            debug!(
                                                peer = %peer_id,
                                                afi = rr_msg.afi,
                                                safi = rr_msg.safi,
                                                "received ROUTE_REFRESH from peer (ignoring, no routes to send)"
                                            );
                                        }
                                        ROUTE_REFRESH_BORR => {
                                            debug!(
                                                peer = %peer_id,
                                                afi = rr_msg.afi,
                                                safi = rr_msg.safi,
                                                "received BoRR from peer (unexpected direction)"
                                            );
                                        }
                                        ROUTE_REFRESH_EORR => {
                                            // End of Route Refresh — sweep stale routes
                                            let is_refreshing = match rr_msg.afi {
                                                AFI_IPV4 => rr_state.ipv4_refreshing,
                                                AFI_IPV6 => rr_state.ipv6_refreshing,
                                                _ => false,
                                            };

                                            if is_refreshing {
                                                // Check if both AFIs are done
                                                match rr_msg.afi {
                                                    AFI_IPV4 => rr_state.ipv4_refreshing = false,
                                                    AFI_IPV6 => rr_state.ipv6_refreshing = false,
                                                    _ => {}
                                                }

                                                // Sweep when all refreshing AFIs are done
                                                if !rr_state.ipv4_refreshing && !rr_state.ipv6_refreshing {
                                                    if let Ok(mut rib) = rib_arc.write() {
                                                        let removed = rib.sweep_stale();
                                                        info!(
                                                            peer = %peer_id,
                                                            removed,
                                                            "Enhanced Route Refresh complete, swept stale routes"
                                                        );
                                                    }

                                                    // Update prefix counts
                                                    if let Ok(rib) = rib_arc.read() {
                                                        let mut info = self.peer_info.write().unwrap();
                                                        info.prefixes = PrefixCounts {
                                                            ipv4: rib.ipv4_count(),
                                                            ipv6: rib.ipv6_count(),
                                                        };
                                                    }
                                                } else {
                                                    debug!(
                                                        peer = %peer_id,
                                                        afi = rr_msg.afi,
                                                        "received EoRR, waiting for remaining AFI"
                                                    );
                                                }
                                            } else {
                                                warn!(
                                                    peer = %peer_id,
                                                    afi = rr_msg.afi,
                                                    "received EoRR without active refresh, ignoring"
                                                );
                                            }
                                        }
                                        other => {
                                            debug!(
                                                peer = %peer_id,
                                                subtype = other,
                                                "received Route Refresh with unknown subtype"
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(peer = %peer_id, error = %e, "failed to decode Route Refresh message");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn peer_id(&self) -> PeerId {
        format!("{}", self.peer_config.address)
    }

    fn set_state(&self, state: PeerState) {
        let mut info = self.peer_info.write().unwrap();
        info.state = state;
        debug!(peer = %info.id, state = %state, "state transition");
    }
}
