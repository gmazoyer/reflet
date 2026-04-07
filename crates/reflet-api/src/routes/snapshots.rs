use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use reflet_core::rib_snapshots::{self, SnapshotMeta};

use super::routes::PaginationParams;
use crate::error::ApiError;
use crate::routes::routes::{PaginatedRoutes, PaginationMeta};
use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct SnapshotListResponse {
    pub snapshots: Vec<SnapshotMeta>,
}

#[derive(Deserialize)]
pub struct SnapshotPath {
    pub id: String,
    pub timestamp: String,
}

fn snapshot_data_dir(state: &AppState) -> Result<&str, ApiError> {
    state
        .snapshot_data_dir
        .as_deref()
        .ok_or_else(|| ApiError::NotFound("snapshots are not configured".to_string()))
}

fn parse_snapshot_timestamp(ts: &str) -> Result<DateTime<Utc>, ApiError> {
    // Accept both filesystem-safe format (2026-03-26T14-00-00Z) and RFC 3339
    chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%dT%H-%M-%SZ")
        .map(|naive| naive.and_utc())
        .or_else(|_| DateTime::parse_from_rfc3339(ts).map(|dt| dt.with_timezone(&Utc)))
        .map_err(|_| ApiError::BadRequest(format!("invalid timestamp: {ts}")))
}

/// GET /api/v1/peers/:id/snapshots
#[utoipa::path(
    get,
    path = "/api/v1/peers/{id}/snapshots",
    params(
        ("id" = String, Path, description = "Peer name"),
    ),
    responses(
        (status = 200, description = "Available snapshots for peer", body = SnapshotListResponse),
        (status = 404, description = "Peer not found or snapshots not configured")
    ),
    tag = "snapshots"
)]
pub async fn list_snapshots(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<SnapshotListResponse>, ApiError> {
    let id = state.resolve_name(&name)?;
    state.peer_or_404(&id)?;
    let data_dir = snapshot_data_dir(&state)?;

    let snapshots = tokio::task::spawn_blocking({
        let data_dir = data_dir.to_string();
        let id = id.clone();
        move || rib_snapshots::list_snapshots(&data_dir, &id)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("snapshot task failed: {e}")))?
    .map_err(|e| ApiError::Internal(format!("failed to list snapshots: {e}")))?;

    Ok(Json(SnapshotListResponse { snapshots }))
}

/// Shared implementation for browsing snapshot routes.
async fn get_snapshot_routes(
    state: &AppState,
    name: &str,
    timestamp_str: &str,
    params: &PaginationParams,
    ipv4: bool,
) -> Result<(HeaderMap, Json<PaginatedRoutes>), ApiError> {
    let id = state.resolve_name(name)?;
    state.peer_or_404(&id)?;
    let data_dir = snapshot_data_dir(state)?.to_string();
    let timestamp = parse_snapshot_timestamp(timestamp_str)?;

    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(100).clamp(1, 10000);
    let search = params.search.clone();

    let (data, total) = tokio::task::spawn_blocking({
        let id = id.clone();
        let timestamp_str = timestamp_str.to_string();
        move || -> Result<(Vec<_>, usize), ApiError> {
            let rib = rib_snapshots::load_snapshot(&data_dir, &id, &timestamp).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    ApiError::NotFound(format!("snapshot not found: {timestamp_str}"))
                } else {
                    ApiError::Internal(format!("failed to load snapshot: {e}"))
                }
            })?;

            Ok(if ipv4 {
                rib.paginate_ipv4(page, per_page, search.as_deref())
            } else {
                rib.paginate_ipv6(page, per_page, search.as_deref())
            })
        }
    })
    .await
    .map_err(|e| ApiError::Internal(format!("snapshot task failed: {e}")))??;

    let data = data
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

    Ok((headers, Json(response)))
}

/// GET /api/v1/peers/:id/snapshots/:timestamp/routes/ipv4
#[utoipa::path(
    get,
    path = "/api/v1/peers/{id}/snapshots/{timestamp}/routes/ipv4",
    params(
        ("id" = String, Path, description = "Peer name"),
        ("timestamp" = String, Path, description = "Snapshot timestamp"),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "IPv4 routes from snapshot", body = PaginatedRoutes),
        (status = 404, description = "Peer or snapshot not found")
    ),
    tag = "snapshots"
)]
pub async fn get_snapshot_ipv4_routes(
    State(state): State<AppState>,
    Path(SnapshotPath {
        id: name,
        timestamp,
    }): Path<SnapshotPath>,
    Query(params): Query<PaginationParams>,
) -> Result<(HeaderMap, Json<PaginatedRoutes>), ApiError> {
    get_snapshot_routes(&state, &name, &timestamp, &params, true).await
}

/// GET /api/v1/peers/:id/snapshots/:timestamp/routes/ipv6
#[utoipa::path(
    get,
    path = "/api/v1/peers/{id}/snapshots/{timestamp}/routes/ipv6",
    params(
        ("id" = String, Path, description = "Peer name"),
        ("timestamp" = String, Path, description = "Snapshot timestamp"),
        PaginationParams,
    ),
    responses(
        (status = 200, description = "IPv6 routes from snapshot", body = PaginatedRoutes),
        (status = 404, description = "Peer or snapshot not found")
    ),
    tag = "snapshots"
)]
pub async fn get_snapshot_ipv6_routes(
    State(state): State<AppState>,
    Path(SnapshotPath {
        id: name,
        timestamp,
    }): Path<SnapshotPath>,
    Query(params): Query<PaginationParams>,
) -> Result<(HeaderMap, Json<PaginatedRoutes>), ApiError> {
    get_snapshot_routes(&state, &name, &timestamp, &params, false).await
}
