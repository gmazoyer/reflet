use axum::Router;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::rfc8522::handlers as rfc;
use crate::routes::{
    asns, communities, events, lookup, metrics, peers, routes, snapshots, summary, whoami,
};
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(
    paths(
        summary::get_summary,
        summary::health,
        peers::list_peers,
        peers::get_peer,
        peers::refresh_peer,
        routes::get_peer_ipv4_routes,
        routes::get_peer_ipv6_routes,
        lookup::lookup,
        events::get_events,
        events::event_stream,
        communities::get_definitions,
        asns::get_asns,
        whoami::whoami,
        snapshots::list_snapshots,
        snapshots::get_snapshot_ipv4_routes,
        snapshots::get_snapshot_ipv6_routes,
    ),
    components(schemas(
        reflet_core::peer::PeerInfo,
        reflet_core::peer::PeerState,
        reflet_core::peer::PrefixCounts,
        reflet_core::prefix::Prefix,
        reflet_core::prefix::AddressFamily,
        reflet_core::route::BgpRoute,
        reflet_core::route::Origin,
        reflet_core::route::AsPathSegment,
        reflet_core::route::Community,
        reflet_core::route::ExtCommunity,
        reflet_core::route::LargeCommunity,
        summary::SummaryResponse,
        summary::HealthResponse,
        routes::PaginatedRoutes,
        routes::PaginationMeta,
        lookup::LookupResponse,
        lookup::LookupResult,
        reflet_core::asn::AsnInfo,
        reflet_core::community::CommunityDefinitions,
        reflet_core::community::CommunityPattern,
        reflet_core::community::CommunityRange,
        reflet_core::community::SegmentMatcher,
        reflet_core::community::CommunityType,
        whoami::WhoamiResponse,
        peers::RouteRefreshResponse,
        reflet_core::event_log::RouteEvent,
        reflet_core::event_log::RouteEventType,
        events::EventsResponse,
        reflet_core::rpki::RpkiStatus,
        summary::RpkiSummary,
        reflet_core::rib_snapshots::SnapshotMeta,
        snapshots::SnapshotListResponse,
    )),
    tags(
        (name = "summary", description = "System summary and health"),
        (name = "peers", description = "BGP peer management"),
        (name = "routes", description = "BGP route tables"),
        (name = "lookup", description = "Prefix lookup"),
        (name = "communities", description = "Community definitions"),
        (name = "asns", description = "ASN information"),
        (name = "events", description = "Route change events"),
        (name = "snapshots", description = "Historical RIB snapshots"),
    )
)]
struct ApiDoc;

/// Build the complete Axum router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/api/v1/summary", axum::routing::get(summary::get_summary))
        .route("/api/v1/health", axum::routing::get(summary::health))
        .route("/api/v1/peers", axum::routing::get(peers::list_peers))
        .route("/api/v1/peers/{id}", axum::routing::get(peers::get_peer))
        .route(
            "/api/v1/peers/{id}/refresh",
            axum::routing::post(peers::refresh_peer),
        )
        .route(
            "/api/v1/peers/{id}/routes/ipv4",
            axum::routing::get(routes::get_peer_ipv4_routes),
        )
        .route(
            "/api/v1/peers/{id}/routes/ipv6",
            axum::routing::get(routes::get_peer_ipv6_routes),
        )
        .route("/api/v1/lookup", axum::routing::get(lookup::lookup))
        .route(
            "/api/v1/communities/definitions",
            axum::routing::get(communities::get_definitions),
        )
        .route("/api/v1/whoami", axum::routing::get(whoami::whoami))
        .route("/api/v1/asns", axum::routing::get(asns::get_asns))
        .route("/api/v1/events", axum::routing::get(events::get_events))
        .route(
            "/api/v1/peers/{id}/snapshots",
            axum::routing::get(snapshots::list_snapshots),
        )
        .route(
            "/api/v1/peers/{id}/snapshots/{timestamp}/routes/ipv4",
            axum::routing::get(snapshots::get_snapshot_ipv4_routes),
        )
        .route(
            "/api/v1/peers/{id}/snapshots/{timestamp}/routes/ipv6",
            axum::routing::get(snapshots::get_snapshot_ipv6_routes),
        );

    // RFC 8522 compatibility routes
    let rfc8522_routes = Router::new()
        .route(
            "/.well-known/looking-glass/v1/routers",
            axum::routing::get(rfc::list_routers),
        )
        .route(
            "/.well-known/looking-glass/v1/routers/{id}",
            axum::routing::get(rfc::get_router),
        )
        .route(
            "/.well-known/looking-glass/v1/cmd",
            axum::routing::get(rfc::available_commands),
        )
        .route(
            "/.well-known/looking-glass/v1/show/bgp/summary",
            axum::routing::get(rfc::bgp_summary),
        )
        .route(
            "/.well-known/looking-glass/v1/show/route/{prefix}",
            axum::routing::get(rfc::show_route),
        )
        .route(
            "/.well-known/looking-glass/v1/show/bgp/{prefix}",
            axum::routing::get(rfc::show_route),
        );

    // Compressed routes (all except SSE which must not be buffered)
    let compressed = Router::new()
        .merge(api_routes)
        .merge(rfc8522_routes)
        .route("/metrics", axum::routing::get(metrics::metrics))
        .merge(SwaggerUi::new("/docs").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .layer(CompressionLayer::new());

    // SSE route lives outside the compression layer to avoid buffering
    Router::new()
        .route(
            "/api/v1/events/stream",
            axum::routing::get(events::event_stream),
        )
        .merge(compressed)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
