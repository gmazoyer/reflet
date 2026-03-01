use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use utoipa::ToSchema;

use reflet_core::peer::PeerInfo;

use crate::error::ApiError;
use crate::state::AppState;

/// Response for a route refresh request.
#[derive(Serialize, ToSchema)]
pub struct RouteRefreshResponse {
    pub message: String,
}

/// GET /api/v1/peers
#[utoipa::path(
    get,
    path = "/api/v1/peers",
    responses(
        (status = 200, description = "List of all peers", body = Vec<PeerInfo>)
    ),
    tag = "peers"
)]
pub async fn list_peers(State(state): State<AppState>) -> Json<Vec<PeerInfo>> {
    Json(state.peer_infos())
}

/// GET /api/v1/peers/:id
#[utoipa::path(
    get,
    path = "/api/v1/peers/{id}",
    params(
        ("id" = String, Path, description = "Peer name")
    ),
    responses(
        (status = 200, description = "Peer details", body = PeerInfo),
        (status = 404, description = "Peer not found")
    ),
    tag = "peers"
)]
pub async fn get_peer(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<PeerInfo>, ApiError> {
    let id = state.resolve_name(&name)?;
    state.peer_or_404(&id).map(Json)
}

/// POST /api/v1/peers/:id/refresh
#[utoipa::path(
    post,
    path = "/api/v1/peers/{id}/refresh",
    params(
        ("id" = String, Path, description = "Peer name")
    ),
    responses(
        (status = 200, description = "Route refresh requested", body = RouteRefreshResponse),
        (status = 404, description = "Peer not found or no active session"),
    ),
    tag = "peers"
)]
pub async fn refresh_peer(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<RouteRefreshResponse>, ApiError> {
    let id = state.resolve_name(&name)?;
    state.request_route_refresh(&id)?;
    Ok(Json(RouteRefreshResponse {
        message: format!("route refresh requested for peer {name}"),
    }))
}
