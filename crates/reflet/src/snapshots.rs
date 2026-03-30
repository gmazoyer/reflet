use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::info;

use reflet_core::config::{Config, SnapshotConfig};
use reflet_core::rib::RibStore;
use reflet_core::rib_snapshots;

/// Spawn one background task per peer that has `snapshot_interval > 0`.
///
/// Each task periodically saves a RIB snapshot to disk and enforces retention.
/// Returns the task handles so they can be aborted on shutdown.
pub fn spawn_snapshot_tasks(config: &Config, rib_store: &RibStore) -> Vec<JoinHandle<()>> {
    let snap_config = match &config.snapshots {
        Some(c) => c.clone(),
        None => return vec![],
    };

    let mut handles = Vec::new();

    for peer in &config.peers {
        let interval = match peer.snapshot_interval {
            Some(i) if i >= 60 => i,
            _ => continue,
        };

        let peer_id = peer.address.to_string();
        let rib_store = rib_store.clone();
        let snap_config = snap_config.clone();

        let handle = tokio::spawn(async move {
            snapshot_loop(&peer_id, interval, &rib_store, &snap_config).await;
        });

        info!(
            peer = %peer.address,
            interval_secs = interval,
            "started snapshot task"
        );

        handles.push(handle);
    }

    handles
}

async fn snapshot_loop(
    peer_id: &str,
    interval: u64,
    rib_store: &RibStore,
    snap_config: &SnapshotConfig,
) {
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        // Save snapshot in a blocking thread to avoid stalling the async runtime
        let rib_store = rib_store.clone();
        let peer_id = peer_id.to_string();
        let data_dir = snap_config.data_dir.clone();
        let max_snapshots = snap_config.max_snapshots;
        let max_age_hours = snap_config.max_age_hours;

        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = rib_snapshots::save_snapshot(&rib_store, &peer_id, &data_dir) {
                tracing::warn!(peer = %peer_id, error = %e, "failed to save RIB snapshot");
                return;
            }
            if let Err(e) =
                rib_snapshots::enforce_retention(&data_dir, &peer_id, max_snapshots, max_age_hours)
            {
                tracing::warn!(peer = %peer_id, error = %e, "failed to enforce snapshot retention");
            }
        })
        .await;
    }
}
