use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::rib::{PeerRib, RibStore};
use crate::rib_persistence::PersistedPeerRib;

/// Metadata about a single RIB snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SnapshotMeta {
    pub peer_id: String,
    pub timestamp: DateTime<Utc>,
    pub route_count: usize,
    pub ipv4_count: usize,
    pub ipv6_count: usize,
}

/// Build the per-peer snapshot directory path.
fn snapshot_dir(data_dir: &str, peer_id: &str) -> PathBuf {
    let sanitized = peer_id.replace(':', "_");
    Path::new(data_dir).join(sanitized)
}

/// Format a timestamp for use in filenames (filesystem-safe).
/// Replaces colons with hyphens: `2026-03-26T14:00:00Z` → `2026-03-26T14-00-00Z`.
fn format_timestamp(ts: &DateTime<Utc>) -> String {
    ts.format("%Y-%m-%dT%H-%M-%SZ").to_string()
}

/// Parse a filesystem-safe timestamp back into a `DateTime<Utc>`.
fn parse_timestamp(name: &str) -> Option<DateTime<Utc>> {
    // Strip the .rib.json.gz or .meta.json suffix to get the timestamp part
    let ts_str = name
        .strip_suffix(".rib.json.gz")
        .or_else(|| name.strip_suffix(".meta.json"))?;
    // Parse from the filesystem-safe format
    chrono::NaiveDateTime::parse_from_str(ts_str, "%Y-%m-%dT%H-%M-%SZ")
        .ok()
        .map(|naive| naive.and_utc())
}

/// Save a snapshot of a single peer's RIB to disk.
///
/// Returns `None` if the peer's RIB is empty (no snapshot written).
/// Uses atomic writes (write to `.tmp`, then rename) to prevent partial reads.
pub fn save_snapshot(
    rib_store: &RibStore,
    peer_id: &str,
    data_dir: &str,
) -> Result<Option<SnapshotMeta>, std::io::Error> {
    let Some(rib_arc) = rib_store.get(peer_id) else {
        return Ok(None);
    };

    // Hold the read lock only long enough to clone route data
    let (routes, ipv4_count, ipv6_count) = {
        let Ok(rib) = rib_arc.read() else {
            warn!(peer = %peer_id, "failed to read-lock RIB for snapshot, skipping");
            return Ok(None);
        };

        if rib.total_count() == 0 {
            return Ok(None);
        }

        let ipv4_count = rib.ipv4_count();
        let ipv6_count = rib.ipv6_count();

        let mut routes = Vec::with_capacity(ipv4_count + ipv6_count);
        for (_prefix, prefix_routes) in rib.ipv4.iter() {
            routes.extend(prefix_routes.iter().cloned());
        }
        for (_prefix, prefix_routes) in rib.ipv6.iter() {
            routes.extend(prefix_routes.iter().cloned());
        }

        (routes, ipv4_count, ipv6_count)
    };
    // Read lock is dropped here — all I/O below is lock-free

    let now = Utc::now();
    let route_count = routes.len();
    let ts = format_timestamp(&now);
    let dir = snapshot_dir(data_dir, peer_id);
    fs::create_dir_all(&dir)?;

    // Write gzipped RIB data atomically
    let rib_path = dir.join(format!("{ts}.rib.json.gz"));
    let tmp_rib_path = dir.join(format!("{ts}.rib.json.gz.tmp"));

    let persisted = PersistedPeerRib {
        peer_id: peer_id.to_string(),
        saved_at: now,
        routes,
    };

    let file = fs::File::create(&tmp_rib_path)?;
    let mut encoder = GzEncoder::new(file, Compression::fast());
    let json = serde_json::to_vec(&persisted).map_err(std::io::Error::other)?;
    encoder.write_all(&json)?;
    encoder.finish()?;
    fs::rename(&tmp_rib_path, &rib_path)?;

    // Write companion metadata file atomically
    let meta = SnapshotMeta {
        peer_id: peer_id.to_string(),
        timestamp: now,
        route_count,
        ipv4_count,
        ipv6_count,
    };

    let meta_path = dir.join(format!("{ts}.meta.json"));
    let tmp_meta_path = dir.join(format!("{ts}.meta.json.tmp"));
    let meta_json = serde_json::to_vec_pretty(&meta).map_err(std::io::Error::other)?;
    fs::write(&tmp_meta_path, &meta_json)?;
    fs::rename(&tmp_meta_path, &meta_path)?;

    info!(
        peer = %peer_id,
        routes = route_count,
        ipv4 = ipv4_count,
        ipv6 = ipv6_count,
        file = %rib_path.display(),
        "saved RIB snapshot"
    );

    Ok(Some(meta))
}

/// List available snapshots for a peer, sorted newest-first.
pub fn list_snapshots(data_dir: &str, peer_id: &str) -> Result<Vec<SnapshotMeta>, std::io::Error> {
    let dir = snapshot_dir(data_dir, peer_id);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut snapshots = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".meta.json") {
            continue;
        }

        match fs::read(&path) {
            Ok(data) => match serde_json::from_slice::<SnapshotMeta>(&data) {
                Ok(meta) => snapshots.push(meta),
                Err(e) => warn!(file = %name, error = %e, "failed to parse snapshot metadata"),
            },
            Err(e) => warn!(file = %name, error = %e, "failed to read snapshot metadata"),
        }
    }

    // Sort newest first
    snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(snapshots)
}

/// Load a snapshot from disk and reconstruct a `PeerRib`.
pub fn load_snapshot(
    data_dir: &str,
    peer_id: &str,
    timestamp: &DateTime<Utc>,
) -> Result<PeerRib, std::io::Error> {
    let dir = snapshot_dir(data_dir, peer_id);
    let ts = format_timestamp(timestamp);
    let path = dir.join(format!("{ts}.rib.json.gz"));

    let file = fs::File::open(&path)?;
    let mut decoder = GzDecoder::new(file);
    let mut json = Vec::new();
    decoder.read_to_end(&mut json)?;

    let persisted: PersistedPeerRib =
        serde_json::from_slice(&json).map_err(std::io::Error::other)?;

    let mut rib = PeerRib::new();
    for route in persisted.routes {
        rib.insert(route);
    }

    Ok(rib)
}

/// Delete old snapshots that exceed the configured retention limits.
///
/// Returns the number of snapshots deleted.
pub fn enforce_retention(
    data_dir: &str,
    peer_id: &str,
    max_snapshots: Option<usize>,
    max_age_hours: Option<u64>,
) -> Result<usize, std::io::Error> {
    let dir = snapshot_dir(data_dir, peer_id);
    if !dir.exists() {
        return Ok(0);
    }

    // Collect all snapshot timestamps from .meta.json files
    let mut timestamps: Vec<DateTime<Utc>> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if name.ends_with(".meta.json")
            && let Some(ts) = parse_timestamp(&name)
        {
            timestamps.push(ts);
        }
    }

    // Sort oldest first for easier removal
    timestamps.sort();

    let now = Utc::now();
    let mut to_delete = std::collections::HashSet::new();

    // Mark snapshots exceeding max age
    if let Some(max_hours) = max_age_hours {
        let cutoff = now - chrono::Duration::hours(max_hours as i64);
        for ts in &timestamps {
            if *ts < cutoff {
                to_delete.insert(*ts);
            }
        }
    }

    // Mark oldest snapshots exceeding max count
    if let Some(max) = max_snapshots
        && timestamps.len() > max
    {
        let excess = timestamps.len() - max;
        for ts in timestamps.iter().take(excess) {
            to_delete.insert(*ts);
        }
    }

    // Delete marked snapshots
    let mut deleted = 0;
    for ts in &to_delete {
        let ts_str = format_timestamp(ts);
        let rib_path = dir.join(format!("{ts_str}.rib.json.gz"));
        let meta_path = dir.join(format!("{ts_str}.meta.json"));

        if let Err(e) = fs::remove_file(&rib_path) {
            warn!(file = %rib_path.display(), error = %e, "failed to delete old snapshot");
        }
        if let Err(e) = fs::remove_file(&meta_path) {
            warn!(file = %meta_path.display(), error = %e, "failed to delete old snapshot metadata");
        }
        deleted += 1;
    }

    if deleted > 0 {
        info!(
            peer = %peer_id,
            deleted,
            remaining = timestamps.len() - deleted,
            "cleaned up old snapshots"
        );
    }

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefix::Prefix;
    use crate::route::{AsPathSegment, BgpRoute, Origin};

    fn make_route(prefix_str: &str, next_hop: &str, as_path: Vec<u32>) -> BgpRoute {
        let prefix: Prefix = prefix_str.parse().unwrap();
        let origin_as = as_path.last().copied();
        BgpRoute {
            prefix,
            path_id: None,
            origin: Origin::Igp,
            as_path: vec![AsPathSegment::Sequence(as_path)],
            next_hop: next_hop.parse().unwrap(),
            med: None,
            local_pref: Some(100),
            communities: vec![],
            ext_communities: vec![],
            large_communities: vec![],
            origin_as,
            received_at: Utc::now(),
            stale: false,
            rpki_status: None,
        }
    }

    fn make_test_store() -> RibStore {
        let store = RibStore::new();
        {
            let rib_arc = store.get_or_create("10.0.0.1");
            let mut rib = rib_arc.write().unwrap();
            rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000, 65001]));
            rib.insert(make_route("192.168.0.0/16", "10.0.0.1", vec![65000, 65002]));
            rib.insert(make_route("2001:db8::/32", "::1", vec![65000, 65003]));
        }
        store
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();
        let store = make_test_store();

        // Save snapshot
        let meta = save_snapshot(&store, "10.0.0.1", data_dir)
            .unwrap()
            .unwrap();
        assert_eq!(meta.peer_id, "10.0.0.1");
        assert_eq!(meta.route_count, 3);
        assert_eq!(meta.ipv4_count, 2);
        assert_eq!(meta.ipv6_count, 1);

        // Load snapshot
        let rib = load_snapshot(data_dir, "10.0.0.1", &meta.timestamp).unwrap();
        assert_eq!(rib.ipv4_count(), 2);
        assert_eq!(rib.ipv6_count(), 1);
        assert_eq!(rib.total_count(), 3);
    }

    #[test]
    fn list_snapshots_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();
        let store = make_test_store();

        // Create multiple snapshots with different timestamps
        let meta1 = save_snapshot(&store, "10.0.0.1", data_dir)
            .unwrap()
            .unwrap();
        // Ensure a different timestamp by waiting briefly
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let meta2 = save_snapshot(&store, "10.0.0.1", data_dir)
            .unwrap()
            .unwrap();

        let snapshots = list_snapshots(data_dir, "10.0.0.1").unwrap();
        assert_eq!(snapshots.len(), 2);
        // Newest first
        assert_eq!(snapshots[0].timestamp, meta2.timestamp);
        assert_eq!(snapshots[1].timestamp, meta1.timestamp);
    }

    #[test]
    fn skip_empty_rib() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();
        let store = RibStore::new();
        store.get_or_create("10.0.0.1");

        let result = save_snapshot(&store, "10.0.0.1", data_dir).unwrap();
        assert!(result.is_none());

        // No files should be created
        let snapshots = list_snapshots(data_dir, "10.0.0.1").unwrap();
        assert!(snapshots.is_empty());
    }

    #[test]
    fn skip_unknown_peer() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();
        let store = RibStore::new();

        let result = save_snapshot(&store, "10.0.0.99", data_dir).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn list_snapshots_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();

        let snapshots = list_snapshots(data_dir, "10.0.0.1").unwrap();
        assert!(snapshots.is_empty());
    }

    #[test]
    fn enforce_retention_max_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();
        let store = make_test_store();

        // Create 3 snapshots
        for _ in 0..3 {
            save_snapshot(&store, "10.0.0.1", data_dir).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }

        let before = list_snapshots(data_dir, "10.0.0.1").unwrap();
        assert_eq!(before.len(), 3);

        // Keep only 2
        let deleted = enforce_retention(data_dir, "10.0.0.1", Some(2), None).unwrap();
        assert_eq!(deleted, 1);

        let after = list_snapshots(data_dir, "10.0.0.1").unwrap();
        assert_eq!(after.len(), 2);
        // Newest two should remain
        assert_eq!(after[0].timestamp, before[0].timestamp);
        assert_eq!(after[1].timestamp, before[1].timestamp);
    }

    #[test]
    fn enforce_retention_max_age() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();

        // Manually create a snapshot with an old timestamp
        let peer_dir = snapshot_dir(data_dir, "10.0.0.1");
        fs::create_dir_all(&peer_dir).unwrap();

        let old_ts = Utc::now() - chrono::Duration::hours(48);
        let ts_str = format_timestamp(&old_ts);

        // Write a minimal meta file
        let meta = SnapshotMeta {
            peer_id: "10.0.0.1".to_string(),
            timestamp: old_ts,
            route_count: 1,
            ipv4_count: 1,
            ipv6_count: 0,
        };
        let meta_json = serde_json::to_vec(&meta).unwrap();
        fs::write(peer_dir.join(format!("{ts_str}.meta.json")), &meta_json).unwrap();
        fs::write(peer_dir.join(format!("{ts_str}.rib.json.gz")), b"fake").unwrap();

        // Create a fresh snapshot
        let store = make_test_store();
        save_snapshot(&store, "10.0.0.1", data_dir).unwrap();

        let before = list_snapshots(data_dir, "10.0.0.1").unwrap();
        assert_eq!(before.len(), 2);

        // Delete snapshots older than 24 hours
        let deleted = enforce_retention(data_dir, "10.0.0.1", None, Some(24)).unwrap();
        assert_eq!(deleted, 1);

        let after = list_snapshots(data_dir, "10.0.0.1").unwrap();
        assert_eq!(after.len(), 1);
    }

    #[test]
    fn atomic_write_not_corrupted() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();
        let store = make_test_store();

        let meta = save_snapshot(&store, "10.0.0.1", data_dir)
            .unwrap()
            .unwrap();

        // Verify no .tmp files remain
        let peer_dir = snapshot_dir(data_dir, "10.0.0.1");
        for entry in fs::read_dir(&peer_dir).unwrap() {
            let name = entry.unwrap().file_name().to_str().unwrap().to_string();
            assert!(!name.ends_with(".tmp"), "found leftover tmp file: {name}");
        }

        // Verify the snapshot loads correctly
        let rib = load_snapshot(data_dir, "10.0.0.1", &meta.timestamp).unwrap();
        assert_eq!(rib.total_count(), 3);
    }

    #[test]
    fn timestamp_roundtrip() {
        use chrono::Timelike;
        let now = Utc::now();
        // Truncate to seconds (our format doesn't include sub-seconds)
        let truncated = now.with_nanosecond(0).unwrap();
        let formatted = format_timestamp(&truncated);
        let parsed = parse_timestamp(&format!("{formatted}.meta.json")).unwrap();
        assert_eq!(parsed, truncated);
    }
}
