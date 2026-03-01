use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use http_body_util::BodyExt;
use tower::ServiceExt;

use reflet_api::app::build_router;
use reflet_api::state::AppState;
use reflet_core::asn::AsnStore;
use reflet_core::community::CommunityStore;
use reflet_core::config::BgpConfig;
use reflet_core::event_log::EventLog;
use reflet_core::peer::{PeerInfo, PeerState, PrefixCounts};
use reflet_core::prefix::{AddressFamily, Prefix};
use reflet_core::rib::RibStore;
use reflet_core::route::{AsPathSegment, BgpRoute, Community, LargeCommunity, Origin};
use reflet_core::rpki::RpkiStore;

fn test_notify() -> Arc<tokio::sync::Notify> {
    Arc::new(tokio::sync::Notify::new())
}

/// Helper to create a test AppState with pre-populated data.
fn test_state() -> AppState {
    let rib_store = RibStore::new();
    let peers_map: HashMap<String, Arc<RwLock<PeerInfo>>> = HashMap::new();
    let peers = Arc::new(RwLock::new(peers_map));

    // Create peer A (Established with routes)
    let all_families = vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast];
    let mut peer_a = PeerInfo::new(
        "10.0.0.2".parse().unwrap(),
        65001,
        "Router A".into(),
        "Primary router in Amsterdam".into(),
        Some("DC1, Amsterdam".into()),
        all_families.clone(),
    );
    peer_a.state = PeerState::Established;
    peer_a.uptime = Some(Utc::now());
    peer_a.prefixes = PrefixCounts { ipv4: 2, ipv6: 1 };
    peer_a.router_id = "10.0.0.2".parse().unwrap();

    // Create peer B (Idle, no routes)
    let peer_b = PeerInfo::new(
        "10.0.0.3".parse().unwrap(),
        65002,
        "Router B".into(),
        String::new(),
        None,
        all_families,
    );

    // Insert peers
    {
        let mut p = peers.write().unwrap();
        p.insert(peer_a.id.clone(), Arc::new(RwLock::new(peer_a.clone())));
        p.insert(peer_b.id.clone(), Arc::new(RwLock::new(peer_b)));
    }

    // Populate RIB for peer A
    {
        let rib_arc = rib_store.get_or_create(&peer_a.id);
        let mut rib = rib_arc.write().unwrap();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65001, 65010]));
        rib.insert(make_route("192.168.1.0/24", "10.0.0.1", vec![65001, 65020]));
        rib.insert(make_route("2001:db8::/32", "::1", vec![65001, 65030]));
    }

    let bgp_config = BgpConfig::default();

    AppState::new(
        rib_store,
        peers,
        bgp_config,
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
    )
}

fn make_route(prefix_str: &str, next_hop: &str, as_path: Vec<u32>) -> BgpRoute {
    make_route_with_path_id(prefix_str, next_hop, as_path, None)
}

fn make_route_with_path_id(
    prefix_str: &str,
    next_hop: &str,
    as_path: Vec<u32>,
    path_id: Option<u32>,
) -> BgpRoute {
    let prefix: Prefix = prefix_str.parse().unwrap();
    let origin_as = as_path.last().copied();
    BgpRoute {
        prefix,
        path_id,
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

async fn post(app: axum::Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    (status, body_str)
}

async fn get(app: axum::Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();

    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8(body.to_vec()).unwrap();
    (status, body_str)
}

// --- Health ---

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/health").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

// --- Summary ---

#[tokio::test]
async fn summary_returns_correct_counts() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/summary").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["local_asn"], 65000);
    assert_eq!(json["peer_count"], 2);
    assert_eq!(json["established_peers"], 1);
    assert_eq!(json["total_ipv4_prefixes"], 2);
    assert_eq!(json["total_ipv6_prefixes"], 1);
}

// --- Peers ---

#[tokio::test]
async fn list_peers_returns_all() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let peers = json.as_array().unwrap();
    assert_eq!(peers.len(), 2);
}

#[tokio::test]
async fn get_peer_by_id() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["remote_asn"], 65001);
    assert_eq!(json["name"], "Router A");
    assert_eq!(json["description"], "Primary router in Amsterdam");
    assert_eq!(json["state"], "Established");
}

#[tokio::test]
async fn get_peer_not_found() {
    let app = build_router(test_state());
    let (status, _) = get(app, "/api/v1/peers/nonexistent").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- Routes ---

#[tokio::test]
async fn get_peer_ipv4_routes() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(json["meta"]["total"], 2);
    assert_eq!(json["meta"]["page"], 1);
    assert_eq!(json["meta"]["per_page"], 100);
}

#[tokio::test]
async fn get_peer_ipv6_routes() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv6").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(json["meta"]["total"], 1);
}

#[tokio::test]
async fn get_peer_routes_pagination() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4?page=1&per_page=1").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(json["meta"]["total"], 2);
    assert_eq!(json["meta"]["per_page"], 1);
}

#[tokio::test]
async fn get_peer_routes_search() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4?search=192.168").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "192.168.1.0/24");
}

#[tokio::test]
async fn get_routes_for_unknown_peer() {
    let app = build_router(test_state());
    let (status, _) = get(app, "/api/v1/peers/nonexistent/routes/ipv4").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- Lookup ---

#[tokio::test]
async fn lookup_exact() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/lookup?prefix=10.0.0.0/24&type=exact").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["lookup_type"], "exact");
    assert_eq!(json["query"], "10.0.0.0/24");
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["routes"][0]["prefix"], "10.0.0.0/24");
}

#[tokio::test]
async fn lookup_longest_match() {
    let app = build_router(test_state());
    let (status, body) = get(
        app,
        "/api/v1/lookup?prefix=10.0.0.128/32&type=longest-match",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["matched_prefix"], "10.0.0.0/24");
}

#[tokio::test]
async fn lookup_subnets() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/lookup?prefix=10.0.0.0/16&type=subnets").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
}

#[tokio::test]
async fn lookup_invalid_prefix() {
    let app = build_router(test_state());
    let (status, _) = get(app, "/api/v1/lookup?prefix=invalid&type=exact").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn lookup_invalid_type() {
    let app = build_router(test_state());
    let (status, _) = get(app, "/api/v1/lookup?prefix=10.0.0.0/24&type=badtype").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn lookup_default_type_is_longest_match() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/lookup?prefix=10.0.0.128/32").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["lookup_type"], "longest-match");
}

// --- Lookup without prefix length ---

#[tokio::test]
async fn lookup_ipv4_without_prefix_length() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/lookup?prefix=10.0.0.1&type=longest-match").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["query"], "10.0.0.1");
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["matched_prefix"], "10.0.0.0/24");
}

#[tokio::test]
async fn lookup_ipv6_without_prefix_length() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/lookup?prefix=2001:db8::1&type=longest-match").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["query"], "2001:db8::1");
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["matched_prefix"], "2001:db8::/32");
}

// --- RFC 8522 ---

#[tokio::test]
async fn rfc8522_list_routers() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/.well-known/looking-glass/v1/routers").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "success");
    let routers = json["data"].as_array().unwrap();
    assert_eq!(routers.len(), 2);
}

#[tokio::test]
async fn rfc8522_get_router() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/.well-known/looking-glass/v1/routers/Router%20A").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "success");
    assert_eq!(json["data"]["remote_asn"], 65001);
}

#[tokio::test]
async fn rfc8522_available_commands() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/.well-known/looking-glass/v1/cmd").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "success");
    let cmds = json["data"].as_array().unwrap();
    assert!(!cmds.is_empty());
}

#[tokio::test]
async fn rfc8522_bgp_summary() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/.well-known/looking-glass/v1/show/bgp/summary").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "success");
    let entries = json["data"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn rfc8522_show_route() {
    let app = build_router(test_state());
    let (status, body) = get(
        app,
        "/.well-known/looking-glass/v1/show/route/10.0.0.0%2F24",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["status"], "success");
    let entries = json["data"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["prefix"], "10.0.0.0/24");
}

// --- Routes search by ASN ---

#[tokio::test]
async fn get_peer_routes_search_by_asn() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4?search=AS65010").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.0.0/24");
    assert_eq!(json["meta"]["total"], 1);
}

#[tokio::test]
async fn get_peer_routes_search_by_as_path() {
    let app = build_router(test_state());
    let (status, body) = get(
        app,
        "/api/v1/peers/Router%20A/routes/ipv4?search=65001%2065010",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.0.0/24");
    assert_eq!(json["meta"]["total"], 1);
}

#[tokio::test]
async fn get_peer_routes_search_prefix_backward_compat() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4?search=10.0.0").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.0.0/24");
}

// --- Add-Path ---

#[tokio::test]
async fn lookup_exact_returns_multiple_paths() {
    let rib_store = RibStore::new();
    let peers_map: HashMap<String, Arc<RwLock<PeerInfo>>> = HashMap::new();
    let peers = Arc::new(RwLock::new(peers_map));

    let mut peer_a = PeerInfo::new(
        "10.0.0.2".parse().unwrap(),
        65001,
        "Router A".into(),
        String::new(),
        None,
        vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast],
    );
    peer_a.state = PeerState::Established;
    peer_a.prefixes = PrefixCounts { ipv4: 2, ipv6: 0 };

    {
        let mut p = peers.write().unwrap();
        p.insert(peer_a.id.clone(), Arc::new(RwLock::new(peer_a.clone())));
    }

    // Insert two Add-Path routes for the same prefix
    {
        let rib_arc = rib_store.get_or_create(&peer_a.id);
        let mut rib = rib_arc.write().unwrap();
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65001, 65010],
            Some(1),
        ));
        rib.insert(make_route_with_path_id(
            "10.0.0.0/24",
            "10.0.0.2",
            vec![65001, 65020],
            Some(2),
        ));
    }

    let bgp_config = BgpConfig::default();
    let state = AppState::new(
        rib_store,
        peers,
        bgp_config,
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
    );
    let app = build_router(state);

    let (status, body) = get(app, "/api/v1/lookup?prefix=10.0.0.0/24&type=exact").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 1); // one peer
    let routes = results[0]["routes"].as_array().unwrap();
    assert_eq!(routes.len(), 2); // two paths
    assert_eq!(routes[0]["path_id"], 1);
    assert_eq!(routes[1]["path_id"], 2);
}

// --- Communities ---

#[tokio::test]
async fn community_definitions_returns_empty() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/communities/definitions").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json["standard"].as_object().unwrap().is_empty());
    assert!(json["large"].as_object().unwrap().is_empty());
    assert!(json["patterns"].as_array().unwrap().is_empty());
}

// --- ASNs ---

#[tokio::test]
async fn asn_info_returns_empty() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/asns").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.as_object().unwrap().is_empty());
}

// --- Advanced route filtering ---

fn make_route_with_attrs(
    prefix_str: &str,
    next_hop: &str,
    as_path: Vec<u32>,
    origin: Origin,
    med: Option<u32>,
    local_pref: Option<u32>,
    communities: Vec<Community>,
    large_communities: Vec<LargeCommunity>,
) -> BgpRoute {
    let prefix: Prefix = prefix_str.parse().unwrap();
    let origin_as = as_path.last().copied();
    BgpRoute {
        prefix,
        path_id: None,
        origin,
        as_path: vec![AsPathSegment::Sequence(as_path)],
        next_hop: next_hop.parse().unwrap(),
        med,
        local_pref,
        communities,
        ext_communities: vec![],
        large_communities,
        origin_as,
        received_at: Utc::now(),
        stale: false,
        rpki_status: None,
    }
}

fn test_state_with_attrs() -> AppState {
    let rib_store = RibStore::new();
    let peers_map: HashMap<String, Arc<RwLock<PeerInfo>>> = HashMap::new();
    let peers = Arc::new(RwLock::new(peers_map));

    let mut peer_a = PeerInfo::new(
        "10.0.0.2".parse().unwrap(),
        65001,
        "Router A".into(),
        String::new(),
        None,
        vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast],
    );
    peer_a.state = PeerState::Established;
    peer_a.prefixes = PrefixCounts { ipv4: 3, ipv6: 0 };

    {
        let mut p = peers.write().unwrap();
        p.insert(peer_a.id.clone(), Arc::new(RwLock::new(peer_a.clone())));
    }

    {
        let rib_arc = rib_store.get_or_create(&peer_a.id);
        let mut rib = rib_arc.write().unwrap();

        // Route 1: community 65000:100, origin IGP, med 50, localpref 200
        rib.insert(make_route_with_attrs(
            "10.0.0.0/24",
            "10.0.0.1",
            vec![65001, 65010],
            Origin::Igp,
            Some(50),
            Some(200),
            vec![Community {
                asn: 65000,
                value: 100,
            }],
            vec![LargeCommunity {
                global_admin: 65000,
                local_data1: 1,
                local_data2: 2,
            }],
        ));

        // Route 2: community 65000:200, origin Incomplete, med 150, localpref 100
        rib.insert(make_route_with_attrs(
            "10.0.1.0/24",
            "10.0.0.1",
            vec![65001, 65020],
            Origin::Incomplete,
            Some(150),
            Some(100),
            vec![Community {
                asn: 65000,
                value: 200,
            }],
            vec![],
        ));

        // Route 3: no community, origin IGP, no med, localpref 100
        rib.insert(make_route_with_attrs(
            "10.0.2.0/24",
            "10.0.0.1",
            vec![65001, 65030],
            Origin::Igp,
            None,
            Some(100),
            vec![],
            vec![],
        ));
    }

    let bgp_config = BgpConfig::default();
    AppState::new(
        rib_store,
        peers,
        bgp_config,
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
    )
}

#[tokio::test]
async fn get_peer_routes_search_by_community() {
    let app = build_router(test_state_with_attrs());
    let (status, body) = get(
        app,
        "/api/v1/peers/Router%20A/routes/ipv4?search=community:65000:100",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.0.0/24");
}

#[tokio::test]
async fn get_peer_routes_search_by_origin() {
    let app = build_router(test_state_with_attrs());
    let (status, body) = get(
        app,
        "/api/v1/peers/Router%20A/routes/ipv4?search=origin:incomplete",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.1.0/24");
}

#[tokio::test]
async fn get_peer_routes_search_by_med() {
    let app = build_router(test_state_with_attrs());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4?search=med:%3E100").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.1.0/24");
}

#[tokio::test]
async fn get_peer_routes_search_by_localpref() {
    let app = build_router(test_state_with_attrs());
    let (status, body) = get(
        app,
        "/api/v1/peers/Router%20A/routes/ipv4?search=localpref:%3E%3D200",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.0.0/24");
}

#[tokio::test]
async fn get_peer_routes_search_combined_filters() {
    let app = build_router(test_state_with_attrs());
    // origin:igp AND localpref:>=200 → only 10.0.0.0/24
    let (status, body) = get(
        app,
        "/api/v1/peers/Router%20A/routes/ipv4?search=origin:igp%20localpref:%3E%3D200",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0]["prefix"], "10.0.0.0/24");
}

// --- Route Refresh ---

#[tokio::test]
async fn refresh_peer_no_session_returns_404() {
    let app = build_router(test_state());
    // Peer 10.0.0.2 exists but has no command channel (no active session in tests)
    let (status, body) = post(app, "/api/v1/peers/Router%20A/refresh").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json["message"]
            .as_str()
            .unwrap()
            .contains("no active session")
    );
}

#[tokio::test]
async fn refresh_unknown_peer_returns_404() {
    let app = build_router(test_state());
    let (status, _) = post(app, "/api/v1/peers/nonexistent/refresh").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn refresh_peer_with_channel_returns_200() {
    let rib_store = RibStore::new();
    let peers_map: HashMap<String, Arc<RwLock<PeerInfo>>> = HashMap::new();
    let peers = Arc::new(RwLock::new(peers_map));

    let mut peer_a = PeerInfo::new(
        "10.0.0.2".parse().unwrap(),
        65001,
        "Router A".into(),
        String::new(),
        None,
        vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast],
    );
    peer_a.state = PeerState::Established;
    {
        let mut p = peers.write().unwrap();
        p.insert(peer_a.id.clone(), Arc::new(RwLock::new(peer_a)));
    }

    // Create a command channel and store the sender
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let command_channels: Arc<
        RwLock<HashMap<String, tokio::sync::mpsc::Sender<reflet_bgp::refresh::SessionCommand>>>,
    > = Arc::new(RwLock::new(HashMap::new()));
    {
        let mut channels = command_channels.write().unwrap();
        channels.insert("10.0.0.2".to_string(), tx);
    }

    let bgp_config = BgpConfig::default();
    let state = AppState::new(
        rib_store,
        peers,
        bgp_config,
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        command_channels,
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
    );
    let app = build_router(state);

    let (status, body) = post(app, "/api/v1/peers/Router%20A/refresh").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json["message"]
            .as_str()
            .unwrap()
            .contains("route refresh requested")
    );

    // Verify the command was actually sent
    let cmd = rx.try_recv();
    assert!(cmd.is_ok());
}

// --- Events ---

fn test_state_with_events(event_log: EventLog) -> AppState {
    let rib_store = RibStore::new();
    let peers = Arc::new(RwLock::new(HashMap::new()));
    let bgp_config = BgpConfig::default();
    AppState::new(
        rib_store,
        peers,
        bgp_config,
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        event_log,
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
    )
}

#[tokio::test]
async fn get_events_empty() {
    let state = test_state_with_events(EventLog::new(100, None).unwrap());
    let app = build_router(state);
    let (status, body) = get(app, "/api/v1/events").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["events"].as_array().unwrap().len(), 0);
    assert_eq!(json["current_seq"], 0);
    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn get_events_with_data() {
    let event_log = EventLog::new(100, None).unwrap();
    event_log.push_announce(
        "10.0.0.1".into(),
        "192.168.0.0/24".parse().unwrap(),
        None,
        vec![65001, 65010],
        "10.0.0.1".parse().unwrap(),
        Some(65010),
    );
    event_log.push_withdraw("10.0.0.1".into(), "10.0.0.0/24".parse().unwrap(), None);
    event_log.push_session_up("10.0.0.2".into(), 65002);

    let state = test_state_with_events(event_log);
    let app = build_router(state);
    let (status, body) = get(app, "/api/v1/events").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["count"], 3);
    assert_eq!(json["current_seq"], 3);
    let events = json["events"].as_array().unwrap();
    assert_eq!(events[0]["type"], "announce");
    assert_eq!(events[1]["type"], "withdraw");
    assert_eq!(events[2]["type"], "session_up");
}

#[tokio::test]
async fn get_events_filtered_by_peer() {
    let event_log = EventLog::new(100, None).unwrap();
    event_log.push_session_up("10.0.0.1".into(), 65001);
    event_log.push_session_up("10.0.0.2".into(), 65002);
    event_log.push_session_up("10.0.0.1".into(), 65001);

    let state = test_state_with_events(event_log);
    let app = build_router(state);
    let (status, body) = get(app, "/api/v1/events?peer_id=10.0.0.1").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["count"], 2);
}

#[tokio::test]
async fn get_events_since_seq() {
    let event_log = EventLog::new(100, None).unwrap();
    for i in 0..5 {
        event_log.push_session_up(format!("10.0.0.{}", i), 65000 + i);
    }

    let state = test_state_with_events(event_log);
    let app = build_router(state);
    let (status, body) = get(app, "/api/v1/events?since_seq=3").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["count"], 2);
    let events = json["events"].as_array().unwrap();
    assert_eq!(events[0]["seq"], 4);
    assert_eq!(events[1]["seq"], 5);
}

// --- Address family config ---

#[tokio::test]
async fn peer_info_includes_families() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let families = json["families"].as_array().unwrap();
    assert_eq!(families.len(), 2);
    assert!(families.contains(&serde_json::json!("ipv4-unicast")));
    assert!(families.contains(&serde_json::json!("ipv6-unicast")));
}

#[tokio::test]
async fn peer_ipv4_only_families() {
    let rib_store = RibStore::new();
    let peers = Arc::new(RwLock::new(HashMap::new()));

    let mut peer = PeerInfo::new(
        "10.0.0.2".parse().unwrap(),
        65001,
        "IPv4-only peer".into(),
        String::new(),
        None,
        vec![AddressFamily::Ipv4Unicast],
    );
    peer.state = PeerState::Established;
    peer.prefixes = PrefixCounts { ipv4: 1, ipv6: 0 };

    {
        let mut p = peers.write().unwrap();
        p.insert(peer.id.clone(), Arc::new(RwLock::new(peer.clone())));
    }

    {
        let rib_arc = rib_store.get_or_create(&peer.id);
        let mut rib = rib_arc.write().unwrap();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65001, 65010]));
    }

    let bgp_config = BgpConfig::default();
    let state = AppState::new(
        rib_store,
        peers,
        bgp_config,
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
    );
    let app = build_router(state);

    // Check that families only contains ipv4-unicast
    let (status, body) = get(app, "/api/v1/peers/IPv4-only%20peer").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let families = json["families"].as_array().unwrap();
    assert_eq!(families.len(), 1);
    assert_eq!(families[0], "ipv4-unicast");
}

// --- SSE event stream ---

#[tokio::test]
async fn event_stream_returns_sse_content_type() {
    let state = test_state_with_events(EventLog::new(100, None).unwrap());
    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/events/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("text/event-stream"),
        "expected text/event-stream, got {content_type}"
    );
}

// --- RPKI ---

#[tokio::test]
async fn routes_with_rpki_store_include_status() {
    use ipnet::IpNet;

    let rib_store = RibStore::new();
    let peers = Arc::new(RwLock::new(HashMap::new()));

    let mut peer_a = PeerInfo::new(
        "10.0.0.2".parse().unwrap(),
        65001,
        "Router A".into(),
        String::new(),
        None,
        vec![AddressFamily::Ipv4Unicast],
    );
    peer_a.state = PeerState::Established;
    peer_a.prefixes = PrefixCounts { ipv4: 1, ipv6: 0 };

    {
        let mut p = peers.write().unwrap();
        p.insert(peer_a.id.clone(), Arc::new(RwLock::new(peer_a.clone())));
    }

    {
        let rib_arc = rib_store.get_or_create(&peer_a.id);
        let mut rib = rib_arc.write().unwrap();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65001, 65010]));
    }

    // Create RPKI store with a matching VRP
    let rpki_store =
        RpkiStore::from_vrps(vec![("10.0.0.0/24".parse::<IpNet>().unwrap(), 65010, 24)]);

    let state = AppState::new(
        rib_store,
        peers,
        BgpConfig::default(),
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(rpki_store)),
    );
    let app = build_router(state);

    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes[0]["rpki_status"], "valid");
}

#[tokio::test]
async fn routes_without_rpki_store_omit_status() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    // rpki_status should be absent (None is skipped in serialization)
    assert!(routes[0].get("rpki_status").is_none());
}

#[tokio::test]
async fn summary_with_rpki_includes_vrp_count() {
    use ipnet::IpNet;

    let rpki_store = RpkiStore::from_vrps(vec![
        ("10.0.0.0/24".parse::<IpNet>().unwrap(), 65010, 24),
        ("2001:db8::/32".parse::<IpNet>().unwrap(), 65010, 48),
    ]);

    let state = AppState::new(
        RibStore::new(),
        Arc::new(RwLock::new(HashMap::new())),
        BgpConfig::default(),
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(rpki_store)),
    );
    let app = build_router(state);

    let (status, body) = get(app, "/api/v1/summary").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["rpki"]["vrp_count"], 2);
}

#[tokio::test]
async fn summary_without_rpki_omits_field() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/summary").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json.get("rpki").is_none());
}
