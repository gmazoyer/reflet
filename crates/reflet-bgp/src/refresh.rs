use tokio::sync::mpsc;

/// Commands that can be sent to an active BGP session.
pub enum SessionCommand {
    /// Request route refresh for all negotiated address families.
    RouteRefresh,
}

/// Sender half for session commands.
pub type CommandTx = mpsc::Sender<SessionCommand>;
/// Receiver half for session commands.
pub type CommandRx = mpsc::Receiver<SessionCommand>;

/// Per-AFI/SAFI Route Refresh state for Enhanced Route Refresh (RFC 7313).
#[derive(Debug, Default)]
pub struct RouteRefreshState {
    /// Whether route refresh was negotiated with this peer.
    pub supported: bool,
    /// Whether enhanced route refresh was negotiated with this peer.
    pub enhanced: bool,
    /// Whether we are currently in an IPv4 refresh cycle (between sending BoRR and receiving EoRR).
    pub ipv4_refreshing: bool,
    /// Whether we are currently in an IPv6 refresh cycle (between sending BoRR and receiving EoRR).
    pub ipv6_refreshing: bool,
}

/// Route Refresh message type byte (RFC 2918).
pub const MSG_TYPE_ROUTE_REFRESH: u8 = 5;

/// Route Refresh subtypes.
pub const ROUTE_REFRESH_NORMAL: u8 = 0;
/// BoRR — Beginning of Route Refresh (RFC 7313).
pub const ROUTE_REFRESH_BORR: u8 = 1;
/// EoRR — End of Route Refresh (RFC 7313).
pub const ROUTE_REFRESH_EORR: u8 = 2;

/// AFI values.
pub const AFI_IPV4: u16 = 1;
pub const AFI_IPV6: u16 = 2;

/// SAFI Unicast.
pub const SAFI_UNICAST: u8 = 1;
