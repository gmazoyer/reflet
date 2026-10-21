use std::io;

use bytes::{Buf, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zettabgp::prelude::*;

use crate::error::{BgpSessionError, MessageHeaderError};
use crate::refresh::MSG_TYPE_ROUTE_REFRESH;

/// BGP message header size (16 marker + 2 length + 1 type).
pub const BGP_HEADER_SIZE: usize = 19;
/// Maximum BGP message size per RFC 4271.
pub const BGP_MAX_MSG_SIZE: usize = 4096;

/// Extended message type that includes Route Refresh (type 5)
/// which zettabgp doesn't handle.
#[derive(Debug, PartialEq)]
pub enum RawMessageType {
    Standard(BgpMessageType),
    RouteRefresh,
}

/// A decoded Route Refresh message body.
#[derive(Debug, PartialEq)]
pub struct RouteRefreshMessage {
    pub afi: u16,
    pub subtype: u8,
    pub safi: u8,
}

/// Reads BGP messages from a stream, handling type 5 (Route Refresh) that
/// zettabgp doesn't support.
///
/// Bytes stay in an internal buffer until they form a complete message, so
/// `read_message` is cancel safe: it can be a branch of `tokio::select!` next
/// to a timer without losing part of a message. Use one reader for the whole
/// session.
#[derive(Debug, Default)]
pub struct MessageReader {
    rbuf: BytesMut,
}

impl MessageReader {
    /// Read one complete BGP message.
    /// Returns the message type and body length (body is in buf[..body_len]).
    pub async fn read_message(
        &mut self,
        stream: &mut TcpStream,
        params: &BgpSessionParams,
        buf: &mut [u8],
    ) -> Result<(RawMessageType, usize), BgpSessionError> {
        loop {
            if let Some(message) = self.take_message(params, buf)? {
                return Ok(message);
            }

            // `read_buf` is the only await point and is cancel safe: bytes
            // already read stay in `rbuf` for the next call.
            self.rbuf.reserve(BGP_MAX_MSG_SIZE);
            if stream.read_buf(&mut self.rbuf).await? == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "early eof").into());
            }
        }
    }

    /// Move the first complete message out of the internal buffer, if there
    /// is one.
    fn take_message(
        &mut self,
        params: &BgpSessionParams,
        buf: &mut [u8],
    ) -> Result<Option<(RawMessageType, usize)>, BgpSessionError> {
        if self.rbuf.len() < BGP_HEADER_SIZE {
            return Ok(None);
        }
        let header = &self.rbuf[..BGP_HEADER_SIZE];

        // Validate marker (first 16 bytes must be 0xFF)
        if header[..16] != [0xFF; 16] {
            return Err(MessageHeaderError::ConnectionNotSynchronized.into());
        }

        // Extract length (bytes 16-17, big-endian)
        let length = u16::from_be_bytes([header[16], header[17]]);
        let msg_len = length as usize;
        if !(BGP_HEADER_SIZE..=BGP_MAX_MSG_SIZE).contains(&msg_len) {
            return Err(MessageHeaderError::BadMessageLength(length).into());
        }

        let body_len = msg_len - BGP_HEADER_SIZE;
        if body_len > buf.len() {
            return Err(MessageHeaderError::MessageTooLarge { length, body_len }.into());
        }

        if self.rbuf.len() < msg_len {
            return Ok(None);
        }

        let msg_type = if header[18] == MSG_TYPE_ROUTE_REFRESH {
            // Route Refresh — handle ourselves
            RawMessageType::RouteRefresh
        } else {
            // Standard message types (1-4) — use zettabgp's decode. After the
            // checks above, it can only fail on the type byte.
            let (msg_type, _) = params.decode_message_head(header).map_err(|source| {
                MessageHeaderError::BadMessageType {
                    msg_type: header[18],
                    source,
                }
            })?;
            RawMessageType::Standard(msg_type)
        };

        buf[..body_len].copy_from_slice(&self.rbuf[BGP_HEADER_SIZE..msg_len]);
        self.rbuf.advance(msg_len);

        Ok(Some((msg_type, body_len)))
    }
}

/// Decode a Route Refresh message body (4 bytes: AFI u16, subtype u8, SAFI u8).
pub fn decode_route_refresh(
    buf: &[u8],
    len: usize,
) -> Result<RouteRefreshMessage, BgpSessionError> {
    if len < 4 {
        return Err(BgpSessionError::Protocol(format!(
            "Route Refresh message too short: {len} bytes"
        )));
    }
    Ok(RouteRefreshMessage {
        afi: u16::from_be_bytes([buf[0], buf[1]]),
        subtype: buf[2],
        safi: buf[3],
    })
}

/// Send a ROUTE_REFRESH message (RFC 2918).
pub async fn send_route_refresh(
    stream: &mut TcpStream,
    afi: u16,
    safi: u8,
    subtype: u8,
    buf: &mut [u8],
) -> Result<(), BgpSessionError> {
    // 16-byte marker (all 0xFF)
    buf[..16].fill(0xFF);
    // 2-byte length: 19 (header) + 4 (body) = 23
    let total_len: u16 = 23;
    buf[16..18].copy_from_slice(&total_len.to_be_bytes());
    // Type byte
    buf[18] = MSG_TYPE_ROUTE_REFRESH;
    // Body: AFI (2) + subtype (1) + SAFI (1)
    buf[19..21].copy_from_slice(&afi.to_be_bytes());
    buf[21] = subtype;
    buf[22] = safi;

    stream.write_all(&buf[..23]).await?;
    Ok(())
}

/// Send a BGP message to the stream.
pub async fn send_message<M: BgpMessage>(
    stream: &mut TcpStream,
    params: &BgpSessionParams,
    msg_type: BgpMessageType,
    msg: &M,
    buf: &mut [u8],
) -> Result<(), BgpSessionError> {
    let msg_len = msg.encode_to(params, &mut buf[BGP_HEADER_SIZE..])?;
    let total_len = params.prepare_message_buf(buf, msg_type, msg_len)?;
    stream.write_all(&buf[..total_len]).await?;
    Ok(())
}

/// Send a KEEPALIVE message.
pub async fn send_keepalive(
    stream: &mut TcpStream,
    params: &BgpSessionParams,
    buf: &mut [u8],
) -> Result<(), BgpSessionError> {
    let ka = BgpKeepaliveMessage {};
    send_message(stream, params, BgpMessageType::Keepalive, &ka, buf).await
}

/// Send an OPEN message.
///
/// `extra_params` holds encoded optional parameters that zettabgp cannot
/// encode, such as the Extended Next Hop capability. They go after the
/// capabilities from `params`.
pub async fn send_open(
    stream: &mut TcpStream,
    params: &BgpSessionParams,
    extra_params: &[u8],
    buf: &mut [u8],
) -> Result<(), BgpSessionError> {
    let total_len = encode_open(params, extra_params, buf)?;
    stream.write_all(&buf[..total_len]).await?;
    Ok(())
}

/// Offset of the Optional Parameters Length field in an OPEN body.
const OPEN_OPT_PARAMS_LEN_OFFSET: usize = 9;
/// Size of an OPEN body before the optional parameters.
const OPEN_FIXED_SIZE: usize = 10;

/// Encode an OPEN message with `extra_params` after the zettabgp
/// capabilities. Returns the length of the full message in `buf`.
fn encode_open(
    params: &BgpSessionParams,
    extra_params: &[u8],
    buf: &mut [u8],
) -> Result<usize, BgpSessionError> {
    let open = params.open_message();
    let body = &mut buf[BGP_HEADER_SIZE..];
    let mut body_len = open.encode_to(params, body)?;

    // zettabgp writes the length with a cast to u8, which wraps above 255.
    // Compute it again from the encoded size.
    let opt_params_len = body_len - OPEN_FIXED_SIZE + extra_params.len();
    let opt_params_len = u8::try_from(opt_params_len).map_err(|_| {
        BgpSessionError::Protocol(format!(
            "OPEN optional parameters too long: {opt_params_len} bytes"
        ))
    })?;
    body[OPEN_OPT_PARAMS_LEN_OFFSET] = opt_params_len;
    body[body_len..body_len + extra_params.len()].copy_from_slice(extra_params);
    body_len += extra_params.len();

    Ok(params.prepare_message_buf(buf, BgpMessageType::Open, body_len)?)
}

/// Send a NOTIFICATION message. zettabgp always encodes `data` as two octets.
pub async fn send_notification(
    stream: &mut TcpStream,
    params: &BgpSessionParams,
    error_code: u8,
    error_subcode: u8,
    data: u16,
    buf: &mut [u8],
) -> Result<(), BgpSessionError> {
    let mut notif = BgpNotificationMessage::new();
    notif.error_code = error_code;
    notif.error_subcode = error_subcode;
    notif.data = data;
    send_message(stream, params, BgpMessageType::Notification, &notif, buf).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refresh::{
        AFI_IPV4, AFI_IPV6, ROUTE_REFRESH_BORR, ROUTE_REFRESH_EORR, ROUTE_REFRESH_NORMAL,
        SAFI_UNICAST,
    };

    #[test]
    fn encode_route_refresh_ipv4_normal() {
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];

        // We can't easily test async send, but we can test the buffer layout
        // manually replicate what send_route_refresh does
        buf[..16].fill(0xFF);
        let total_len: u16 = 23;
        buf[16..18].copy_from_slice(&total_len.to_be_bytes());
        buf[18] = MSG_TYPE_ROUTE_REFRESH;
        buf[19..21].copy_from_slice(&AFI_IPV4.to_be_bytes());
        buf[21] = ROUTE_REFRESH_NORMAL;
        buf[22] = SAFI_UNICAST;

        // Verify marker
        assert_eq!(&buf[..16], &[0xFF; 16]);
        // Verify length
        assert_eq!(u16::from_be_bytes([buf[16], buf[17]]), 23);
        // Verify type
        assert_eq!(buf[18], 5);
        // Verify body
        assert_eq!(u16::from_be_bytes([buf[19], buf[20]]), AFI_IPV4);
        assert_eq!(buf[21], ROUTE_REFRESH_NORMAL);
        assert_eq!(buf[22], SAFI_UNICAST);
    }

    #[test]
    fn encode_route_refresh_ipv6_borr() {
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];
        buf[..16].fill(0xFF);
        buf[16..18].copy_from_slice(&23u16.to_be_bytes());
        buf[18] = MSG_TYPE_ROUTE_REFRESH;
        buf[19..21].copy_from_slice(&AFI_IPV6.to_be_bytes());
        buf[21] = ROUTE_REFRESH_BORR;
        buf[22] = SAFI_UNICAST;

        assert_eq!(u16::from_be_bytes([buf[19], buf[20]]), AFI_IPV6);
        assert_eq!(buf[21], ROUTE_REFRESH_BORR);
    }

    fn open_params() -> BgpSessionParams {
        BgpSessionParams::new(
            65000,
            90,
            BgpTransportMode::IPv6,
            std::net::Ipv4Addr::new(192, 0, 2, 1),
            vec![
                BgpCapability::SafiIPv4u,
                BgpCapability::SafiIPv6u,
                BgpCapability::CapRR,
                BgpCapability::CapASN32(65000),
            ],
        )
    }

    #[test]
    fn encode_open_without_extra_params() {
        let params = open_params();
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];
        let total_len = encode_open(&params, &[], &mut buf).unwrap();

        let mut open = BgpOpenMessage::new();
        open.decode_from(&params, &buf[BGP_HEADER_SIZE..total_len])
            .unwrap();
        assert_eq!(open.caps, params.caps);
    }

    #[test]
    fn encode_open_appends_extra_params() {
        let params = open_params();
        let mut plain = [0u8; BGP_MAX_MSG_SIZE];
        let plain_len = encode_open(&params, &[], &mut plain).unwrap();

        let extra = crate::nexthop::extended_next_hop_param();
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];
        let total_len = encode_open(&params, &extra, &mut buf).unwrap();

        assert_eq!(total_len, plain_len + extra.len());
        assert_eq!(u16::from_be_bytes([buf[16], buf[17]]) as usize, total_len);
        let opt_params_len = BGP_HEADER_SIZE + OPEN_OPT_PARAMS_LEN_OFFSET;
        assert_eq!(
            buf[opt_params_len] as usize,
            plain[opt_params_len] as usize + extra.len()
        );
        assert_eq!(&buf[plain_len..total_len], &extra);

        // zettabgp skips the unknown capability and still decodes the others.
        let mut open = BgpOpenMessage::new();
        open.decode_from(&params, &buf[BGP_HEADER_SIZE..total_len])
            .unwrap();
        assert_eq!(open.caps, params.caps);
    }

    #[test]
    fn encode_open_rejects_too_long_optional_params() {
        let params = open_params();
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];
        let extra = [0u8; 255];
        assert!(matches!(
            encode_open(&params, &extra, &mut buf),
            Err(BgpSessionError::Protocol(_))
        ));
    }

    /// Session parameters with `count` Route Refresh capabilities. zettabgp
    /// puts each one in its own optional parameter of 4 bytes.
    fn params_with_route_refresh_caps(count: usize) -> BgpSessionParams {
        BgpSessionParams::new(
            65000,
            90,
            BgpTransportMode::IPv4,
            std::net::Ipv4Addr::new(192, 0, 2, 1),
            vec![BgpCapability::CapRR; count],
        )
    }

    /// zettabgp writes the Optional Parameters Length with a cast to u8:
    /// for 260 bytes of capabilities, it writes 4.
    #[test]
    fn encode_open_rejects_capabilities_longer_than_255_bytes() {
        let params = params_with_route_refresh_caps(65);
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];
        let extra = crate::nexthop::extended_next_hop_param();
        for extra in [&extra[..], &[]] {
            assert!(
                matches!(
                    encode_open(&params, extra, &mut buf),
                    Err(BgpSessionError::Protocol(_))
                ),
                "{} bytes of extra parameters",
                extra.len()
            );
        }
    }

    #[test]
    fn encode_open_rejects_capabilities_longer_than_255_bytes_with_extra_params() {
        // 248 bytes of capabilities and 10 bytes of extra parameters.
        let params = params_with_route_refresh_caps(62);
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];
        let extra = crate::nexthop::extended_next_hop_param();
        assert!(matches!(
            encode_open(&params, &extra, &mut buf),
            Err(BgpSessionError::Protocol(_))
        ));
    }

    #[test]
    fn encode_open_accepts_optional_params_up_to_255_bytes() {
        // 244 bytes of capabilities and 10 bytes of extra parameters.
        let params = params_with_route_refresh_caps(61);
        let mut buf = [0u8; BGP_MAX_MSG_SIZE];
        let extra = crate::nexthop::extended_next_hop_param();
        let total_len = encode_open(&params, &extra, &mut buf).unwrap();
        assert_eq!(buf[BGP_HEADER_SIZE + OPEN_OPT_PARAMS_LEN_OFFSET], 254);
        assert_eq!(total_len, BGP_HEADER_SIZE + 10 + 254);
    }

    #[test]
    fn decode_route_refresh_valid() {
        let buf = [0x00, 0x01, 0x00, 0x01]; // AFI=1 (IPv4), subtype=0, SAFI=1
        let msg = decode_route_refresh(&buf, 4).unwrap();
        assert_eq!(msg.afi, AFI_IPV4);
        assert_eq!(msg.subtype, ROUTE_REFRESH_NORMAL);
        assert_eq!(msg.safi, SAFI_UNICAST);
    }

    #[test]
    fn decode_route_refresh_eorr() {
        let buf = [0x00, 0x02, 0x02, 0x01]; // AFI=2 (IPv6), subtype=2 (EoRR), SAFI=1
        let msg = decode_route_refresh(&buf, 4).unwrap();
        assert_eq!(msg.afi, AFI_IPV6);
        assert_eq!(msg.subtype, ROUTE_REFRESH_EORR);
        assert_eq!(msg.safi, SAFI_UNICAST);
    }

    #[test]
    fn decode_route_refresh_too_short() {
        let buf = [0x00, 0x01, 0x00];
        assert!(decode_route_refresh(&buf, 3).is_err());
    }

    fn header(length: u16, msg_type: u8) -> Vec<u8> {
        let mut msg = vec![0xFF; 16];
        msg.extend_from_slice(&length.to_be_bytes());
        msg.push(msg_type);
        msg
    }

    #[test]
    fn header_errors_map_to_notification_code_and_subcode() {
        let params = BgpSessionParams::new(
            65000,
            90,
            BgpTransportMode::IPv4,
            std::net::Ipv4Addr::new(192, 0, 2, 1),
            vec![],
        );
        let mut bad_marker = header(19, 4);
        bad_marker[3] = 0;

        // (received bytes, read buffer size, subcode, data, log text)
        let cases = [
            (
                bad_marker,
                BGP_MAX_MSG_SIZE,
                1,
                0,
                "BGP protocol error: invalid BGP marker",
            ),
            (
                header(18, 4),
                BGP_MAX_MSG_SIZE,
                2,
                18,
                "BGP protocol error: invalid message length: 18",
            ),
            (
                header(4097, 2),
                BGP_MAX_MSG_SIZE,
                2,
                4097,
                "BGP protocol error: invalid message length: 4097",
            ),
            (
                header(40, 2),
                10,
                2,
                40,
                "BGP protocol error: message too large: 21 bytes",
            ),
            (
                header(19, 7),
                BGP_MAX_MSG_SIZE,
                3,
                0x0700,
                "BGP library error: BgpError Invalid message type",
            ),
        ];

        for (bytes, buf_len, subcode, data, text) in cases {
            let mut reader = MessageReader {
                rbuf: BytesMut::from(&bytes[..]),
            };
            let mut buf = vec![0u8; buf_len];
            let err = reader.take_message(&params, &mut buf).unwrap_err();
            assert_eq!(err.to_string(), text);
            let header_err = match err {
                BgpSessionError::MessageHeader(e) => e,
                other => panic!("expected a message header error, got {other}"),
            };
            assert_eq!(
                (
                    MessageHeaderError::CODE,
                    header_err.subcode(),
                    header_err.data()
                ),
                (1, subcode, data),
                "{text}"
            );
        }
    }
}
