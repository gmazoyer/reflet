use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::interval;
use zettabgp::prelude::*;

use reflet_bgp::codec::{BGP_MAX_MSG_SIZE, MessageReader, RawMessageType};
use reflet_bgp::error::BgpSessionError;

fn params() -> BgpSessionParams {
    BgpSessionParams::new(
        65000,
        90,
        BgpTransportMode::IPv4,
        Ipv4Addr::new(192, 0, 2, 1),
        vec![],
    )
}

fn keepalive() -> Vec<u8> {
    let mut msg = vec![0xFF; 16];
    msg.extend_from_slice(&19u16.to_be_bytes());
    msg.push(4);
    msg
}

fn route_refresh_ipv6() -> Vec<u8> {
    let mut msg = vec![0xFF; 16];
    msg.extend_from_slice(&23u16.to_be_bytes());
    msg.extend_from_slice(&[5, 0x00, 0x02, 0x00, 0x01]);
    msg
}

/// Return both ends of a TCP connection: (local side, peer side).
async fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    peer.set_nodelay(true).unwrap();
    let (local, _) = listener.accept().await.unwrap();
    (local, peer)
}

/// A timer that fires while a message is only partly received must not make
/// the reader lose the bytes it already has. This is the structure of the
/// main loop in `BgpSession::run_with_stream`.
#[tokio::test]
async fn timer_during_partial_message_keeps_stream_in_sync() {
    let (mut stream, mut peer) = connected_pair().await;
    let writer = tokio::spawn(async move {
        let ka = keepalive();
        peer.write_all(&ka[..10]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        peer.write_all(&ka[10..]).await.unwrap();
        peer.write_all(&ka).await.unwrap();
        peer.write_all(&ka).await.unwrap();
        peer
    });

    let params = params();
    let mut reader = MessageReader::default();
    let mut buf = vec![0u8; BGP_MAX_MSG_SIZE];
    let mut timer = interval(Duration::from_millis(50));
    timer.tick().await;
    let mut ticks = 0;
    let mut messages = 0;

    while messages < 3 {
        tokio::select! {
            _ = timer.tick() => {
                ticks += 1;
                assert!(ticks < 40, "messages not received in time");
            }
            result = reader.read_message(&mut stream, &params, &mut buf) => {
                let (msg_type, body_len) = result.unwrap();
                assert_eq!(msg_type, RawMessageType::Standard(BgpMessageType::Keepalive));
                assert_eq!(body_len, 0);
                messages += 1;
            }
        }
    }

    assert!(ticks > 0, "the timer must fire during the partial message");
    writer.await.unwrap();
}

#[tokio::test]
async fn reads_several_messages_from_one_segment() {
    let (mut stream, mut peer) = connected_pair().await;
    let mut segment = keepalive();
    segment.extend(route_refresh_ipv6());
    segment.extend(keepalive());
    peer.write_all(&segment).await.unwrap();

    let params = params();
    let mut reader = MessageReader::default();
    let mut buf = vec![0u8; BGP_MAX_MSG_SIZE];

    let (msg_type, _) = reader
        .read_message(&mut stream, &params, &mut buf)
        .await
        .unwrap();
    assert_eq!(
        msg_type,
        RawMessageType::Standard(BgpMessageType::Keepalive)
    );

    let (msg_type, body_len) = reader
        .read_message(&mut stream, &params, &mut buf)
        .await
        .unwrap();
    assert_eq!(msg_type, RawMessageType::RouteRefresh);
    assert_eq!(&buf[..body_len], &[0x00, 0x02, 0x00, 0x01]);

    let (msg_type, _) = reader
        .read_message(&mut stream, &params, &mut buf)
        .await
        .unwrap();
    assert_eq!(
        msg_type,
        RawMessageType::Standard(BgpMessageType::Keepalive)
    );
}

#[tokio::test]
async fn rejects_invalid_marker() {
    let (mut stream, mut peer) = connected_pair().await;
    let mut msg = keepalive();
    msg[3] = 0;
    peer.write_all(&msg).await.unwrap();

    let mut reader = MessageReader::default();
    let mut buf = vec![0u8; BGP_MAX_MSG_SIZE];
    let err = reader
        .read_message(&mut stream, &params(), &mut buf)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("invalid BGP marker"), "{err}");
}

#[tokio::test]
async fn peer_close_during_message_is_early_eof() {
    let (mut stream, mut peer) = connected_pair().await;
    peer.write_all(&keepalive()[..10]).await.unwrap();
    drop(peer);

    let mut reader = MessageReader::default();
    let mut buf = vec![0u8; BGP_MAX_MSG_SIZE];
    let err = reader
        .read_message(&mut stream, &params(), &mut buf)
        .await
        .unwrap_err();
    match err {
        BgpSessionError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof),
        other => panic!("expected an I/O error, got {other}"),
    }
}
