use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::prefix::Prefix;
use crate::route::AsPathSegment;

/// The type of route event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RouteEventType {
    Announce,
    Withdraw,
    SessionUp,
    SessionDown,
}

impl RouteEventType {
    /// Parse from a string (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "announce" => Some(Self::Announce),
            "withdraw" => Some(Self::Withdraw),
            "session_up" => Some(Self::SessionUp),
            "session_down" => Some(Self::SessionDown),
            _ => None,
        }
    }
}

/// A route change event.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RouteEvent {
    pub seq: u64,
    pub timestamp: DateTime<Utc>,
    pub peer_id: String,
    #[serde(rename = "type")]
    pub event_type: RouteEventType,
    // Route fields (present for Announce/Withdraw)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<Prefix>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_path: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub next_hop: Option<IpAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_as: Option<u32>,
    // Session fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_asn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Flatten AS path segments into a simple list of ASNs.
pub fn flatten_as_path(segments: &[AsPathSegment]) -> Vec<u32> {
    segments
        .iter()
        .flat_map(|seg| match seg {
            AsPathSegment::Sequence(asns) | AsPathSegment::Set(asns) => asns.iter().copied(),
        })
        .collect()
}

struct EventLogInner {
    buffer: VecDeque<RouteEvent>,
    max_events: usize,
    next_seq: u64,
    file_writer: Option<BufWriter<std::fs::File>>,
}

/// A bounded ring buffer of route events with optional JSONL file output.
#[derive(Clone)]
pub struct EventLog {
    inner: Arc<Mutex<EventLogInner>>,
    notifier: Arc<dyn Fn() + Send + Sync>,
}

impl EventLog {
    /// Create a new event log with the given capacity and optional file path.
    pub fn new(max_events: usize, file_path: Option<&str>) -> io::Result<Self> {
        let file_writer = match file_path {
            Some(path) => {
                let file = OpenOptions::new().create(true).append(true).open(path)?;
                Some(BufWriter::new(file))
            }
            None => None,
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(EventLogInner {
                buffer: VecDeque::with_capacity(max_events.min(10_000)),
                max_events,
                next_seq: 1,
                file_writer,
            })),
            notifier: Arc::new(|| {}),
        })
    }

    /// Create a disabled (no-op) event log.
    pub fn disabled() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventLogInner {
                buffer: VecDeque::new(),
                max_events: 0,
                next_seq: 1,
                file_writer: None,
            })),
            notifier: Arc::new(|| {}),
        }
    }

    /// Set a notifier callback that fires after each event is pushed.
    pub fn with_notifier(mut self, f: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.notifier = f;
        self
    }

    /// Record a route announcement event.
    pub fn push_announce(
        &self,
        peer_id: String,
        prefix: Prefix,
        path_id: Option<u32>,
        as_path: Vec<u32>,
        next_hop: IpAddr,
        origin_as: Option<u32>,
    ) {
        self.push(RouteEvent {
            seq: 0, // assigned in push()
            timestamp: Utc::now(),
            peer_id,
            event_type: RouteEventType::Announce,
            prefix: Some(prefix),
            path_id,
            as_path: Some(as_path),
            next_hop: Some(next_hop),
            origin_as,
            remote_asn: None,
            reason: None,
        });
    }

    /// Record a route withdrawal event.
    pub fn push_withdraw(&self, peer_id: String, prefix: Prefix, path_id: Option<u32>) {
        self.push(RouteEvent {
            seq: 0,
            timestamp: Utc::now(),
            peer_id,
            event_type: RouteEventType::Withdraw,
            prefix: Some(prefix),
            path_id,
            as_path: None,
            next_hop: None,
            origin_as: None,
            remote_asn: None,
            reason: None,
        });
    }

    /// Record a session up event.
    pub fn push_session_up(&self, peer_id: String, remote_asn: u32) {
        self.push(RouteEvent {
            seq: 0,
            timestamp: Utc::now(),
            peer_id,
            event_type: RouteEventType::SessionUp,
            prefix: None,
            path_id: None,
            as_path: None,
            next_hop: None,
            origin_as: None,
            remote_asn: Some(remote_asn),
            reason: None,
        });
    }

    /// Record a session down event.
    pub fn push_session_down(&self, peer_id: String, reason: String) {
        self.push(RouteEvent {
            seq: 0,
            timestamp: Utc::now(),
            peer_id,
            event_type: RouteEventType::SessionDown,
            prefix: None,
            path_id: None,
            as_path: None,
            next_hop: None,
            origin_as: None,
            remote_asn: None,
            reason: Some(reason),
        });
    }

    fn push(&self, mut event: RouteEvent) {
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.max_events == 0 {
                return;
            }

            event.seq = inner.next_seq;
            inner.next_seq += 1;

            // Write to file if configured
            if let Some(ref mut writer) = inner.file_writer
                && let Ok(line) = serde_json::to_string(&event)
            {
                let _ = writeln!(writer, "{}", line);
            }

            // Push to ring buffer
            if inner.buffer.len() >= inner.max_events {
                inner.buffer.pop_front();
            }
            inner.buffer.push_back(event);
        }
        // Notify after releasing the lock
        (self.notifier)();
    }

    /// Query events with optional filters.
    /// Returns events matching all provided filters, up to `limit`.
    pub fn query(
        &self,
        since_seq: Option<u64>,
        peer_id: Option<&str>,
        event_type: Option<RouteEventType>,
        limit: usize,
    ) -> Vec<RouteEvent> {
        let inner = self.inner.lock().unwrap();
        inner
            .buffer
            .iter()
            .filter(|e| since_seq.is_none_or(|s| e.seq > s))
            .filter(|e| peer_id.is_none_or(|p| e.peer_id == p))
            .filter(|e| event_type.is_none_or(|t| e.event_type == t))
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    /// Get the current (latest) sequence number.
    pub fn current_seq(&self) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner.next_seq.saturating_sub(1)
    }

    /// Flush the file writer (call on shutdown).
    pub fn flush(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(ref mut writer) = inner.file_writer {
            let _ = writer.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_announce(peer_id: &str, prefix_str: &str) -> (String, Prefix) {
        let prefix: Prefix = prefix_str.parse().unwrap();
        (peer_id.to_string(), prefix)
    }

    #[test]
    fn ring_buffer_basic() {
        let log = EventLog::new(100, None).unwrap();
        let (peer, prefix) = make_announce("10.0.0.1", "192.168.0.0/24");
        log.push_announce(
            peer,
            prefix,
            None,
            vec![65001],
            "10.0.0.1".parse().unwrap(),
            Some(65001),
        );

        let events = log.query(None, None, None, 100);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[0].event_type, RouteEventType::Announce);
    }

    #[test]
    fn ring_buffer_wrapping() {
        let log = EventLog::new(5, None).unwrap();
        for i in 0..10 {
            let prefix_str = format!("10.0.{}.0/24", i);
            let (peer, prefix) = make_announce("10.0.0.1", &prefix_str);
            log.push_announce(
                peer,
                prefix,
                None,
                vec![65001],
                "10.0.0.1".parse().unwrap(),
                None,
            );
        }

        let events = log.query(None, None, None, 100);
        assert_eq!(events.len(), 5);
        // Should have events 6..10 (most recent)
        assert_eq!(events[0].seq, 6);
        assert_eq!(events[4].seq, 10);
    }

    #[test]
    fn sequence_numbers() {
        let log = EventLog::new(3, None).unwrap();
        for i in 0..5 {
            let prefix_str = format!("10.0.{}.0/24", i);
            let (peer, prefix) = make_announce("10.0.0.1", &prefix_str);
            log.push_announce(
                peer,
                prefix,
                None,
                vec![],
                "10.0.0.1".parse().unwrap(),
                None,
            );
        }

        let events = log.query(None, None, None, 100);
        // Buffer holds 3 events: seq 3, 4, 5
        assert_eq!(events.len(), 3);
        for (i, event) in events.iter().enumerate() {
            assert_eq!(event.seq, (i + 3) as u64);
        }
        assert_eq!(log.current_seq(), 5);
    }

    #[test]
    fn filter_by_peer() {
        let log = EventLog::new(100, None).unwrap();
        for peer_ip in &["10.0.0.1", "10.0.0.2", "10.0.0.1"] {
            let (peer, prefix) = make_announce(peer_ip, "192.168.0.0/24");
            log.push_announce(
                peer,
                prefix,
                None,
                vec![],
                "10.0.0.1".parse().unwrap(),
                None,
            );
        }

        let events = log.query(None, Some("10.0.0.1"), None, 100);
        assert_eq!(events.len(), 2);
        for e in &events {
            assert_eq!(e.peer_id, "10.0.0.1");
        }
    }

    #[test]
    fn filter_by_type() {
        let log = EventLog::new(100, None).unwrap();
        let (peer, prefix) = make_announce("10.0.0.1", "192.168.0.0/24");
        log.push_announce(
            peer.clone(),
            prefix.clone(),
            None,
            vec![],
            "10.0.0.1".parse().unwrap(),
            None,
        );
        log.push_withdraw(peer.clone(), prefix, None);
        log.push_session_up(peer.clone(), 65001);
        log.push_session_down(peer, "hold timer expired".into());

        let announces = log.query(None, None, Some(RouteEventType::Announce), 100);
        assert_eq!(announces.len(), 1);

        let withdraws = log.query(None, None, Some(RouteEventType::Withdraw), 100);
        assert_eq!(withdraws.len(), 1);

        let ups = log.query(None, None, Some(RouteEventType::SessionUp), 100);
        assert_eq!(ups.len(), 1);

        let downs = log.query(None, None, Some(RouteEventType::SessionDown), 100);
        assert_eq!(downs.len(), 1);
        assert_eq!(downs[0].reason.as_deref(), Some("hold timer expired"));
    }

    #[test]
    fn filter_since_seq() {
        let log = EventLog::new(100, None).unwrap();
        for i in 0..5 {
            let prefix_str = format!("10.0.{}.0/24", i);
            let (peer, prefix) = make_announce("10.0.0.1", &prefix_str);
            log.push_announce(
                peer,
                prefix,
                None,
                vec![],
                "10.0.0.1".parse().unwrap(),
                None,
            );
        }

        let events = log.query(Some(3), None, None, 100);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 4);
        assert_eq!(events[1].seq, 5);
    }

    #[test]
    fn filter_combined() {
        let log = EventLog::new(100, None).unwrap();
        // Peer A: 2 announces
        let (peer, prefix) = make_announce("10.0.0.1", "192.168.0.0/24");
        log.push_announce(
            peer,
            prefix,
            None,
            vec![],
            "10.0.0.1".parse().unwrap(),
            None,
        );
        let (peer, prefix) = make_announce("10.0.0.1", "192.168.1.0/24");
        log.push_announce(
            peer,
            prefix,
            None,
            vec![],
            "10.0.0.1".parse().unwrap(),
            None,
        );
        // Peer B: 1 announce
        let (peer, prefix) = make_announce("10.0.0.2", "10.0.0.0/24");
        log.push_announce(
            peer,
            prefix,
            None,
            vec![],
            "10.0.0.2".parse().unwrap(),
            None,
        );
        // Peer A: 1 withdraw
        let (peer, prefix) = make_announce("10.0.0.1", "192.168.0.0/24");
        log.push_withdraw(peer, prefix, None);

        // Peer A + announce + since_seq=1
        let events = log.query(
            Some(1),
            Some("10.0.0.1"),
            Some(RouteEventType::Announce),
            100,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, 2);
    }

    #[test]
    fn limit_returns_most_recent() {
        let log = EventLog::new(100, None).unwrap();
        for i in 0..10 {
            let prefix_str = format!("10.0.{}.0/24", i);
            let (peer, prefix) = make_announce("10.0.0.1", &prefix_str);
            log.push_announce(
                peer,
                prefix,
                None,
                vec![],
                "10.0.0.1".parse().unwrap(),
                None,
            );
        }

        let events = log.query(None, None, None, 3);
        assert_eq!(events.len(), 3);
        // Should be the last 3
        assert_eq!(events[0].seq, 8);
        assert_eq!(events[1].seq, 9);
        assert_eq!(events[2].seq, 10);
    }

    #[test]
    fn disabled_mode() {
        let log = EventLog::disabled();
        let (peer, prefix) = make_announce("10.0.0.1", "192.168.0.0/24");
        log.push_announce(
            peer,
            prefix,
            None,
            vec![],
            "10.0.0.1".parse().unwrap(),
            None,
        );

        let events = log.query(None, None, None, 100);
        assert!(events.is_empty());
        assert_eq!(log.current_seq(), 0);
    }

    #[test]
    fn file_output() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("events.jsonl");
        let log = EventLog::new(100, Some(file_path.to_str().unwrap())).unwrap();

        let (peer, prefix) = make_announce("10.0.0.1", "192.168.0.0/24");
        log.push_announce(
            peer.clone(),
            prefix,
            None,
            vec![65001, 65010],
            "10.0.0.1".parse().unwrap(),
            Some(65010),
        );
        log.push_session_up(peer, 65001);
        log.flush();

        let content = std::fs::read_to_string(&file_path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        let event: RouteEvent = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(event.event_type, RouteEventType::Announce);
        assert_eq!(event.as_path, Some(vec![65001, 65010]));

        let event: RouteEvent = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(event.event_type, RouteEventType::SessionUp);
        assert_eq!(event.remote_asn, Some(65001));
    }

    #[test]
    fn notifier_fires_on_push() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let count = Arc::new(AtomicU32::new(0));
        let count_ref = count.clone();
        let log = EventLog::new(100, None)
            .unwrap()
            .with_notifier(Arc::new(move || {
                count_ref.fetch_add(1, Ordering::Relaxed);
            }));

        let (peer, prefix) = make_announce("10.0.0.1", "192.168.0.0/24");
        log.push_announce(
            peer,
            prefix,
            None,
            vec![65001],
            "10.0.0.1".parse().unwrap(),
            Some(65001),
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);

        log.push_session_up("10.0.0.2".into(), 65002);
        assert_eq!(count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn flatten_as_path_works() {
        let segments = vec![
            AsPathSegment::Sequence(vec![65001, 65002]),
            AsPathSegment::Set(vec![65003, 65004]),
        ];
        assert_eq!(flatten_as_path(&segments), vec![65001, 65002, 65003, 65004]);
        assert!(flatten_as_path(&[]).is_empty());
    }
}
