use thiserror::Error;
use zettabgp::prelude::BgpError;

#[derive(Debug, Error)]
pub enum BgpSessionError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("BGP protocol error: {0}")]
    Protocol(String),

    /// A bad message header. The session reports it to the peer with a
    /// NOTIFICATION before it closes.
    #[error(transparent)]
    MessageHeader(#[from] MessageHeaderError),

    #[error("BGP library error: {0}")]
    Bgp(#[from] BgpError),

    #[error("peer sent NOTIFICATION: code={code}, subcode={subcode}, message={message}")]
    Notification {
        code: u8,
        subcode: u8,
        message: String,
    },

    #[error("hold timer expired")]
    HoldTimerExpired,

    #[error("session shutdown requested")]
    Shutdown,
}

/// A Message Header Error (RFC 4271 section 6.1).
///
/// The Display texts are the ones that the logs showed before these errors
/// had their own type, so keep them unchanged.
#[derive(Debug, Error)]
pub enum MessageHeaderError {
    /// The marker is not all ones.
    #[error("BGP protocol error: invalid BGP marker")]
    ConnectionNotSynchronized,

    /// The length field is outside 19 to 4096.
    #[error("BGP protocol error: invalid message length: {0}")]
    BadMessageLength(u16),

    /// The body does not fit in the read buffer.
    #[error("BGP protocol error: message too large: {body_len} bytes")]
    MessageTooLarge { length: u16, body_len: usize },

    /// The type field is not a known message type.
    #[error("BGP library error: {source}")]
    BadMessageType { msg_type: u8, source: BgpError },
}

impl MessageHeaderError {
    /// NOTIFICATION error code for all header errors.
    pub const CODE: u8 = 1;

    /// NOTIFICATION error subcode.
    pub fn subcode(&self) -> u8 {
        match self {
            Self::ConnectionNotSynchronized => 1,
            Self::BadMessageLength(_) | Self::MessageTooLarge { .. } => 2,
            Self::BadMessageType { .. } => 3,
        }
    }

    /// NOTIFICATION data: the bad length field for subcode 2 and the bad type
    /// field for subcode 3. zettabgp always encodes two octets of data, so the
    /// one-octet type goes in the first octet and a zero octet follows it.
    pub fn data(&self) -> u16 {
        match self {
            Self::ConnectionNotSynchronized => 0,
            Self::BadMessageLength(length) | Self::MessageTooLarge { length, .. } => *length,
            Self::BadMessageType { msg_type, .. } => u16::from(*msg_type) << 8,
        }
    }
}
