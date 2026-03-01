use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use prefix_trie::PrefixMap;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::prefix::Prefix;

/// RPKI validation status for a BGP route (RFC 6811).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RpkiStatus {
    Valid,
    Invalid,
    NotFound,
}

/// A Validated ROA Payload.
struct Vrp {
    asn: u32,
    max_length: u8,
}

/// Stores VRPs in prefix-tries for fast covering-prefix lookups.
pub struct RpkiStore {
    ipv4: PrefixMap<Ipv4Net, Vec<Vrp>>,
    ipv6: PrefixMap<Ipv6Net, Vec<Vrp>>,
    vrp_count: usize,
}

impl RpkiStore {
    /// Create an empty store (RPKI disabled).
    pub fn empty() -> Self {
        Self {
            ipv4: PrefixMap::new(),
            ipv6: PrefixMap::new(),
            vrp_count: 0,
        }
    }

    /// Build from a list of parsed VRPs: (prefix, asn, max_length).
    pub fn from_vrps(vrps: Vec<(IpNet, u32, u8)>) -> Self {
        let mut ipv4: PrefixMap<Ipv4Net, Vec<Vrp>> = PrefixMap::new();
        let mut ipv6: PrefixMap<Ipv6Net, Vec<Vrp>> = PrefixMap::new();
        let vrp_count = vrps.len();

        for (prefix, asn, max_length) in vrps {
            match prefix {
                IpNet::V4(net) => {
                    ipv4.entry(net).or_default().push(Vrp { asn, max_length });
                }
                IpNet::V6(net) => {
                    ipv6.entry(net).or_default().push(Vrp { asn, max_length });
                }
            }
        }

        Self {
            ipv4,
            ipv6,
            vrp_count,
        }
    }

    /// Validate a route against stored VRPs (RFC 6811).
    ///
    /// For a route with prefix P/L and origin AS X:
    /// 1. Find all VRPs whose prefix covers P/L
    /// 2. Among those where L <= max_length: if any has ASN == X -> Valid
    /// 3. If covering VRPs exist but none match -> Invalid
    /// 4. If no covering VRPs at all -> NotFound
    pub fn validate(&self, prefix: &Prefix, origin_as: Option<u32>) -> RpkiStatus {
        let origin_as = match origin_as {
            Some(asn) => asn,
            None => return RpkiStatus::NotFound,
        };

        let prefix_len = prefix.prefix_len();

        match prefix {
            Prefix::V4(net) => self.validate_v4(net, prefix_len, origin_as),
            Prefix::V6(net) => self.validate_v6(net, prefix_len, origin_as),
        }
    }

    fn validate_v4(&self, net: &Ipv4Net, prefix_len: u8, origin_as: u32) -> RpkiStatus {
        let mut has_applicable = false;

        for (_covering_prefix, vrps) in self.ipv4.cover(net) {
            for vrp in vrps {
                if prefix_len <= vrp.max_length {
                    has_applicable = true;
                    if vrp.asn == origin_as {
                        return RpkiStatus::Valid;
                    }
                }
            }
        }

        if has_applicable {
            RpkiStatus::Invalid
        } else {
            RpkiStatus::NotFound
        }
    }

    fn validate_v6(&self, net: &Ipv6Net, prefix_len: u8, origin_as: u32) -> RpkiStatus {
        let mut has_applicable = false;

        for (_covering_prefix, vrps) in self.ipv6.cover(net) {
            for vrp in vrps {
                if prefix_len <= vrp.max_length {
                    has_applicable = true;
                    if vrp.asn == origin_as {
                        return RpkiStatus::Valid;
                    }
                }
            }
        }

        if has_applicable {
            RpkiStatus::Invalid
        } else {
            RpkiStatus::NotFound
        }
    }

    /// Number of VRPs in the store.
    pub fn vrp_count(&self) -> usize {
        self.vrp_count
    }

    /// Returns true if the store has no VRPs (RPKI disabled or not yet loaded).
    pub fn is_empty(&self) -> bool {
        self.vrp_count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> RpkiStore {
        // ROA: 10.0.0.0/24, AS 65001, max /24
        // ROA: 10.0.0.0/16, AS 65002, max /24
        // ROA: 2001:db8::/32, AS 65001, max /48
        let vrps = vec![
            ("10.0.0.0/24".parse::<IpNet>().unwrap(), 65001, 24),
            ("10.0.0.0/16".parse::<IpNet>().unwrap(), 65002, 24),
            ("2001:db8::/32".parse::<IpNet>().unwrap(), 65001, 48),
        ];
        RpkiStore::from_vrps(vrps)
    }

    #[test]
    fn valid_route() {
        let store = make_store();
        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65001)), RpkiStatus::Valid);
    }

    #[test]
    fn valid_route_from_covering_prefix() {
        let store = make_store();
        // 10.0.0.0/24 is covered by 10.0.0.0/16, AS 65002, max /24
        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65002)), RpkiStatus::Valid);
    }

    #[test]
    fn invalid_as_mismatch() {
        let store = make_store();
        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        // AS 65099 doesn't match any VRP
        assert_eq!(store.validate(&prefix, Some(65099)), RpkiStatus::Invalid);
    }

    #[test]
    fn invalid_length_violation() {
        let store = make_store();
        // /25 exceeds max_length of /24 for the exact ROA
        // but /25 also exceeds max_length /24 for the /16 covering ROA
        let prefix: Prefix = "10.0.0.0/25".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65001)), RpkiStatus::NotFound);
    }

    #[test]
    fn not_found_no_covering_vrp() {
        let store = make_store();
        let prefix: Prefix = "192.168.0.0/24".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65001)), RpkiStatus::NotFound);
    }

    #[test]
    fn not_found_origin_as_none() {
        let store = make_store();
        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        assert_eq!(store.validate(&prefix, None), RpkiStatus::NotFound);
    }

    #[test]
    fn valid_ipv6() {
        let store = make_store();
        let prefix: Prefix = "2001:db8:1::/48".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65001)), RpkiStatus::Valid);
    }

    #[test]
    fn invalid_ipv6_as_mismatch() {
        let store = make_store();
        let prefix: Prefix = "2001:db8:1::/48".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65099)), RpkiStatus::Invalid);
    }

    #[test]
    fn ipv6_length_exceeds_max() {
        let store = make_store();
        // /64 exceeds max_length /48
        let prefix: Prefix = "2001:db8:1::/64".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65001)), RpkiStatus::NotFound);
    }

    #[test]
    fn empty_store() {
        let store = RpkiStore::empty();
        assert!(store.is_empty());
        assert_eq!(store.vrp_count(), 0);
        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65001)), RpkiStatus::NotFound);
    }

    #[test]
    fn vrp_count() {
        let store = make_store();
        assert_eq!(store.vrp_count(), 3);
        assert!(!store.is_empty());
    }

    #[test]
    fn multiple_covering_vrps_first_match_wins() {
        // Two VRPs for the same prefix, different ASNs
        let vrps = vec![
            ("10.0.0.0/24".parse::<IpNet>().unwrap(), 65001, 24),
            ("10.0.0.0/24".parse::<IpNet>().unwrap(), 65002, 24),
        ];
        let store = RpkiStore::from_vrps(vrps);
        let prefix: Prefix = "10.0.0.0/24".parse().unwrap();
        assert_eq!(store.validate(&prefix, Some(65001)), RpkiStatus::Valid);
        assert_eq!(store.validate(&prefix, Some(65002)), RpkiStatus::Valid);
        assert_eq!(store.validate(&prefix, Some(65099)), RpkiStatus::Invalid);
    }
}
