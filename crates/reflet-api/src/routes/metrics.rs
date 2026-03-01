use std::fmt::Write;

use axum::extract::State;
use axum::http::{HeaderName, StatusCode};
use chrono::Utc;

use reflet_core::peer::PeerState;

use crate::state::AppState;

/// Escape a label value per Prometheus exposition format:
/// backslash → \\, double-quote → \", newline → \n.
fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// GET /metrics — Prometheus exposition format.
pub async fn metrics(
    State(state): State<AppState>,
) -> (StatusCode, [(HeaderName, &'static str); 1], String) {
    let mut buf = String::with_capacity(4096);

    let peers = state.peer_infos();
    let asn = state.bgp_config.local_asn;
    let router_id = state.bgp_config.router_id;

    // --- refletinfo ---
    writeln!(buf, "# HELP refletinfo Instance information.").unwrap();
    writeln!(buf, "# TYPE refletinfo gauge").unwrap();
    writeln!(
        buf,
        "refletinfo{{asn=\"{asn}\",router_id=\"{router_id}\"}} 1"
    )
    .unwrap();

    // --- refletpeers_total ---
    writeln!(buf, "# HELP refletpeers_total Total configured peers.").unwrap();
    writeln!(buf, "# TYPE refletpeers_total gauge").unwrap();
    writeln!(buf, "refletpeers_total {}", peers.len()).unwrap();

    // --- refletpeers_established ---
    let established = peers
        .iter()
        .filter(|p| p.state == PeerState::Established)
        .count();
    writeln!(
        buf,
        "# HELP refletpeers_established Peers in Established state."
    )
    .unwrap();
    writeln!(buf, "# TYPE refletpeers_established gauge").unwrap();
    writeln!(buf, "refletpeers_established {established}").unwrap();

    // --- refletprefixes_total ---
    let total_v4: usize = peers.iter().map(|p| p.prefixes.ipv4).sum();
    let total_v6: usize = peers.iter().map(|p| p.prefixes.ipv6).sum();
    writeln!(
        buf,
        "# HELP refletprefixes_total Total prefixes by address family."
    )
    .unwrap();
    writeln!(buf, "# TYPE refletprefixes_total gauge").unwrap();
    writeln!(buf, "refletprefixes_total{{af=\"ipv4\"}} {total_v4}").unwrap();
    writeln!(buf, "refletprefixes_total{{af=\"ipv6\"}} {total_v6}").unwrap();

    // --- per-peer metrics ---
    writeln!(buf, "# HELP refletpeer_up Whether peer is established.").unwrap();
    writeln!(buf, "# TYPE refletpeer_up gauge").unwrap();
    writeln!(
        buf,
        "# HELP refletpeer_prefixes Prefix count per peer per address family."
    )
    .unwrap();
    writeln!(buf, "# TYPE refletpeer_prefixes gauge").unwrap();
    writeln!(
        buf,
        "# HELP refletpeer_uptime_seconds Seconds since session established."
    )
    .unwrap();
    writeln!(buf, "# TYPE refletpeer_uptime_seconds gauge").unwrap();

    let now = Utc::now();
    for peer in &peers {
        let peer_label = escape_label(&peer.id);
        let asn_label = peer.remote_asn;
        let name_label = escape_label(&peer.name);

        let up: u8 = if peer.state == PeerState::Established {
            1
        } else {
            0
        };
        writeln!(
            buf,
            "refletpeer_up{{peer=\"{peer_label}\",asn=\"{asn_label}\",name=\"{name_label}\"}} {up}"
        )
        .unwrap();

        writeln!(
            buf,
            "refletpeer_prefixes{{peer=\"{peer_label}\",af=\"ipv4\"}} {}",
            peer.prefixes.ipv4
        )
        .unwrap();
        writeln!(
            buf,
            "refletpeer_prefixes{{peer=\"{peer_label}\",af=\"ipv6\"}} {}",
            peer.prefixes.ipv6
        )
        .unwrap();

        let uptime_secs = peer
            .uptime
            .map(|t| (now - t).num_seconds().max(0))
            .unwrap_or(0);
        writeln!(
            buf,
            "refletpeer_uptime_seconds{{peer=\"{peer_label}\"}} {uptime_secs}"
        )
        .unwrap();
    }

    (
        StatusCode::OK,
        [(
            HeaderName::from_static("content-type"),
            "text/plain; version=0.4.0; charset=utf-8",
        )],
        buf,
    )
}
