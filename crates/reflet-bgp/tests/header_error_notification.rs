use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;
use zettabgp::prelude::*;

use reflet_bgp::codec::{self, BGP_MAX_MSG_SIZE, MessageReader, RawMessageType};
use reflet_bgp::error::BgpSessionError;
use reflet_bgp::session::{BgpSession, LocalParams};
use reflet_core::config::PeerConfig;
use reflet_core::event_log::EventLog;
use reflet_core::peer::{PeerInfo, PeerState};
use reflet_core::prefix::AddressFamily;
use reflet_core::rib::RibStore;

const STEP_TIMEOUT: Duration = Duration::from_secs(5);

fn params(asn: u32, router_id: Ipv4Addr) -> BgpSessionParams {
    BgpSessionParams::new(asn, 90, BgpTransportMode::IPv4, router_id, vec![])
}

/// A peer that sends a message with a bad marker in an established session
/// receives a NOTIFICATION with code 1 (Message Header Error) and subcode 1
/// (Connection Not Synchronized) before the connection closes.
#[tokio::test]
async fn bad_marker_gets_notification_before_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut peer = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (stream, _) = listener.accept().await.unwrap();

    let peer_config = PeerConfig {
        address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        remote_asn: 65001,
        name: String::new(),
        description: String::new(),
        location: None,
        families: vec![AddressFamily::Ipv4Unicast],
        extended_next_hop: false,
        snapshot_interval: None,
    };
    let peer_info = Arc::new(RwLock::new(PeerInfo::new(
        peer_config.address,
        peer_config.remote_asn,
        String::new(),
        String::new(),
        None,
        peer_config.families.clone(),
    )));
    // Keep both senders open: a closed channel makes its select! branch
    // complete at once, again and again.
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let (_command_tx, command_rx) = mpsc::channel(4);
    let mut session = BgpSession::new(
        peer_config,
        LocalParams {
            session: params(65000, Ipv4Addr::new(192, 0, 2, 1)),
            open_extra_params: Vec::new(),
        },
        RibStore::new(),
        peer_info.clone(),
        shutdown_rx,
        command_rx,
        false,
        false,
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
    );
    let session = tokio::spawn(async move { session.run_with_stream(stream).await });

    let peer_params = params(65001, Ipv4Addr::new(192, 0, 2, 2));
    let mut reader = MessageReader::default();
    let mut buf = vec![0u8; BGP_MAX_MSG_SIZE];

    // Establish the session.
    let (msg_type, _) = timeout(
        STEP_TIMEOUT,
        reader.read_message(&mut peer, &peer_params, &mut buf),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(msg_type, RawMessageType::Standard(BgpMessageType::Open));
    codec::send_open(&mut peer, &peer_params, &[], &mut buf)
        .await
        .unwrap();
    let (msg_type, _) = timeout(
        STEP_TIMEOUT,
        reader.read_message(&mut peer, &peer_params, &mut buf),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        msg_type,
        RawMessageType::Standard(BgpMessageType::Keepalive)
    );
    codec::send_keepalive(&mut peer, &peer_params, &mut buf)
        .await
        .unwrap();

    // Send a KEEPALIVE with a bad marker.
    let mut bad_keepalive = vec![0xFF; 16];
    bad_keepalive.extend_from_slice(&19u16.to_be_bytes());
    bad_keepalive.push(4);
    bad_keepalive[3] = 0;
    peer.write_all(&bad_keepalive).await.unwrap();

    let (msg_type, body_len) = timeout(
        STEP_TIMEOUT,
        reader.read_message(&mut peer, &peer_params, &mut buf),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        msg_type,
        RawMessageType::Standard(BgpMessageType::Notification)
    );
    let mut notification = BgpNotificationMessage::new();
    notification
        .decode_from(&peer_params, &buf[..body_len])
        .unwrap();
    assert_eq!(
        (notification.error_code, notification.error_subcode),
        (1, 1)
    );

    // The NOTIFICATION is the last message: then the connection closes.
    let err = timeout(
        STEP_TIMEOUT,
        reader.read_message(&mut peer, &peer_params, &mut buf),
    )
    .await
    .unwrap()
    .unwrap_err();
    match err {
        BgpSessionError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
        other => panic!("expected the connection to close, got {other}"),
    }

    let err = timeout(STEP_TIMEOUT, session)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.to_string(), "BGP protocol error: invalid BGP marker");
    // The error came from the main loop, not from the OPEN or KEEPALIVE read.
    assert_eq!(peer_info.read().unwrap().state, PeerState::Established);
}
