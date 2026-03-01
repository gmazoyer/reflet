use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use reflet_core::route::BgpRoute;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
pub struct PaginationParams {
    #[param(default = 1)]
    pub page: Option<usize>,
    #[param(default = 100)]
    pub per_page: Option<usize>,
    /// Filter routes using a search DSL. Supports: prefix substring (`10.0.0`),
    /// ASN (`AS65001`), AS path subsequence (`65000 65001`), community
    /// (`community:65000:100`, wildcards with `*`), large community
    /// (`lc:65000:1:2`), origin (`origin:igp`), MED (`med:>100`, `med:<=200`),
    /// and local-pref (`localpref:>=150`). Multiple filters are AND-combined.
    pub search: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PaginatedRoutes {
    pub data: Vec<BgpRoute>,
    pub meta: PaginationMeta,
}

#[derive(Serialize, ToSchema)]
pub struct PaginationMeta {
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

fn build_paginated_response(
    data: Vec<BgpRoute>,
    total: usize,
    page: usize,
    per_page: usize,
    state: &AppState,
) -> (PaginatedRoutes, HeaderMap) {
    let data: Vec<BgpRoute> = data
        .into_iter()
        .map(|r| state.annotate_route(state.sanitize_route(r)))
        .collect();
    let mut headers = HeaderMap::new();
    headers.insert("X-Total-Count", total.to_string().parse().unwrap());
    headers.insert("X-Page", page.to_string().parse().unwrap());
    headers.insert("X-Per-Page", per_page.to_string().parse().unwrap());

    let response = PaginatedRoutes {
        data,
        meta: PaginationMeta {
            total,
            page,
            per_page,
        },
    };

    (response, headers)
}

/// Shared implementation for both IPv4 and IPv6 route listing.
fn get_peer_routes(
    state: &AppState,
    id: &str,
    params: &PaginationParams,
    ipv4: bool,
) -> Result<(HeaderMap, Json<PaginatedRoutes>), ApiError> {
    state.peer_or_404(id)?;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(100).clamp(1, 10000);

    let (data, total) = if let Some(rib_arc) = state.rib_store.get(id) {
        let rib = rib_arc
            .read()
            .map_err(|e| ApiError::Internal(format!("failed to read RIB: {e}")))?;
        if ipv4 {
            rib.paginate_ipv4(page, per_page, params.search.as_deref())
        } else {
            rib.paginate_ipv6(page, per_page, params.search.as_deref())
        }
    } else {
        (Vec::new(), 0)
    };

    let (response, headers) = build_paginated_response(data, total, page, per_page, state);
    Ok((headers, Json(response)))
}

/// GET /api/v1/peers/:id/routes/ipv4
#[utoipa::path(
    get,
    path = "/api/v1/peers/{id}/routes/ipv4",
    params(
        ("id" = String, Path, description = "Peer identifier"),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "IPv4 routes from peer", body = PaginatedRoutes),
        (status = 404, description = "Peer not found")
    ),
    tag = "routes"
)]
pub async fn get_peer_ipv4_routes(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<(HeaderMap, Json<PaginatedRoutes>), ApiError> {
    let id = state.resolve_name(&name)?;
    get_peer_routes(&state, &id, &params, true)
}

/// GET /api/v1/peers/:id/routes/ipv6
#[utoipa::path(
    get,
    path = "/api/v1/peers/{id}/routes/ipv6",
    params(
        ("id" = String, Path, description = "Peer identifier"),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "IPv6 routes from peer", body = PaginatedRoutes),
        (status = 404, description = "Peer not found")
    ),
    tag = "routes"
)]
pub async fn get_peer_ipv6_routes(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<(HeaderMap, Json<PaginatedRoutes>), ApiError> {
    let id = state.resolve_name(&name)?;
    get_peer_routes(&state, &id, &params, false)
}
