use std::fmt;
use std::net::{IpAddr, Ipv4Addr};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::prefix::AddressFamily;

/// Unique identifier for a BGP peer.
pub type PeerId = String;

/// BGP session state per RFC 4271.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum PeerState {
    Idle,
    Connect,
    Active,
    OpenSent,
    OpenConfirm,
    Established,
}

/// Prefix counts per address family.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
pub struct PrefixCounts {
    pub ipv4: usize,
    pub ipv6: usize,
}

/// Metadata about a BGP peer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PeerInfo {
    pub id: PeerId,
    #[schema(value_type = String, example = "10.0.0.2")]
    pub address: IpAddr,
    pub remote_asn: u32,
    #[schema(value_type = String, example = "10.0.0.1")]
    pub router_id: Ipv4Addr,
    pub name: String,
    pub description: String,
    pub location: Option<String>,
    pub families: Vec<AddressFamily>,
    pub state: PeerState,
    pub uptime: Option<DateTime<Utc>>,
    pub prefixes: PrefixCounts,
}

impl PeerInfo {
    /// Create a new PeerInfo for a configured peer that is not yet connected.
    pub fn new(
        address: IpAddr,
        remote_asn: u32,
        name: String,
        description: String,
        location: Option<String>,
        families: Vec<AddressFamily>,
    ) -> Self {
        let id = format!("{address}");
        Self {
            id,
            address,
            remote_asn,
            router_id: Ipv4Addr::UNSPECIFIED,
            name,
            description,
            location,
            families,
            state: PeerState::Idle,
            uptime: None,
            prefixes: PrefixCounts::default(),
        }
    }
}

impl PrefixCounts {
    pub fn total(&self) -> usize {
        self.ipv4 + self.ipv6
    }
}

impl fmt::Display for PeerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerState::Idle => write!(f, "Idle"),
            PeerState::Connect => write!(f, "Connect"),
            PeerState::Active => write!(f, "Active"),
            PeerState::OpenSent => write!(f, "OpenSent"),
            PeerState::OpenConfirm => write!(f, "OpenConfirm"),
            PeerState::Established => write!(f, "Established"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_peer_is_idle() {
        let peer = PeerInfo::new(
            "10.0.0.2".parse().unwrap(),
            65001,
            "Test".into(),
            String::new(),
            None,
            vec![AddressFamily::Ipv4Unicast],
        );
        assert_eq!(peer.state, PeerState::Idle);
        assert_eq!(peer.id, "10.0.0.2");
        assert!(peer.uptime.is_none());
        assert!(peer.location.is_none());
        assert_eq!(peer.families, vec![AddressFamily::Ipv4Unicast]);
    }

    #[test]
    fn prefix_counts_total() {
        let counts = PrefixCounts {
            ipv4: 100,
            ipv6: 50,
        };
        assert_eq!(counts.total(), 150);
    }

    #[test]
    fn peer_state_display() {
        assert_eq!(PeerState::Established.to_string(), "Established");
        assert_eq!(PeerState::Idle.to_string(), "Idle");
    }
}
