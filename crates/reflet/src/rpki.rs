use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::info;

use reflet_core::rpki::RpkiStore;

#[derive(Deserialize)]
struct RoutinatorResponse {
    roas: Vec<RoutinatorVrp>,
}

#[derive(Deserialize)]
struct RoutinatorVrp {
    asn: String,
    prefix: String,
    #[serde(rename = "maxLength")]
    max_length: u8,
}

/// Fetch VRPs from a Routinator-compatible RPKI validator.
pub async fn fetch(url: &str) -> Result<RpkiStore> {
    let client = reqwest::Client::builder()
        .user_agent("reflet/0.1")
        .build()
        .context("failed to build HTTP client")?;

    let fetch_url = format!("{}/json", url.trim_end_matches('/'));
    let resp: RoutinatorResponse = client
        .get(&fetch_url)
        .send()
        .await
        .context("failed to fetch RPKI VRPs")?
        .error_for_status()
        .context("RPKI validator returned error status")?
        .json()
        .await
        .context("failed to parse RPKI JSON response")?;

    let mut vrps = Vec::with_capacity(resp.roas.len());
    for roa in &resp.roas {
        let asn = roa
            .asn
            .strip_prefix("AS")
            .unwrap_or(&roa.asn)
            .parse::<u32>()
            .context("invalid ASN in VRP")?;
        let prefix = roa.prefix.parse().context("invalid prefix in VRP")?;
        vrps.push((prefix, asn, roa.max_length));
    }

    let store = RpkiStore::from_vrps(vrps);
    info!(vrps = store.vrp_count(), "fetched RPKI VRPs");
    Ok(store)
}

/// Spawn a background task that periodically refreshes the RPKI store.
pub fn spawn_refresh_task(
    url: String,
    interval_secs: u64,
    store: Arc<RwLock<RpkiStore>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(interval_secs);
        loop {
            tokio::time::sleep(interval).await;
            match fetch(&url).await {
                Ok(new_store) => {
                    info!(vrps = new_store.vrp_count(), "refreshed RPKI VRPs");
                    if let Ok(mut s) = store.write() {
                        *s = new_store;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to refresh RPKI VRPs, keeping old data");
                }
            }
        }
    })
}
