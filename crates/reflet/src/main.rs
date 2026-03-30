mod rpki;
mod snapshots;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use clap::Parser;
use tracing::info;

use reflet_core::asn::AsnStore;
use reflet_core::community::CommunityStore;
use reflet_core::config::{Config, LogFormat};
use reflet_core::event_log::EventLog;
use reflet_core::peer::PeerState;
use reflet_core::rib::RibStore;
use reflet_core::rib_persistence;
use reflet_core::rpki::RpkiStore;

use reflet_api::app::build_router;
use reflet_api::state::AppState;
use reflet_bgp::speaker::BgpSpeaker;

#[derive(Parser, Debug)]
#[command(name = "reflet", version, about = "BGP Looking Glass")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    /// Validate configuration and exit
    #[arg(long)]
    check: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load configuration
    let config =
        Config::from_file(Path::new(&cli.config)).context("failed to load configuration")?;

    if cli.check {
        println!("Configuration file '{}' is valid.", cli.config);
        return Ok(());
    }

    // Initialize logging
    init_logging(&config.logging);

    info!(config = %cli.config, "loaded configuration");

    // Create shared RIB store
    let rib_store = RibStore::new();

    // Load persisted RIBs if Graceful Restart is enabled
    let mut is_restarting = false;
    if config.bgp.graceful_restart.enabled
        && let Some(ref data_dir) = config.bgp.graceful_restart.data_dir
    {
        match rib_persistence::load_ribs(&rib_store, data_dir) {
            Ok(loaded_peers) => {
                if !loaded_peers.is_empty() {
                    is_restarting = true;
                    info!(
                        peers = loaded_peers.len(),
                        "loaded persisted RIBs, entering restart mode"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load persisted RIBs, starting fresh");
            }
        }
    }

    // Create event log with SSE notifier
    let event_notify = Arc::new(tokio::sync::Notify::new());
    let notify_ref = event_notify.clone();
    let event_log = if config.event_log.enabled {
        EventLog::new(
            config.event_log.buffer_size,
            config.event_log.file.as_deref(),
        )
        .context("failed to create event log")?
    } else {
        EventLog::disabled()
    }
    .with_notifier(Arc::new(move || notify_ref.notify_waiters()));

    // Create BGP speaker
    let speaker = BgpSpeaker::new(
        config.bgp.clone(),
        config.peers.clone(),
        config.server.bgp_listen,
        rib_store.clone(),
        is_restarting,
        event_log.clone(),
    );

    // Load community definitions
    let community_store = match &config.communities_dir {
        Some(dir) => {
            let path = Path::new(dir);
            if path.is_dir() {
                match CommunityStore::load(path) {
                    Ok(store) => {
                        let defs = store.definitions();
                        info!(
                            standard = defs.standard.len(),
                            large = defs.large.len(),
                            patterns = defs.patterns.len(),
                            "loaded community definitions"
                        );
                        store
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to load community definitions, using empty store");
                        CommunityStore::empty()
                    }
                }
            } else {
                tracing::warn!(dir = %dir, "communities_dir does not exist, using empty store");
                CommunityStore::empty()
            }
        }
        None => CommunityStore::empty(),
    };

    // Load ASN database
    let asn_store = match &config.ipinfo_dataset_file {
        Some(file) => match AsnStore::load(Path::new(file)) {
            Ok(store) => {
                info!(count = store.len(), "loaded ASN database");
                store
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to load ASN database, using empty store");
                AsnStore::empty()
            }
        },
        None => AsnStore::empty(),
    };

    // Load RPKI VRPs
    let rpki_store = if config.rpki.enabled {
        if let Some(ref url) = config.rpki.url {
            match rpki::fetch(url).await {
                Ok(store) => {
                    info!(vrps = store.vrp_count(), "loaded RPKI VRPs");
                    Arc::new(RwLock::new(store))
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to load RPKI VRPs, starting with empty store");
                    Arc::new(RwLock::new(RpkiStore::empty()))
                }
            }
        } else {
            Arc::new(RwLock::new(RpkiStore::empty()))
        }
    } else {
        Arc::new(RwLock::new(RpkiStore::empty()))
    };

    // Start RPKI refresh task if enabled
    let _rpki_handle = if config.rpki.enabled {
        config.rpki.url.as_ref().map(|url| {
            rpki::spawn_refresh_task(
                url.clone(),
                config.rpki.refresh_interval,
                rpki_store.clone(),
            )
        })
    } else {
        None
    };

    // Keep a clone of rib_store for GR shutdown persistence and timer
    let rib_store_main = rib_store.clone();

    // Wrap reloadable fields in Arc<RwLock<>> for SIGHUP config reload
    let community_store = Arc::new(RwLock::new(community_store));
    let asn_store = Arc::new(RwLock::new(asn_store));
    let title = Arc::new(RwLock::new(config.server.title.clone()));
    let hide_peer_addresses = Arc::new(RwLock::new(config.server.hide_peer_addresses));
    let disable_route_refresh = Arc::new(RwLock::new(config.server.disable_route_refresh));

    // Create API state
    let snapshot_data_dir = config.snapshots.as_ref().map(|s| s.data_dir.clone());
    let state = AppState::new(
        rib_store,
        speaker.peers(),
        config.bgp.clone(),
        community_store.clone(),
        asn_store.clone(),
        title.clone(),
        hide_peer_addresses.clone(),
        disable_route_refresh.clone(),
        speaker.command_channels(),
        event_log.clone(),
        event_notify,
        rpki_store.clone(),
        snapshot_data_dir,
    );

    // Spawn SIGHUP handler for config reload
    #[cfg(unix)]
    {
        let reload_config_path = cli.config.clone();
        let reload_title = title;
        let reload_hide = hide_peer_addresses;
        let reload_disable_route_refresh = disable_route_refresh;
        let reload_communities = community_store;
        let reload_asns = asn_store;
        let reload_rpki = rpki_store;
        tokio::spawn(async move {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sig = signal(SignalKind::hangup()).expect("failed to install SIGHUP handler");
            loop {
                sig.recv().await;
                info!("received SIGHUP, reloading configuration");
                reload_config(
                    &reload_config_path,
                    &reload_title,
                    &reload_hide,
                    &reload_disable_route_refresh,
                    &reload_communities,
                    &reload_asns,
                    &reload_rpki,
                );
            }
        });
    }

    // Clone shutdown handles before state is moved into the router
    let shutdown_flag = state.shutdown.clone();
    let shutdown_notify = state.event_notify.clone();

    // Build HTTP router
    let app = build_router(state);

    // Get the is_restarting flag and peers map before moving speaker
    let is_restarting_flag = speaker.is_restarting();
    let peers_for_timer = speaker.peers();

    // Start BGP speaker in background
    let bgp_handle = tokio::spawn(async move {
        if let Err(e) = speaker.run().await {
            tracing::error!(error = %e, "BGP speaker error");
        }
    });

    // Start global restart timer if we're in restart mode
    if is_restarting {
        let restart_time = config.bgp.graceful_restart.restart_time;
        let flag = is_restarting_flag;
        let peers = peers_for_timer;
        let rib = rib_store_main.clone();

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(restart_time as u64)).await;

            // Clear the restarting flag so new sessions no longer set R-bit
            flag.store(false, Ordering::Relaxed);
            info!("global GR restart timer expired, clearing R-bit");

            // Clear stale routes for peers still in Idle (they didn't reconnect in time)
            if let Ok(peers_map) = peers.read() {
                for (peer_id, info_arc) in peers_map.iter() {
                    let is_idle = info_arc
                        .read()
                        .map(|info| info.state == PeerState::Idle)
                        .unwrap_or(false);

                    if is_idle {
                        if let Some(rib_arc) = rib.get(peer_id)
                            && let Ok(mut peer_rib) = rib_arc.write()
                            && peer_rib.total_count() > 0
                        {
                            let cleared = peer_rib.total_count();
                            peer_rib.clear();
                            info!(
                                peer = %peer_id,
                                cleared,
                                "cleared stale routes for peer that didn't reconnect"
                            );
                        }
                        // Reset prefix counts
                        if let Ok(mut info) = info_arc.write() {
                            info.prefixes = Default::default();
                        }
                    }
                }
            }
        });
    }

    // Start snapshot tasks for peers that have snapshot_interval configured
    let snapshot_handles = snapshots::spawn_snapshot_tasks(&config, &rib_store_main);

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind(config.server.listen).await?;
    info!(addr = %config.server.listen, "HTTP server listening");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        // Signal SSE streams to terminate so connections can close
        shutdown_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        shutdown_notify.notify_waiters();
    })
    .await?;

    info!("shutting down");

    // Flush event log
    event_log.flush();

    // Save RIBs on graceful shutdown if GR is enabled
    if config.bgp.graceful_restart.enabled
        && let Some(ref data_dir) = config.bgp.graceful_restart.data_dir
    {
        match rib_persistence::save_ribs(&rib_store_main, data_dir) {
            Ok(()) => info!("saved RIBs for graceful restart"),
            Err(e) => tracing::warn!(error = %e, "failed to save RIBs"),
        }
    }

    bgp_handle.abort();
    for handle in snapshot_handles {
        handle.abort();
    }

    Ok(())
}

fn reload_config(
    config_path: &str,
    title: &Arc<RwLock<String>>,
    hide_peer_addresses: &Arc<RwLock<bool>>,
    disable_route_refresh: &Arc<RwLock<bool>>,
    community_store: &Arc<RwLock<CommunityStore>>,
    asn_store: &Arc<RwLock<AsnStore>>,
    _rpki_store: &Arc<RwLock<RpkiStore>>,
) {
    let config = match Config::from_file(Path::new(config_path)) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to reload configuration, keeping current config");
            return;
        }
    };

    // Update server presentation settings
    *title.write().unwrap() = config.server.title.clone();
    *hide_peer_addresses.write().unwrap() = config.server.hide_peer_addresses;
    *disable_route_refresh.write().unwrap() = config.server.disable_route_refresh;

    // Reload community definitions
    let new_communities = match &config.communities_dir {
        Some(dir) => {
            let path = Path::new(dir);
            if path.is_dir() {
                match CommunityStore::load(path) {
                    Ok(store) => {
                        let defs = store.definitions();
                        info!(
                            standard = defs.standard.len(),
                            large = defs.large.len(),
                            patterns = defs.patterns.len(),
                            "reloaded community definitions"
                        );
                        store
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to reload community definitions, keeping current");
                        return;
                    }
                }
            } else {
                tracing::warn!(dir = %dir, "communities_dir does not exist, using empty store");
                CommunityStore::empty()
            }
        }
        None => CommunityStore::empty(),
    };
    *community_store.write().unwrap() = new_communities;

    // Reload ASN database
    let new_asns = match &config.ipinfo_dataset_file {
        Some(file) => match AsnStore::load(Path::new(file)) {
            Ok(store) => {
                info!(count = store.len(), "reloaded ASN database");
                store
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to reload ASN database, keeping current");
                return;
            }
        },
        None => AsnStore::empty(),
    };
    *asn_store.write().unwrap() = new_asns;

    // Note: BGP peers, listen addresses, RPKI URL/interval, logging, and event log
    // settings are not reloaded. Those require a full restart.

    info!("configuration reloaded successfully");
}

fn init_logging(config: &reflet_core::config::LoggingConfig) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.level));

    match config.format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .json()
                .init();
        }
        LogFormat::Pretty => {
            tracing_subscriber::fmt().with_env_filter(env_filter).init();
        }
    }
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
    info!("received shutdown signal");
}
