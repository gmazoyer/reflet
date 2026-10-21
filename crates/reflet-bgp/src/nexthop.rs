//! Next hop handling that zettabgp 0.3 does not cover.
//!
//! - RFC 8950 lets IPv4 routes carry an IPv6 next hop. zettabgp decodes the
//!   next hop of an AFI 1 MP_REACH_NLRI as IPv4 and keeps its first 4 bytes.
//! - RFC 2545 puts a global and a link-local address in a 32-byte next hop.
//!   zettabgp keeps only the first one.
//! - zettabgp keeps the first 4 bytes of a NEXT_HOP that is longer than 4
//!   bytes, and rejects a shorter one. RFC 7606 section 7.3 calls both
//!   malformed.
//! - A peer sends IPv4 routes with an IPv6 next hop only when we announce the
//!   Extended Next Hop Encoding capability (code 5), and zettabgp cannot
//!   encode it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ops::Range;

use crate::refresh::{AFI_IPV4, AFI_IPV6, SAFI_UNICAST};

const ATTR_FLAG_EXTENDED_LENGTH: u8 = 0x10;
const ATTR_TYPE_NEXT_HOP: u8 = 3;
const ATTR_TYPE_MP_REACH_NLRI: u8 = 14;
const OPT_PARAM_CAPABILITIES: u8 = 2;
const CAP_EXTENDED_NEXT_HOP: u8 = 5;

/// Next hop of an MP_REACH_NLRI attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpNextHop {
    /// The global address, or the link-local address when the field holds
    /// no global address.
    pub address: IpAddr,
    /// The link-local address that follows the global address in a 32-byte
    /// next hop field. Always in fe80::/10.
    pub link_local: Option<Ipv6Addr>,
}

/// The path attributes that Reflet reads from the raw UPDATE, because
/// zettabgp 0.3 decodes them wrongly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RawAttributes {
    /// The next hop of the first MP_REACH_NLRI attribute. `None` when the
    /// next hop length is not 4, 16 or 32 bytes (for example a VPN next hop
    /// with a route distinguisher).
    pub mp_next_hop: Option<MpNextHop>,
    /// The first NEXT_HOP attribute, when its value is not 4 bytes long.
    pub malformed_next_hop: Option<MalformedNextHop>,
}

/// A NEXT_HOP attribute with a value that is not 4 bytes long.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedNextHop {
    /// The length of the attribute value.
    pub len: usize,
    /// The position of the attribute, header included, in the UPDATE body.
    span: Range<usize>,
    /// The position of the Total Path Attribute Length field.
    attrs_len_at: usize,
}

impl MalformedNextHop {
    /// Copy `body` without this attribute, so that zettabgp can decode the
    /// other attributes and the routes. `body` must be the UPDATE body that
    /// [`scan_attributes`] read.
    pub fn remove_from(&self, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(body.len() - self.span.len());
        out.extend_from_slice(&body[..self.span.start]);
        out.extend_from_slice(&body[self.span.end..]);
        let at = self.attrs_len_at;
        let attrs_len = u16::from_be_bytes([out[at], out[at + 1]]) - self.span.len() as u16;
        out[at..at + 2].copy_from_slice(&attrs_len.to_be_bytes());
        out
    }
}

/// Read the path attributes of an UPDATE body.
///
/// The result is empty when the body is malformed or has no path
/// attributes, for example an UPDATE with only withdrawn routes. The scan
/// stops at the first truncated attribute.
pub fn scan_attributes(body: &[u8]) -> RawAttributes {
    let mut raw = RawAttributes::default();
    let Some(withdrawn_len) = read_u16(body, 0) else {
        return raw;
    };
    let attrs_len_at = 2 + withdrawn_len as usize;
    let Some(attrs_len) = read_u16(body, attrs_len_at) else {
        return raw;
    };
    let attrs_start = attrs_len_at + 2;
    let Some(attrs) = body.get(..attrs_start + attrs_len as usize) else {
        return raw;
    };

    let mut pos = attrs_start;
    let mut seen_next_hop = false;
    let mut seen_mp_reach = false;
    while let Some((attr_type, value, end)) = read_attribute(attrs, pos) {
        match attr_type {
            ATTR_TYPE_NEXT_HOP if !seen_next_hop => {
                seen_next_hop = true;
                if value.len() != 4 {
                    raw.malformed_next_hop = Some(MalformedNextHop {
                        len: value.len(),
                        span: pos..end,
                        attrs_len_at,
                    });
                }
            }
            ATTR_TYPE_MP_REACH_NLRI if !seen_mp_reach => {
                seen_mp_reach = true;
                raw.mp_next_hop = decode_next_hop(value);
            }
            _ => {}
        }
        pos = end;
    }
    raw
}

/// Read the attribute at `pos` in `attrs`. Returns its type code, its value
/// and the position after it.
fn read_attribute(attrs: &[u8], pos: usize) -> Option<(u8, &[u8], usize)> {
    let flags = *attrs.get(pos)?;
    let attr_type = *attrs.get(pos + 1)?;
    let (header_len, value_len) = if flags & ATTR_FLAG_EXTENDED_LENGTH != 0 {
        (4, read_u16(attrs, pos + 2)? as usize)
    } else {
        (3, *attrs.get(pos + 2)? as usize)
    };
    let start = pos + header_len;
    let value = attrs.get(start..start + value_len)?;
    Some((attr_type, value, start + value_len))
}

/// Decode the next hop from an MP_REACH_NLRI value: AFI (2 bytes),
/// SAFI (1 byte), next hop length (1 byte), next hop.
fn decode_next_hop(value: &[u8]) -> Option<MpNextHop> {
    let len = *value.get(3)? as usize;
    let next_hop = value.get(4..4 + len)?;
    match len {
        4 => Some(MpNextHop {
            address: IpAddr::V4(Ipv4Addr::from(<[u8; 4]>::try_from(next_hop).ok()?)),
            link_local: None,
        }),
        16 => Some(MpNextHop {
            address: IpAddr::V6(read_ipv6(next_hop)?),
            link_local: None,
        }),
        32 => {
            let global = read_ipv6(&next_hop[..16])?;
            let link_local = read_ipv6(&next_hop[16..])?;
            // RFC 2545 puts a link-local address second. Ignore any other
            // address there, as FRR does.
            if !link_local.is_unicast_link_local() {
                return Some(MpNextHop {
                    address: IpAddr::V6(global),
                    link_local: None,
                });
            }
            // Some speakers send "::" or the link-local address again in the
            // global field when the interface has no global address.
            if global.is_unspecified() || global == link_local {
                Some(MpNextHop {
                    address: IpAddr::V6(link_local),
                    link_local: None,
                })
            } else {
                Some(MpNextHop {
                    address: IpAddr::V6(global),
                    link_local: Some(link_local),
                })
            }
        }
        _ => None,
    }
}

fn read_u16(buf: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(buf.get(at..at + 2)?.try_into().ok()?))
}

fn read_ipv6(buf: &[u8]) -> Option<Ipv6Addr> {
    Some(Ipv6Addr::from(<[u8; 16]>::try_from(buf).ok()?))
}

/// The Extended Next Hop Encoding capability (RFC 8950) for IPv4 unicast
/// routes with an IPv6 next hop, encoded as an OPEN optional parameter.
pub fn extended_next_hop_param() -> [u8; 10] {
    let [nlri_afi_hi, nlri_afi_lo] = AFI_IPV4.to_be_bytes();
    let [nlri_safi_hi, nlri_safi_lo] = u16::from(SAFI_UNICAST).to_be_bytes();
    let [next_hop_afi_hi, next_hop_afi_lo] = AFI_IPV6.to_be_bytes();
    [
        OPT_PARAM_CAPABILITIES,
        8,
        CAP_EXTENDED_NEXT_HOP,
        6,
        nlri_afi_hi,
        nlri_afi_lo,
        nlri_safi_hi,
        nlri_safi_lo,
        next_hop_afi_hi,
        next_hop_afi_lo,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refresh::{AFI_IPV4, AFI_IPV6};
    use crate::test_util::{
        ATTR_FLAG_EXTENDED_LENGTH, ATTR_FLAG_OPTIONAL, NLRI_V4, NLRI_V6, UpdateBody,
        mp_reach_value, v6,
    };

    fn parse_mp_reach_next_hop(body: &[u8]) -> Option<MpNextHop> {
        scan_attributes(body).mp_next_hop
    }

    /// An UPDATE body with ORIGIN and one MP_REACH_NLRI.
    fn update_with_mp_reach(afi: u16, next_hop: &[u8], nlri: &[u8]) -> Vec<u8> {
        UpdateBody::new().mp_reach(afi, next_hop, nlri).build()
    }

    #[test]
    fn ipv4_route_with_ipv6_global_next_hop() {
        let body = update_with_mp_reach(AFI_IPV4, &v6("2001:db8::1"), &NLRI_V4);
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "2001:db8::1".parse().unwrap(),
                link_local: None,
            })
        );
    }

    #[test]
    fn ipv4_route_with_ipv6_link_local_next_hop() {
        let body = update_with_mp_reach(AFI_IPV4, &v6("fe80::1"), &NLRI_V4);
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "fe80::1".parse().unwrap(),
                link_local: None,
            })
        );
    }

    #[test]
    fn ipv4_route_with_global_and_link_local_next_hop() {
        let next_hop = [v6("2001:db8::1"), v6("fe80::1")].concat();
        let body = update_with_mp_reach(AFI_IPV4, &next_hop, &NLRI_V4);
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "2001:db8::1".parse().unwrap(),
                link_local: Some("fe80::1".parse().unwrap()),
            })
        );
    }

    /// RFC 2545: the second address of a 32-byte next hop is link-local.
    /// Any other second address is ignored, as FRR does.
    #[test]
    fn second_address_must_be_link_local() {
        let cases = [
            ("2001:db8::1", "::", "2001:db8::1", None),
            ("2001:db8::1", "2001:db8::2", "2001:db8::1", None),
            ("2001:db8::1", "fe80::1", "2001:db8::1", Some("fe80::1")),
            ("::", "2001:db8::2", "::", None),
        ];
        for (global, second, address, link_local) in cases {
            let next_hop = [v6(global), v6(second)].concat();
            let body = update_with_mp_reach(AFI_IPV4, &next_hop, &NLRI_V4);
            assert_eq!(
                parse_mp_reach_next_hop(&body),
                Some(MpNextHop {
                    address: address.parse().unwrap(),
                    link_local: link_local.map(|ll| ll.parse().unwrap()),
                }),
                "next hop {global} {second}"
            );
        }
    }

    #[test]
    fn ipv4_route_with_ipv4_next_hop() {
        let body = update_with_mp_reach(AFI_IPV4, &[192, 0, 2, 1], &NLRI_V4);
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "192.0.2.1".parse().unwrap(),
                link_local: None,
            })
        );
    }

    #[test]
    fn ipv6_route_with_global_and_link_local_next_hop() {
        let next_hop = [v6("2001:db8::1"), v6("fe80::1")].concat();
        // MP_REACH_NLRI with a 2-byte attribute length.
        let body = UpdateBody::new()
            .attr(
                ATTR_FLAG_OPTIONAL | ATTR_FLAG_EXTENDED_LENGTH,
                ATTR_TYPE_MP_REACH_NLRI,
                &mp_reach_value(AFI_IPV6, &next_hop, &NLRI_V6),
            )
            .build();
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "2001:db8::1".parse().unwrap(),
                link_local: Some("fe80::1".parse().unwrap()),
            })
        );
    }

    #[test]
    fn ipv6_route_with_unspecified_global_next_hop() {
        let next_hop = [v6("::"), v6("fe80::1")].concat();
        let body = update_with_mp_reach(AFI_IPV6, &next_hop, &NLRI_V6);
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "fe80::1".parse().unwrap(),
                link_local: None,
            })
        );
    }

    #[test]
    fn ipv6_route_with_link_local_twice() {
        let next_hop = [v6("fe80::1"), v6("fe80::1")].concat();
        let body = update_with_mp_reach(AFI_IPV6, &next_hop, &NLRI_V6);
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "fe80::1".parse().unwrap(),
                link_local: None,
            })
        );
    }

    #[test]
    fn next_hop_after_withdrawn_routes() {
        // One withdrawn IPv4 route (10.0.0.0/8) before the attributes.
        let body = UpdateBody::new()
            .withdrawn(&[8, 10])
            .mp_reach(AFI_IPV6, &v6("2001:db8::1"), &NLRI_V6)
            .build();
        assert_eq!(
            parse_mp_reach_next_hop(&body).map(|nh| nh.address),
            Some("2001:db8::1".parse().unwrap())
        );
    }

    #[test]
    fn no_mp_reach_attribute() {
        let body = UpdateBody::new().nlri(&NLRI_V4).build();
        assert_eq!(parse_mp_reach_next_hop(&body), None);
    }

    #[test]
    fn unsupported_next_hop_length() {
        let body = update_with_mp_reach(AFI_IPV4, &[0; 12], &NLRI_V4);
        assert_eq!(parse_mp_reach_next_hop(&body), None);
    }

    #[test]
    fn truncated_body() {
        let body = update_with_mp_reach(AFI_IPV6, &v6("2001:db8::1"), &NLRI_V6);
        for len in 0..body.len() {
            assert_eq!(parse_mp_reach_next_hop(&body[..len]), None, "length {len}");
        }
    }

    #[test]
    fn malformed_next_hop_lengths() {
        for len in [0, 3, 5, 16] {
            let body = UpdateBody::new()
                .next_hop(&vec![1; len])
                .nlri(&NLRI_V4)
                .build();
            assert_eq!(
                scan_attributes(&body).malformed_next_hop.map(|nh| nh.len),
                Some(len)
            );
        }
        let body = UpdateBody::new()
            .next_hop(&[192, 0, 2, 1])
            .nlri(&NLRI_V4)
            .build();
        assert_eq!(scan_attributes(&body).malformed_next_hop, None);
    }

    #[test]
    fn remove_malformed_next_hop() {
        let body = UpdateBody::new()
            .withdrawn(&[8, 10])
            .next_hop(&v6("2001:db8::1"))
            .mp_reach(AFI_IPV6, &v6("2001:db8::1"), &NLRI_V6)
            .nlri(&NLRI_V4)
            .build();
        let expected = UpdateBody::new()
            .withdrawn(&[8, 10])
            .mp_reach(AFI_IPV6, &v6("2001:db8::1"), &NLRI_V6)
            .nlri(&NLRI_V4)
            .build();
        let next_hop = scan_attributes(&body).malformed_next_hop.unwrap();
        assert_eq!(next_hop.remove_from(&body), expected);
    }

    /// Withdrawn routes only, and the IPv4 End-of-RIB: the scan reads the
    /// two length fields and stops.
    #[test]
    fn update_with_no_path_attributes() {
        for body in [
            UpdateBody::empty().withdrawn(&[8, 10]).build(),
            UpdateBody::empty().build(),
        ] {
            assert_eq!(scan_attributes(&body), RawAttributes::default());
        }
    }

    #[test]
    fn extended_next_hop_param_layout() {
        let param = extended_next_hop_param();
        assert_eq!(param[0], OPT_PARAM_CAPABILITIES);
        assert_eq!(param[1] as usize, param.len() - 2);
        assert_eq!(param[2], CAP_EXTENDED_NEXT_HOP);
        assert_eq!(param[3] as usize, param.len() - 4);
        assert_eq!(&param[4..], &[0, 1, 0, 1, 0, 2]);
    }
}
