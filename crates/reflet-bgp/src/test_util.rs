//! Builders for raw UPDATE messages in unit tests.

use std::net::Ipv6Addr;

use crate::refresh::SAFI_UNICAST;

pub const ATTR_FLAG_OPTIONAL: u8 = 0x80;
pub const ATTR_FLAG_TRANSITIVE: u8 = 0x40;
pub const ATTR_FLAG_EXTENDED_LENGTH: u8 = 0x10;

pub const ATTR_ORIGIN: u8 = 1;
pub const ATTR_AS_PATH: u8 = 2;
pub const ATTR_NEXT_HOP: u8 = 3;
pub const ATTR_MP_REACH_NLRI: u8 = 14;

/// 192.0.2.0/24 in NLRI encoding.
pub const NLRI_V4: [u8; 4] = [24, 192, 0, 2];
/// 2001:db8::/32 in NLRI encoding.
pub const NLRI_V6: [u8; 5] = [32, 0x20, 0x01, 0x0d, 0xb8];

pub fn v6(s: &str) -> [u8; 16] {
    s.parse::<Ipv6Addr>().unwrap().octets()
}

/// The value of an MP_REACH_NLRI attribute for SAFI unicast.
pub fn mp_reach_value(afi: u16, next_hop: &[u8], nlri: &[u8]) -> Vec<u8> {
    let mut value = Vec::new();
    value.extend_from_slice(&afi.to_be_bytes());
    value.push(SAFI_UNICAST);
    value.push(next_hop.len() as u8);
    value.extend_from_slice(next_hop);
    value.push(0); // reserved
    value.extend_from_slice(nlri);
    value
}

/// The body of an UPDATE message (the message without its header).
#[derive(Default)]
pub struct UpdateBody {
    withdrawn: Vec<u8>,
    attrs: Vec<u8>,
    nlri: Vec<u8>,
}

impl UpdateBody {
    /// An UPDATE with no attributes and no routes.
    pub fn empty() -> Self {
        Self::default()
    }

    /// An UPDATE with an ORIGIN IGP attribute.
    pub fn new() -> Self {
        Self::empty().attr(ATTR_FLAG_TRANSITIVE, ATTR_ORIGIN, &[0])
    }

    /// Add withdrawn IPv4 routes in NLRI encoding.
    pub fn withdrawn(mut self, prefixes: &[u8]) -> Self {
        self.withdrawn.extend_from_slice(prefixes);
        self
    }

    /// Add a path attribute. The length takes 2 bytes when `flags` has the
    /// extended length bit.
    pub fn attr(mut self, flags: u8, type_code: u8, value: &[u8]) -> Self {
        self.attrs.extend_from_slice(&[flags, type_code]);
        if flags & ATTR_FLAG_EXTENDED_LENGTH != 0 {
            self.attrs
                .extend_from_slice(&(value.len() as u16).to_be_bytes());
        } else {
            self.attrs.push(value.len() as u8);
        }
        self.attrs.extend_from_slice(value);
        self
    }

    /// Add an AS_PATH attribute with one AS_SEQUENCE.
    pub fn aspath(self, asns: &[u32]) -> Self {
        let mut value = vec![2, asns.len() as u8];
        for asn in asns {
            value.extend_from_slice(&asn.to_be_bytes());
        }
        self.attr(ATTR_FLAG_TRANSITIVE, ATTR_AS_PATH, &value)
    }

    /// Add a NEXT_HOP attribute with `next_hop` as its value.
    pub fn next_hop(self, next_hop: &[u8]) -> Self {
        self.attr(ATTR_FLAG_TRANSITIVE, ATTR_NEXT_HOP, next_hop)
    }

    /// Add an MP_REACH_NLRI attribute for SAFI unicast.
    pub fn mp_reach(self, afi: u16, next_hop: &[u8], nlri: &[u8]) -> Self {
        self.attr(
            ATTR_FLAG_OPTIONAL,
            ATTR_MP_REACH_NLRI,
            &mp_reach_value(afi, next_hop, nlri),
        )
    }

    /// Add IPv4 routes to the NLRI field.
    pub fn nlri(mut self, prefixes: &[u8]) -> Self {
        self.nlri.extend_from_slice(prefixes);
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&(self.withdrawn.len() as u16).to_be_bytes());
        body.extend_from_slice(&self.withdrawn);
        body.extend_from_slice(&(self.attrs.len() as u16).to_be_bytes());
        body.extend_from_slice(&self.attrs);
        body.extend_from_slice(&self.nlri);
        body
    }
}
