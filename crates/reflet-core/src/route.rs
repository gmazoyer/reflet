use std::fmt;
use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::prefix::Prefix;
use crate::rpki::RpkiStatus;

/// BGP origin attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum Origin {
    Igp,
    Egp,
    Incomplete,
}

/// An AS path segment (sequence or set).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", content = "asns")]
pub enum AsPathSegment {
    Sequence(Vec<u32>),
    Set(Vec<u32>),
}

/// Standard BGP community (RFC 1997).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct Community {
    pub asn: u16,
    pub value: u16,
}

/// Extended BGP community (RFC 4360).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct ExtCommunity {
    pub type_high: u8,
    pub type_low: u8,
    pub value: [u8; 6],
}

/// Large BGP community (RFC 8092).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub struct LargeCommunity {
    pub global_admin: u32,
    pub local_data1: u32,
    pub local_data2: u32,
}

/// A BGP route with all path attributes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BgpRoute {
    pub prefix: Prefix,
    /// Add-Path path identifier (RFC 7911). `None` for non-Add-Path peers.
    pub path_id: Option<u32>,
    pub origin: Origin,
    pub as_path: Vec<AsPathSegment>,
    #[schema(value_type = String, example = "10.0.0.1")]
    pub next_hop: IpAddr,
    pub med: Option<u32>,
    pub local_pref: Option<u32>,
    pub communities: Vec<Community>,
    pub ext_communities: Vec<ExtCommunity>,
    pub large_communities: Vec<LargeCommunity>,
    /// Origin AS derived from the last ASN in the AS path.
    pub origin_as: Option<u32>,
    pub received_at: DateTime<Utc>,
    /// Whether this route is marked as stale during Enhanced Route Refresh (RFC 7313).
    /// Stale routes are swept after the refresh cycle completes.
    #[serde(skip)]
    pub stale: bool,
    /// RPKI validation status, set at API serve-time (not stored in RIB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpki_status: Option<RpkiStatus>,
}

impl BgpRoute {
    /// Derive the origin AS from the AS path (last ASN in the path).
    pub fn derive_origin_as(as_path: &[AsPathSegment]) -> Option<u32> {
        for segment in as_path.iter().rev() {
            match segment {
                AsPathSegment::Sequence(asns) => {
                    if let Some(&last) = asns.last() {
                        return Some(last);
                    }
                }
                AsPathSegment::Set(asns) => {
                    if let Some(&first) = asns.first() {
                        return Some(first);
                    }
                }
            }
        }
        None
    }

    /// Returns the AS path as a flat list of ASNs (for display).
    pub fn as_path_flat(&self) -> Vec<u32> {
        let mut result = Vec::new();
        for segment in &self.as_path {
            match segment {
                AsPathSegment::Sequence(asns) => result.extend(asns),
                AsPathSegment::Set(asns) => result.extend(asns),
            }
        }
        result
    }

    /// Returns the AS path length (number of ASNs).
    pub fn as_path_length(&self) -> usize {
        self.as_path
            .iter()
            .map(|seg| match seg {
                AsPathSegment::Sequence(asns) => asns.len(),
                AsPathSegment::Set(_) => 1, // AS_SET counts as 1
            })
            .sum()
    }
}

impl fmt::Display for Community {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.asn, self.value)
    }
}

impl fmt::Display for LargeCommunity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.global_admin, self.local_data1, self.local_data2
        )
    }
}

impl fmt::Display for ExtCommunity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:#04x}:{:#04x}:{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.type_high,
            self.type_low,
            self.value[0],
            self.value[1],
            self.value[2],
            self.value[3],
            self.value[4],
            self.value[5]
        )
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Igp => write!(f, "IGP"),
            Origin::Egp => write!(f, "EGP"),
            Origin::Incomplete => write!(f, "Incomplete"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_origin_as_from_sequence() {
        let path = vec![AsPathSegment::Sequence(vec![65000, 65001, 65002])];
        assert_eq!(BgpRoute::derive_origin_as(&path), Some(65002));
    }

    #[test]
    fn derive_origin_as_from_multiple_segments() {
        let path = vec![
            AsPathSegment::Sequence(vec![65000, 65001]),
            AsPathSegment::Sequence(vec![65002]),
        ];
        assert_eq!(BgpRoute::derive_origin_as(&path), Some(65002));
    }

    #[test]
    fn derive_origin_as_empty() {
        let path: Vec<AsPathSegment> = vec![];
        assert_eq!(BgpRoute::derive_origin_as(&path), None);
    }

    #[test]
    fn as_path_length() {
        let route = BgpRoute {
            prefix: "10.0.0.0/24".parse().unwrap(),
            path_id: None,
            origin: Origin::Igp,
            as_path: vec![
                AsPathSegment::Sequence(vec![65000, 65001]),
                AsPathSegment::Set(vec![65002, 65003]),
            ],
            next_hop: "10.0.0.1".parse().unwrap(),
            med: None,
            local_pref: None,
            communities: vec![],
            ext_communities: vec![],
            large_communities: vec![],
            origin_as: None,
            received_at: Utc::now(),
            stale: false,
            rpki_status: None,
        };
        // Sequence(2) + Set(counts as 1) = 3
        assert_eq!(route.as_path_length(), 3);
    }

    #[test]
    fn community_display() {
        let c = Community {
            asn: 65000,
            value: 100,
        };
        assert_eq!(c.to_string(), "65000:100");
    }

    #[test]
    fn large_community_display() {
        let lc = LargeCommunity {
            global_admin: 65000,
            local_data1: 1,
            local_data2: 2,
        };
        assert_eq!(lc.to_string(), "65000:1:2");
    }
}
