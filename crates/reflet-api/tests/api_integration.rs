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
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
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
        next_hop_link_local: None,
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
    assert_eq!(json["id"], "10.0.0.2");
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
    let (status, body) = get(
        app,
        "/api/v1/peers/Router%20A/routes/ipv4?page=1&per_page=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    assert_eq!(routes.len(), 1);
    assert_eq!(json["meta"]["total"], 2);
    assert_eq!(json["meta"]["per_page"], 1);
}

/// Add an IPv4 route with an IPv6 global and link-local next hop (RFC 8950)
/// to peer A.
fn test_state_with_ipv6_next_hop() -> AppState {
    let state = test_state();
    let rib_arc = state.rib_store.get_or_create("10.0.0.2");
    let mut route = make_route("198.51.100.0/24", "2001:db8::1", vec![65001, 65040]);
    route.next_hop_link_local = Some("fe80::1".parse().unwrap());
    rib_arc.write().unwrap().insert(route);
    state
}

#[tokio::test]
async fn get_peer_routes_ipv6_next_hop_for_ipv4_route() {
    let app = build_router(test_state_with_ipv6_next_hop());
    let (status, body) = get(app, "/api/v1/peers/Router%20A/routes/ipv4").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["data"].as_array().unwrap();
    let route = routes
        .iter()
        .find(|r| r["prefix"] == "198.51.100.0/24")
        .unwrap();
    assert_eq!(route["next_hop"], "2001:db8::1");
    assert_eq!(route["next_hop_link_local"], "fe80::1");

    // Routes without a link-local next hop do not have the field.
    let other = routes
        .iter()
        .find(|r| r["prefix"] == "10.0.0.0/24")
        .unwrap();
    assert!(other.get("next_hop_link_local").is_none());
}

#[tokio::test]
async fn get_peer_routes_search_by_next_hop() {
    for (search, expected) in [
        ("nexthop:2001:db8::1", vec!["198.51.100.0/24"]),
        ("nexthop:fe80::1", vec!["198.51.100.0/24"]),
        ("nexthop:10.0.0.1", vec!["10.0.0.0/24", "192.168.1.0/24"]),
    ] {
        let app = build_router(test_state_with_ipv6_next_hop());
        let uri = format!("/api/v1/peers/Router%20A/routes/ipv4?search={search}");
        let (status, body) = get(app, &uri).await;
        assert_eq!(status, StatusCode::OK, "{search}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        let prefixes: Vec<&str> = json["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["prefix"].as_str().unwrap())
            .collect();
        assert_eq!(prefixes, expected, "{search}");
    }
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
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
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
        next_hop_link_local: None,
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
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
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
    let (status, body) = get(
        app,
        "/api/v1/peers/Router%20A/routes/ipv4?search=med:%3E100",
    )
    .await;
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
        Arc::new(RwLock::new(false)),
        command_channels,
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
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
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        event_log,
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
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
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
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
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(rpki_store)),
        None,
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
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(rpki_store)),
        None,
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

// --- Route Refresh Disabled ---

#[tokio::test]
async fn refresh_peer_disabled_returns_403() {
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
    {
        let mut p = peers.write().unwrap();
        p.insert(peer_a.id.clone(), Arc::new(RwLock::new(peer_a)));
    }

    let (tx, _rx) = tokio::sync::mpsc::channel(4);
    let command_channels: Arc<
        RwLock<HashMap<String, tokio::sync::mpsc::Sender<reflet_bgp::refresh::SessionCommand>>>,
    > = Arc::new(RwLock::new(HashMap::new()));
    {
        let mut channels = command_channels.write().unwrap();
        channels.insert("10.0.0.2".to_string(), tx);
    }

    let state = AppState::new(
        rib_store,
        peers,
        BgpConfig::default(),
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(true)), // disable_route_refresh = true
        command_channels,
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
    );
    let app = build_router(state);

    let (status, body) = post(app, "/api/v1/peers/Router%20A/refresh").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(json["message"].as_str().unwrap().contains("disabled"));
}

#[tokio::test]
async fn summary_includes_route_refresh_enabled() {
    let app = build_router(test_state());
    let (status, body) = get(app, "/api/v1/summary").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["route_refresh_enabled"], true);
}

#[tokio::test]
async fn summary_route_refresh_disabled() {
    let state = AppState::new(
        RibStore::new(),
        Arc::new(RwLock::new(HashMap::new())),
        BgpConfig::default(),
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(true)), // disable_route_refresh = true
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        None,
    );
    let app = build_router(state);

    let (status, body) = get(app, "/api/v1/summary").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["route_refresh_enabled"], false);
}

// --- Snapshots ---

fn test_state_with_snapshots(data_dir: &str) -> AppState {
    let rib_store = RibStore::new();
    let peers_map: HashMap<String, Arc<RwLock<PeerInfo>>> = HashMap::new();
    let peers = Arc::new(RwLock::new(peers_map));

    // Create a peer
    let peer_a = PeerInfo {
        id: "10.0.0.1".to_string(),
        name: "Router A".to_string(),
        description: "Test router".to_string(),
        location: None,
        address: "10.0.0.1".parse().unwrap(),
        remote_asn: 65001,
        state: PeerState::Established,
        router_id: "10.0.0.1".parse().unwrap(),
        families: vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast],
        prefixes: PrefixCounts { ipv4: 1, ipv6: 0 },
        uptime: None,
        order: 0,
    };
    peers
        .write()
        .unwrap()
        .insert(peer_a.id.clone(), Arc::new(RwLock::new(peer_a.clone())));

    // Populate RIB so we can snapshot it
    {
        let rib_arc = rib_store.get_or_create(&peer_a.id);
        let mut rib = rib_arc.write().unwrap();
        rib.insert(make_route("10.0.0.0/24", "10.0.0.1", vec![65001, 65010]));
        rib.insert(make_route("192.168.0.0/16", "10.0.0.1", vec![65001]));
        rib.insert(make_route("2001:db8::/32", "::1", vec![65001, 65020]));
    }

    AppState::new(
        rib_store,
        peers,
        BgpConfig::default(),
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        EventLog::disabled(),
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        Some(data_dir.to_string()),
    )
}

#[tokio::test]
async fn list_snapshots_empty() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with_snapshots(dir.path().to_str().unwrap());
    let app = build_router(state);

    let (status, body) = get(app, "/api/v1/peers/Router%20A/snapshots").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["snapshots"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn list_snapshots_with_data() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let state = test_state_with_snapshots(data_dir);

    // Save a snapshot
    reflet_core::rib_snapshots::save_snapshot(&state.rib_store, "10.0.0.1", data_dir).unwrap();

    let app = build_router(state);
    let (status, body) = get(app, "/api/v1/peers/Router%20A/snapshots").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let snapshots = json["snapshots"].as_array().unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0]["route_count"], 3);
    assert_eq!(snapshots[0]["ipv4_count"], 2);
    assert_eq!(snapshots[0]["ipv6_count"], 1);
}

#[tokio::test]
async fn browse_snapshot_ipv4_routes() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let state = test_state_with_snapshots(data_dir);

    let meta = reflet_core::rib_snapshots::save_snapshot(&state.rib_store, "10.0.0.1", data_dir)
        .unwrap()
        .unwrap();

    let ts = meta.timestamp.format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let app = build_router(state);
    let url = format!("/api/v1/peers/Router%20A/snapshots/{ts}/routes/ipv4");
    let (status, body) = get(app, &url).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["meta"]["total"], 2);
    assert_eq!(json["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn browse_snapshot_ipv6_routes() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let state = test_state_with_snapshots(data_dir);

    let meta = reflet_core::rib_snapshots::save_snapshot(&state.rib_store, "10.0.0.1", data_dir)
        .unwrap()
        .unwrap();

    let ts = meta.timestamp.format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let app = build_router(state);
    let url = format!("/api/v1/peers/Router%20A/snapshots/{ts}/routes/ipv6");
    let (status, body) = get(app, &url).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["meta"]["total"], 1);
}

#[tokio::test]
async fn browse_snapshot_with_search() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let state = test_state_with_snapshots(data_dir);

    let meta = reflet_core::rib_snapshots::save_snapshot(&state.rib_store, "10.0.0.1", data_dir)
        .unwrap()
        .unwrap();

    let ts = meta.timestamp.format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let app = build_router(state);
    let url = format!("/api/v1/peers/Router%20A/snapshots/{ts}/routes/ipv4?search=AS65010");
    let (status, body) = get(app, &url).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    // Only the route with AS65010 in its path should match
    assert_eq!(json["meta"]["total"], 1);
}

#[tokio::test]
async fn snapshot_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let state = test_state_with_snapshots(dir.path().to_str().unwrap());
    let app = build_router(state);

    let (status, _) = get(
        app,
        "/api/v1/peers/Router%20A/snapshots/2099-01-01T00-00-00Z/routes/ipv4",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn snapshots_not_configured() {
    // State with snapshot_data_dir = None
    let state = test_state();
    let app = build_router(state);

    let (status, _) = get(app, "/api/v1/peers/Router%20A/snapshots").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- hide_peer_addresses on a link-local session ---

/// A peer on a link-local session (fe80::2), with `hide_peer_addresses` on.
/// Its link-local address is also the next hop of its routes.
fn test_state_link_local_peer(snapshot_data_dir: Option<String>) -> AppState {
    let rib_store = RibStore::new();
    let peers = Arc::new(RwLock::new(HashMap::new()));
    let mut peer = PeerInfo::new(
        "fe80::2".parse().unwrap(),
        65001,
        "Router LL".into(),
        String::new(),
        None,
        vec![AddressFamily::Ipv4Unicast, AddressFamily::Ipv6Unicast],
    );
    peer.state = PeerState::Established;
    peers
        .write()
        .unwrap()
        .insert(peer.id.clone(), Arc::new(RwLock::new(peer.clone())));

    {
        let rib_arc = rib_store.get_or_create(&peer.id);
        let mut rib = rib_arc.write().unwrap();
        let mut route = make_route("198.51.100.0/24", "2001:db8::1", vec![65001]);
        route.next_hop_link_local = Some("fe80::2".parse().unwrap());
        rib.insert(route);
        rib.insert(make_route("203.0.113.0/24", "192.0.2.1", vec![65001]));
        rib.insert(make_route("2001:db8::/32", "fe80::2", vec![65001]));
    }

    let event_log = EventLog::new(100, None).unwrap();
    event_log.push_announce(
        peer.id.clone(),
        "2001:db8::/32".parse().unwrap(),
        None,
        vec![65001],
        "fe80::2".parse().unwrap(),
        Some(65001),
    );
    event_log.push_announce(
        peer.id.clone(),
        "198.51.100.0/24".parse().unwrap(),
        None,
        vec![65001],
        "2001:db8::1".parse().unwrap(),
        Some(65001),
    );

    AppState::new(
        rib_store,
        peers,
        BgpConfig::default(),
        Arc::new(RwLock::new(CommunityStore::empty())),
        Arc::new(RwLock::new(AsnStore::empty())),
        Arc::new(RwLock::new("Reflet".into())),
        Arc::new(RwLock::new(true)),
        Arc::new(RwLock::new(false)),
        Arc::new(RwLock::new(HashMap::new())),
        event_log,
        test_notify(),
        Arc::new(RwLock::new(RpkiStore::empty())),
        snapshot_data_dir,
    )
}

/// Return `prefix => (next_hop, next_hop_link_local)` for the routes in a
/// paginated response.
fn next_hops_by_prefix(body: &str) -> HashMap<String, (String, Option<String>)> {
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["prefix"].as_str().unwrap().to_string(),
                (
                    r["next_hop"].as_str().unwrap().to_string(),
                    r["next_hop_link_local"].as_str().map(String::from),
                ),
            )
        })
        .collect()
}

/// Read `count` events from an SSE stream.
async fn sse_events(app: axum::Router, uri: &str, count: usize) -> Vec<serde_json::Value> {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let mut body = response.into_body();
    let mut stream = String::new();
    while stream.matches("data:").count() < count {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("SSE event")
            .unwrap()
            .unwrap();
        if let Some(data) = frame.data_ref() {
            stream.push_str(std::str::from_utf8(data).unwrap());
        }
    }
    stream
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str(data.trim()).unwrap())
        .collect()
}

#[tokio::test]
async fn hide_peer_addresses_masks_link_local_next_hops_in_routes() {
    let state = test_state_link_local_peer(None);

    let (_, body) = get(build_router(state.clone()), "/api/v1/peers/Router%20LL").await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["address"], "::");

    let (status, body) = get(
        build_router(state.clone()),
        "/api/v1/peers/Router%20LL/routes/ipv4",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let routes = next_hops_by_prefix(&body);
    assert_eq!(
        routes["198.51.100.0/24"],
        ("2001:db8::1".into(), Some("::".into()))
    );
    assert_eq!(routes["203.0.113.0/24"], ("192.0.2.1".into(), None));

    let (_, body) = get(
        build_router(state.clone()),
        "/api/v1/peers/Router%20LL/routes/ipv6",
    )
    .await;
    let routes = next_hops_by_prefix(&body);
    assert_eq!(routes["2001:db8::/32"], ("::".into(), None));

    let (_, body) = get(
        build_router(state.clone()),
        "/api/v1/lookup?prefix=2001:db8::/32&type=exact",
    )
    .await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["results"][0]["routes"][0]["next_hop"], "::");

    let (_, body) = get(
        build_router(state),
        "/.well-known/looking-glass/v1/show/route/2001:db8::%2F32",
    )
    .await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["data"][0]["next_hop"], "::");
}

#[tokio::test]
async fn hide_peer_addresses_search_ignores_link_local_next_hops() {
    for (family, search, expected) in [
        ("ipv4", "nexthop:fe80::2", vec![]),
        ("ipv4", "nexthop:fe80::/10", vec![]),
        ("ipv4", "nexthop:2001:db8::1", vec!["198.51.100.0/24"]),
        ("ipv6", "nexthop:fe80::2", vec![]),
        ("ipv6", "nexthop:::/0", vec![]),
    ] {
        let app = build_router(test_state_link_local_peer(None));
        let uri = format!("/api/v1/peers/Router%20LL/routes/{family}?search={search}");
        let (status, body) = get(app, &uri).await;
        assert_eq!(status, StatusCode::OK, "{search}");
        let routes = next_hops_by_prefix(&body);
        let mut prefixes: Vec<&str> = routes.keys().map(String::as_str).collect();
        prefixes.sort();
        assert_eq!(prefixes, expected, "{family} {search}");
    }
}

#[tokio::test]
async fn hide_peer_addresses_masks_link_local_next_hops_in_events() {
    let state = test_state_link_local_peer(None);
    let (status, body) = get(build_router(state.clone()), "/api/v1/events").await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let events = json["events"].as_array().unwrap();
    assert_eq!(events[0]["next_hop"], "::");
    assert_eq!(events[1]["next_hop"], "2001:db8::1");

    // The SSE stream sends the two events at once, then waits for more.
    let events = sse_events(build_router(state), "/api/v1/events/stream", 2).await;
    let next_hops: Vec<&str> = events
        .iter()
        .map(|event| event["next_hop"].as_str().unwrap())
        .collect();
    assert_eq!(next_hops, ["::", "2001:db8::1"]);
}

#[tokio::test]
async fn hide_peer_addresses_masks_link_local_next_hops_in_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let state = test_state_link_local_peer(Some(data_dir.to_string()));
    let meta = reflet_core::rib_snapshots::save_snapshot(&state.rib_store, "fe80::2", data_dir)
        .unwrap()
        .unwrap();
    let ts = meta.timestamp.format("%Y-%m-%dT%H-%M-%SZ").to_string();

    let uri = format!("/api/v1/peers/Router%20LL/snapshots/{ts}/routes/ipv6");
    let (status, body) = get(build_router(state.clone()), &uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        next_hops_by_prefix(&body)["2001:db8::/32"],
        ("::".into(), None)
    );

    let uri =
        format!("/api/v1/peers/Router%20LL/snapshots/{ts}/routes/ipv4?search=nexthop:fe80::2");
    let (_, body) = get(build_router(state), &uri).await;
    assert!(next_hops_by_prefix(&body).is_empty(), "{body}");
}

// --- hide_peer_addresses and peer IDs ---

#[tokio::test]
async fn hide_peer_addresses_shows_peer_name_as_peer_id() {
    let state = test_state_link_local_peer(None);
    for (uri, id_field) in [
        ("/api/v1/peers", "/0/id"),
        ("/api/v1/peers/Router%20LL", "/id"),
        (
            "/api/v1/lookup?prefix=2001:db8::/32&type=exact",
            "/results/0/peer_id",
        ),
        (
            "/api/v1/lookup?prefix=198.51.100.1/32",
            "/results/0/peer_id",
        ),
        ("/api/v1/events", "/events/0/peer_id"),
        ("/.well-known/looking-glass/v1/routers", "/data/0/id"),
        (
            "/.well-known/looking-glass/v1/routers/Router%20LL",
            "/data/id",
        ),
        (
            "/.well-known/looking-glass/v1/show/bgp/summary",
            "/data/0/peer",
        ),
        (
            "/.well-known/looking-glass/v1/show/route/2001:db8::%2F32",
            "/data/0/peer_id",
        ),
    ] {
        let (status, body) = get(build_router(state.clone()), uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(!body.contains("fe80"), "{uri}: {body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json.pointer(id_field).unwrap(), "Router LL", "{uri}");
    }

    let (_, body) = get(build_router(state.clone()), "/metrics").await;
    assert!(!body.contains("fe80"), "{body}");
    assert!(body.contains(r#"refletpeer_up{peer="Router LL""#), "{body}");

    let events = sse_events(build_router(state.clone()), "/api/v1/events/stream", 2).await;
    assert!(
        events.iter().all(|event| event["peer_id"] == "Router LL"),
        "{events:?}"
    );

    // The peer has no session, so the route refresh fails.
    let (status, body) = post(build_router(state), "/api/v1/peers/Router%20LL/refresh").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("fe80"), "{body}");
    assert!(body.contains("Router LL"), "{body}");
}

#[tokio::test]
async fn hide_peer_addresses_shows_peer_name_in_snapshot_list() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_str().unwrap();
    let state = test_state_link_local_peer(Some(data_dir.to_string()));
    reflet_core::rib_snapshots::save_snapshot(&state.rib_store, "fe80::2", data_dir).unwrap();

    let (status, body) = get(build_router(state), "/api/v1/peers/Router%20LL/snapshots").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.contains("fe80"), "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["snapshots"][0]["peer_id"], "Router LL");
}

#[tokio::test]
async fn hide_peer_addresses_events_filter_takes_peer_name() {
    let state = test_state_link_local_peer(None);
    for (filter, count) in [("Router%20LL", 2), ("fe80::2", 0), ("Unknown", 0)] {
        let uri = format!("/api/v1/events?peer_id={filter}");
        let (status, body) = get(build_router(state.clone()), &uri).await;
        assert_eq!(status, StatusCode::OK, "{filter}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(json["count"], count, "{filter}");
    }

    let uri = "/api/v1/events/stream?peer_id=Router%20LL";
    assert_eq!(
        sse_events(build_router(state.clone()), uri, 2).await.len(),
        2
    );

    // A filter on the peer address must not select the events of the peer,
    // so the stream sends nothing.
    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v1/events/stream?peer_id=fe80::2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut body = response.into_body();
    let frame = tokio::time::timeout(std::time::Duration::from_millis(500), body.frame()).await;
    assert!(frame.is_err(), "the stream sent an event");
}

#[tokio::test]
async fn hide_peer_addresses_hides_ids_of_unconfigured_peers() {
    // A saved RIB and the event log can hold a peer that is no longer in the
    // configuration, so it has no name.
    let state = test_state_link_local_peer(None);
    state
        .rib_store
        .get_or_create("192.0.2.9")
        .write()
        .unwrap()
        .insert(make_route("192.0.2.128/25", "203.0.113.254", vec![65009]));
    state
        .event_log
        .push_session_down("192.0.2.9".into(), "peer removed".into());

    let (_, body) = get(
        build_router(state.clone()),
        "/api/v1/lookup?prefix=192.0.2.128/25&type=exact",
    )
    .await;
    assert!(!body.contains("192.0.2.9"), "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["results"][0]["peer_id"], "0.0.0.0");
    assert_eq!(json["results"][0]["peer_name"], "0.0.0.0");

    let (_, body) = get(build_router(state), "/api/v1/events?type=session_down").await;
    assert!(!body.contains("192.0.2.9"), "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["events"][0]["peer_id"], "0.0.0.0");
}
