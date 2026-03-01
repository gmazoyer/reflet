use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use ipnet::{Ipv4Net, Ipv6Net};
use serde::{Deserialize, Serialize};
use utoipa::PartialSchema;
use utoipa::openapi::{ObjectBuilder, RefOr, Schema, schema};

/// Unified prefix type supporting both IPv4 and IPv6.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Prefix {
    V4(Ipv4Net),
    V6(Ipv6Net),
}

impl PartialSchema for Prefix {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(schema::Type::String)
            .examples([Some(serde_json::json!("10.0.0.0/24"))])
            .description(Some("IPv4 or IPv6 prefix in CIDR notation"))
            .build()
            .into()
    }
}

impl utoipa::ToSchema for Prefix {}

/// Address family identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AddressFamily {
    Ipv4Unicast,
    Ipv6Unicast,
}

impl Prefix {
    /// Returns the address family of this prefix.
    pub fn address_family(&self) -> AddressFamily {
        match self {
            Prefix::V4(_) => AddressFamily::Ipv4Unicast,
            Prefix::V6(_) => AddressFamily::Ipv6Unicast,
        }
    }

    /// Returns the network address as an IpAddr.
    pub fn addr(&self) -> IpAddr {
        match self {
            Prefix::V4(n) => IpAddr::V4(n.addr()),
            Prefix::V6(n) => IpAddr::V6(n.addr()),
        }
    }

    /// Returns the prefix length.
    pub fn prefix_len(&self) -> u8 {
        match self {
            Prefix::V4(n) => n.prefix_len(),
            Prefix::V6(n) => n.prefix_len(),
        }
    }

    /// Check if this prefix contains an IP address.
    pub fn contains(&self, addr: &IpAddr) -> bool {
        match (self, addr) {
            (Prefix::V4(net), IpAddr::V4(ip)) => net.contains(ip),
            (Prefix::V6(net), IpAddr::V6(ip)) => net.contains(ip),
            _ => false,
        }
    }
}

impl fmt::Display for Prefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Prefix::V4(n) => write!(f, "{n}"),
            Prefix::V6(n) => write!(f, "{n}"),
        }
    }
}

impl From<Ipv4Net> for Prefix {
    fn from(net: Ipv4Net) -> Self {
        Prefix::V4(net)
    }
}

impl From<Ipv6Net> for Prefix {
    fn from(net: Ipv6Net) -> Self {
        Prefix::V6(net)
    }
}

impl FromStr for Prefix {
    type Err = ipnet::AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // If no prefix length is given, treat as a host route (/32 or /128).
        if !s.contains('/')
            && let Ok(addr) = s.parse::<IpAddr>()
        {
            return match addr {
                IpAddr::V4(v4) => Ok(Prefix::V4(Ipv4Net::new(v4, 32).unwrap())),
                IpAddr::V6(v6) => Ok(Prefix::V6(Ipv6Net::new(v6, 128).unwrap())),
            };
        }

        if s.contains(':') {
            Ok(Prefix::V6(s.parse()?))
        } else {
            Ok(Prefix::V4(s.parse()?))
        }
    }
}

impl fmt::Display for AddressFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddressFamily::Ipv4Unicast => write!(f, "ipv4-unicast"),
            AddressFamily::Ipv6Unicast => write!(f, "ipv6-unicast"),
        }
    }
}

impl FromStr for AddressFamily {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ipv4-unicast" => Ok(AddressFamily::Ipv4Unicast),
            "ipv6-unicast" => Ok(AddressFamily::Ipv6Unicast),
            _ => Err(format!("unknown address family: {s}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ipv4_prefix() {
        let p: Prefix = "10.0.0.0/24".parse().unwrap();
        assert!(matches!(p, Prefix::V4(_)));
        assert_eq!(p.prefix_len(), 24);
        assert_eq!(p.address_family(), AddressFamily::Ipv4Unicast);
    }

    #[test]
    fn parse_ipv6_prefix() {
        let p: Prefix = "2001:db8::/32".parse().unwrap();
        assert!(matches!(p, Prefix::V6(_)));
        assert_eq!(p.prefix_len(), 32);
        assert_eq!(p.address_family(), AddressFamily::Ipv6Unicast);
    }

    #[test]
    fn prefix_contains_address() {
        let p: Prefix = "10.0.0.0/24".parse().unwrap();
        assert!(p.contains(&"10.0.0.1".parse().unwrap()));
        assert!(!p.contains(&"10.0.1.1".parse().unwrap()));
    }

    #[test]
    fn prefix_display() {
        let p: Prefix = "10.0.0.0/24".parse().unwrap();
        assert_eq!(p.to_string(), "10.0.0.0/24");
    }

    #[test]
    fn address_family_roundtrip() {
        let af = AddressFamily::Ipv4Unicast;
        assert_eq!(af.to_string().parse::<AddressFamily>().unwrap(), af);

        let af = AddressFamily::Ipv6Unicast;
        assert_eq!(af.to_string().parse::<AddressFamily>().unwrap(), af);
    }

    #[test]
    fn parse_ipv4_host_without_prefix_len() {
        let p: Prefix = "10.0.0.1".parse().unwrap();
        assert!(matches!(p, Prefix::V4(_)));
        assert_eq!(p.prefix_len(), 32);
        assert_eq!(p.to_string(), "10.0.0.1/32");
    }

    #[test]
    fn parse_ipv6_host_without_prefix_len() {
        let p: Prefix = "2001:db8::1".parse().unwrap();
        assert!(matches!(p, Prefix::V6(_)));
        assert_eq!(p.prefix_len(), 128);
        assert_eq!(p.to_string(), "2001:db8::1/128");
    }

    #[test]
    fn prefix_serde_json() {
        let p: Prefix = "10.0.0.0/24".parse().unwrap();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"10.0.0.0/24\"");
        let p2: Prefix = serde_json::from_str(&json).unwrap();
        assert_eq!(p, p2);
    }
}
