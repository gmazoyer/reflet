use thiserror::Error;

#[derive(Debug, Error)]
pub enum BgpSessionError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("BGP protocol error: {0}")]
    Protocol(String),

    #[error("BGP library error: {0}")]
    Bgp(#[from] zettabgp::prelude::BgpError),

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
