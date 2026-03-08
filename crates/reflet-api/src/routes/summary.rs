use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct SummaryResponse {
    pub title: String,
    pub local_asn: u32,
    #[schema(value_type = String)]
    pub router_id: std::net::Ipv4Addr,
    pub peer_count: usize,
    pub established_peers: usize,
    pub total_ipv4_prefixes: usize,
    pub total_ipv6_prefixes: usize,
    pub route_refresh_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpki: Option<RpkiSummary>,
}

#[derive(Serialize, ToSchema)]
pub struct RpkiSummary {
    pub vrp_count: usize,
}

/// GET /api/v1/summary
#[utoipa::path(
    get,
    path = "/api/v1/summary",
    responses(
        (status = 200, description = "System summary", body = SummaryResponse)
    ),
    tag = "summary"
)]
pub async fn get_summary(State(state): State<AppState>) -> Json<SummaryResponse> {
    let (peer_count, established, total_v4, total_v6) = state.peer_stats();

    let rpki = {
        let store = state.rpki_store.read().unwrap();
        if store.is_empty() {
            None
        } else {
            Some(RpkiSummary {
                vrp_count: store.vrp_count(),
            })
        }
    };

    let route_refresh_enabled = !*state.disable_route_refresh.read().unwrap();

    Json(SummaryResponse {
        title: state.title.read().unwrap().clone(),
        local_asn: state.bgp_config.local_asn,
        router_id: state.bgp_config.router_id,
        peer_count,
        established_peers: established,
        total_ipv4_prefixes: total_v4,
        total_ipv6_prefixes: total_v6,
        route_refresh_enabled,
        rpki,
    })
}

/// GET /api/v1/health
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "Health check", body = HealthResponse)
    ),
    tag = "health"
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
}
