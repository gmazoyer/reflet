use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::rib::RibStore;
use crate::route::BgpRoute;

const RESTART_MARKER: &str = ".restart_marker";

/// Serializable form of a peer's RIB.
#[derive(Serialize, Deserialize)]
struct PersistedPeerRib {
    peer_id: String,
    saved_at: DateTime<Utc>,
    routes: Vec<BgpRoute>,
}

/// Save all peer RIBs to gzipped JSON files in `data_dir`.
/// Writes a `.restart_marker` sentinel after all files are written successfully.
pub fn save_ribs(rib_store: &RibStore, data_dir: &str) -> Result<(), std::io::Error> {
    let dir = Path::new(data_dir);
    fs::create_dir_all(dir)?;

    let now = Utc::now();
    let peer_ids = rib_store.peer_ids();

    for peer_id in &peer_ids {
        let Some(rib_arc) = rib_store.get(peer_id) else {
            continue;
        };
        let Ok(rib) = rib_arc.read() else {
            warn!(peer = %peer_id, "failed to read-lock RIB for persistence, skipping");
            continue;
        };

        if rib.total_count() == 0 {
            continue;
        }

        let mut routes = Vec::new();
        for (_prefix, prefix_routes) in rib.ipv4.iter() {
            routes.extend(prefix_routes.iter().cloned());
        }
        for (_prefix, prefix_routes) in rib.ipv6.iter() {
            routes.extend(prefix_routes.iter().cloned());
        }

        let persisted = PersistedPeerRib {
            peer_id: peer_id.clone(),
            saved_at: now,
            routes,
        };

        // Use a sanitized filename (replace colons for IPv6 addresses)
        let filename = format!("{}.rib.json.gz", peer_id.replace(':', "_"));
        let path = dir.join(&filename);

        let file = fs::File::create(&path)?;
        let mut encoder = GzEncoder::new(file, Compression::fast());
        let json = serde_json::to_vec(&persisted).map_err(std::io::Error::other)?;
        encoder.write_all(&json)?;
        encoder.finish()?;

        info!(
            peer = %peer_id,
            routes = persisted.routes.len(),
            file = %filename,
            "saved RIB to disk"
        );
    }

    // Write restart marker sentinel
    fs::write(dir.join(RESTART_MARKER), now.to_rfc3339())?;
    info!("wrote restart marker");

    Ok(())
}

/// Load persisted RIBs from `data_dir` into `rib_store`.
/// All loaded routes are marked as stale.
/// Returns the list of peer IDs that were loaded.
///
/// Returns an empty list if no `.restart_marker` is found (e.g. after a crash).
pub fn load_ribs(rib_store: &RibStore, data_dir: &str) -> Result<Vec<String>, std::io::Error> {
    let dir = Path::new(data_dir);

    // Check for restart marker
    let marker_path = dir.join(RESTART_MARKER);
    if !marker_path.exists() {
        info!("no restart marker found, starting with empty RIBs");
        return Ok(vec![]);
    }

    let mut loaded_peers = Vec::new();

    // Find all .rib.json.gz files
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".rib.json.gz") {
            continue;
        }

        match load_single_rib(&path) {
            Ok(persisted) => {
                let peer_id = persisted.peer_id.clone();
                let route_count = persisted.routes.len();

                let rib_arc = rib_store.get_or_create(&peer_id);
                if let Ok(mut rib) = rib_arc.write() {
                    for mut route in persisted.routes {
                        // Mark all loaded routes as stale
                        // (#[serde(skip)] defaults stale to false, so set explicitly)
                        route.stale = true;
                        rib.insert(route);
                    }
                }

                info!(
                    peer = %peer_id,
                    routes = route_count,
                    "loaded persisted RIB"
                );
                loaded_peers.push(peer_id);
            }
            Err(e) => {
                warn!(file = %name, error = %e, "failed to load persisted RIB, skipping");
            }
        }
    }

    // Remove restart marker so a subsequent crash doesn't try to load stale data
    if let Err(e) = fs::remove_file(&marker_path) {
        warn!(error = %e, "failed to remove restart marker");
    }

    Ok(loaded_peers)
}

fn load_single_rib(path: &Path) -> Result<PersistedPeerRib, Box<dyn std::error::Error>> {
    let file = fs::File::open(path)?;
    let mut decoder = GzDecoder::new(file);
    let mut json = Vec::new();
    decoder.read_to_end(&mut json)?;
    let persisted: PersistedPeerRib = serde_json::from_slice(&json)?;
    Ok(persisted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefix::Prefix;
    use crate::route::{AsPathSegment, Origin};

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

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();

        // Create a RibStore with routes
        let store = RibStore::new();
        {
            let rib_arc = store.get_or_create("10.0.0.1");
            let mut rib = rib_arc.write().unwrap();
            rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000, 65001]));
            rib.insert(make_route("192.168.0.0/16", "10.0.0.1", vec![65000]));
            rib.insert(make_route("2001:db8::/32", "::1", vec![65000, 65002]));
        }

        // Save
        save_ribs(&store, data_dir).unwrap();

        // Load into a new store
        let store2 = RibStore::new();
        let loaded = load_ribs(&store2, data_dir).unwrap();

        assert_eq!(loaded, vec!["10.0.0.1"]);

        let rib_arc = store2.get("10.0.0.1").unwrap();
        let rib = rib_arc.read().unwrap();
        assert_eq!(rib.ipv4_count(), 2);
        assert_eq!(rib.ipv6_count(), 1);
        assert_eq!(rib.total_count(), 3);
    }

    #[test]
    fn load_without_marker_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();

        let store = RibStore::new();
        let loaded = load_ribs(&store, data_dir).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_with_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();

        // Create a valid RIB file
        let store = RibStore::new();
        {
            let rib_arc = store.get_or_create("10.0.0.1");
            let mut rib = rib_arc.write().unwrap();
            rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));
        }
        save_ribs(&store, data_dir).unwrap();

        // Create a corrupt file
        fs::write(dir.path().join("10.0.0.99.rib.json.gz"), b"not valid gzip").unwrap();

        // Load should succeed with partial data
        let store2 = RibStore::new();
        let loaded = load_ribs(&store2, data_dir).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], "10.0.0.1");
    }

    #[test]
    fn stale_flag_set_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().to_str().unwrap();

        let store = RibStore::new();
        {
            let rib_arc = store.get_or_create("10.0.0.1");
            let mut rib = rib_arc.write().unwrap();
            rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65000]));
            rib.insert(make_route("2001:db8::/32", "::1", vec![65000]));
        }
        save_ribs(&store, data_dir).unwrap();

        let store2 = RibStore::new();
        load_ribs(&store2, data_dir).unwrap();

        let rib_arc = store2.get("10.0.0.1").unwrap();
        let rib = rib_arc.read().unwrap();

        for (_prefix, routes) in rib.ipv4.iter() {
            for route in routes {
                assert!(route.stale, "loaded IPv4 route should be stale");
            }
        }
        for (_prefix, routes) in rib.ipv6.iter() {
            for route in routes {
                assert!(route.stale, "loaded IPv6 route should be stale");
            }
        }
    }
}
