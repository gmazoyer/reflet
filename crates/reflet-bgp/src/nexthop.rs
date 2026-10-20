//! Next hop handling that zettabgp 0.3 does not cover.
//!
//! - RFC 8950 lets IPv4 routes carry an IPv6 next hop. zettabgp decodes the
//!   next hop of an AFI 1 MP_REACH_NLRI as IPv4 and keeps its first 4 bytes.
//! - RFC 2545 puts a global and a link-local address in a 32-byte next hop.
//!   zettabgp keeps only the first one.
//! - A peer sends IPv4 routes with an IPv6 next hop only when we announce the
//!   Extended Next Hop Encoding capability (code 5), and zettabgp cannot
//!   encode it.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const ATTR_FLAG_EXTENDED_LENGTH: u8 = 0x10;
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
    /// next hop field.
    pub link_local: Option<Ipv6Addr>,
}

/// Read the next hop of the MP_REACH_NLRI attribute in an UPDATE body.
///
/// Returns `None` when the UPDATE has no MP_REACH_NLRI, when the body is
/// malformed, or when the next hop length is not 4, 16 or 32 bytes (for
/// example a VPN next hop with a route distinguisher).
pub fn parse_mp_reach_next_hop(body: &[u8]) -> Option<MpNextHop> {
    let withdrawn_len = read_u16(body, 0)? as usize;
    let attrs_len_at = 2 + withdrawn_len;
    let attrs_len = read_u16(body, attrs_len_at)? as usize;
    let attrs_start = attrs_len_at + 2;
    let mut attrs = body.get(attrs_start..attrs_start + attrs_len)?;

    while !attrs.is_empty() {
        let flags = *attrs.first()?;
        let attr_type = *attrs.get(1)?;
        let (header_len, value_len) = if flags & ATTR_FLAG_EXTENDED_LENGTH != 0 {
            (4, read_u16(attrs, 2)? as usize)
        } else {
            (3, *attrs.get(2)? as usize)
        };
        let value = attrs.get(header_len..header_len + value_len)?;
        if attr_type == ATTR_TYPE_MP_REACH_NLRI {
            return decode_next_hop(value);
        }
        attrs = &attrs[header_len + value_len..];
    }
    None
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
    [
        OPT_PARAM_CAPABILITIES,
        8,
        CAP_EXTENDED_NEXT_HOP,
        6,
        0,
        1, // NLRI AFI: IPv4
        0,
        1, // NLRI SAFI: unicast
        0,
        2, // Next hop AFI: IPv6
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v6(s: &str) -> [u8; 16] {
        s.parse::<Ipv6Addr>().unwrap().octets()
    }

    /// Build an UPDATE body with one ORIGIN attribute and one MP_REACH_NLRI.
    fn update_with_mp_reach(afi: u16, next_hop: &[u8], nlri: &[u8], extended: bool) -> Vec<u8> {
        let mut value = Vec::new();
        value.extend_from_slice(&afi.to_be_bytes());
        value.push(1); // SAFI unicast
        value.push(next_hop.len() as u8);
        value.extend_from_slice(next_hop);
        value.push(0); // reserved
        value.extend_from_slice(nlri);

        let mut attrs = vec![0x40, 1, 1, 0]; // ORIGIN IGP
        if extended {
            attrs.extend_from_slice(&[0x80 | ATTR_FLAG_EXTENDED_LENGTH, ATTR_TYPE_MP_REACH_NLRI]);
            attrs.extend_from_slice(&(value.len() as u16).to_be_bytes());
        } else {
            attrs.extend_from_slice(&[0x80, ATTR_TYPE_MP_REACH_NLRI, value.len() as u8]);
        }
        attrs.extend_from_slice(&value);

        let mut body = vec![0, 0]; // no withdrawn routes
        body.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
        body.extend_from_slice(&attrs);
        body
    }

    const NLRI_V4: [u8; 4] = [24, 192, 0, 2];
    const NLRI_V6: [u8; 5] = [32, 0x20, 0x01, 0x0d, 0xb8];

    #[test]
    fn ipv4_route_with_ipv6_global_next_hop() {
        let body = update_with_mp_reach(1, &v6("2001:db8::1"), &NLRI_V4, false);
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
        let body = update_with_mp_reach(1, &v6("fe80::1"), &NLRI_V4, false);
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
        let body = update_with_mp_reach(1, &next_hop, &NLRI_V4, false);
        assert_eq!(
            parse_mp_reach_next_hop(&body),
            Some(MpNextHop {
                address: "2001:db8::1".parse().unwrap(),
                link_local: Some("fe80::1".parse().unwrap()),
            })
        );
    }

    #[test]
    fn ipv4_route_with_ipv4_next_hop() {
        let body = update_with_mp_reach(1, &[192, 0, 2, 1], &NLRI_V4, false);
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
        let body = update_with_mp_reach(2, &next_hop, &NLRI_V6, true);
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
        let body = update_with_mp_reach(2, &next_hop, &NLRI_V6, false);
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
        let body = update_with_mp_reach(2, &next_hop, &NLRI_V6, false);
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
        let mut body = update_with_mp_reach(2, &v6("2001:db8::1"), &NLRI_V6, false);
        // Insert one withdrawn IPv4 route (10.0.0.0/8) before the attributes.
        body.splice(0..2, [0, 2, 8, 10]);
        assert_eq!(
            parse_mp_reach_next_hop(&body).map(|nh| nh.address),
            Some("2001:db8::1".parse().unwrap())
        );
    }

    #[test]
    fn no_mp_reach_attribute() {
        // ORIGIN only, then one IPv4 NLRI.
        let body = [0, 0, 0, 4, 0x40, 1, 1, 0, 24, 192, 0, 2];
        assert_eq!(parse_mp_reach_next_hop(&body), None);
    }

    #[test]
    fn unsupported_next_hop_length() {
        let body = update_with_mp_reach(1, &[0; 12], &NLRI_V4, false);
        assert_eq!(parse_mp_reach_next_hop(&body), None);
    }

    #[test]
    fn truncated_body() {
        let body = update_with_mp_reach(2, &v6("2001:db8::1"), &NLRI_V6, false);
        for len in 0..body.len() {
            assert_eq!(parse_mp_reach_next_hop(&body[..len]), None, "length {len}");
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
