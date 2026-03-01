use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;

use reflet_core::prefix::Prefix;

use crate::error::ApiError;
use crate::state::AppState;

/// JSend response format per RFC 8522.
#[derive(Serialize)]
pub struct JSendResponse<T: Serialize> {
    pub status: String,
    pub data: T,
}

fn jsend_success<T: Serialize>(data: T) -> Json<JSendResponse<T>> {
    Json(JSendResponse {
        status: "success".to_string(),
        data,
    })
}

// --- Router (peer) types ---

#[derive(Serialize)]
pub struct RouterSummary {
    pub id: String,
    pub name: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct RouterDetail {
    pub id: String,
    pub name: String,
    pub status: String,
    pub remote_asn: u32,
    pub router_id: String,
    pub prefixes_ipv4: usize,
    pub prefixes_ipv6: usize,
}

/// GET /.well-known/looking-glass/v1/routers
pub async fn list_routers(
    State(state): State<AppState>,
) -> Json<JSendResponse<Vec<RouterSummary>>> {
    let peers = state.peer_infos();
    let routers: Vec<RouterSummary> = peers
        .into_iter()
        .map(|p| RouterSummary {
            id: p.id,
            name: p.name,
            status: p.state.to_string(),
        })
        .collect();
    jsend_success(routers)
}

/// GET /.well-known/looking-glass/v1/routers/:id
pub async fn get_router(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<JSendResponse<RouterDetail>>, ApiError> {
    let id = state.resolve_name(&name)?;
    let peer = state
        .peer_info(&id)
        .ok_or_else(|| ApiError::NotFound(format!("router {name} not found")))?;

    Ok(jsend_success(RouterDetail {
        id: peer.id,
        name: peer.name,
        status: peer.state.to_string(),
        remote_asn: peer.remote_asn,
        router_id: if *state.hide_peer_addresses.read().unwrap() {
            "hidden".to_string()
        } else {
            peer.router_id.to_string()
        },
        prefixes_ipv4: peer.prefixes.ipv4,
        prefixes_ipv6: peer.prefixes.ipv6,
    }))
}

/// GET /.well-known/looking-glass/v1/cmd
pub async fn available_commands() -> Json<JSendResponse<Vec<String>>> {
    jsend_success(vec![
        "show route".to_string(),
        "show bgp".to_string(),
        "show bgp summary".to_string(),
    ])
}

#[derive(Serialize)]
pub struct BgpSummaryEntry {
    pub peer: String,
    pub remote_as: u32,
    pub state: String,
    pub prefixes_received: usize,
    pub uptime: Option<String>,
}

/// GET /.well-known/looking-glass/v1/show/bgp/summary
pub async fn bgp_summary(
    State(state): State<AppState>,
) -> Json<JSendResponse<Vec<BgpSummaryEntry>>> {
    let peers = state.peer_infos();
    let entries: Vec<BgpSummaryEntry> = peers
        .into_iter()
        .map(|p| BgpSummaryEntry {
            peer: p.id,
            remote_as: p.remote_asn,
            state: p.state.to_string(),
            prefixes_received: p.prefixes.total(),
            uptime: p.uptime.map(|u| u.to_rfc3339()),
        })
        .collect();
    jsend_success(entries)
}

#[derive(Serialize)]
pub struct RouteLookupEntry {
    pub peer_id: String,
    pub prefix: String,
    pub next_hop: String,
    pub as_path: Vec<u32>,
    pub origin: String,
}

/// GET /.well-known/looking-glass/v1/show/route/:prefix
/// GET /.well-known/looking-glass/v1/show/bgp/:prefix
pub async fn show_route(
    State(state): State<AppState>,
    Path(prefix_str): Path<String>,
) -> Result<Json<JSendResponse<Vec<RouteLookupEntry>>>, ApiError> {
    let prefix: Prefix = prefix_str
        .parse()
        .map_err(|e| ApiError::BadRequest(format!("invalid prefix: {e}")))?;

    let matches = state.rib_store.lookup_exact(&prefix);
    let entries: Vec<RouteLookupEntry> = matches
        .into_iter()
        .flat_map(|(peer_id, routes)| {
            let state = &state;
            routes.into_iter().map(move |route| {
                let route = state.sanitize_route(route);
                RouteLookupEntry {
                    peer_id: peer_id.clone(),
                    prefix: route.prefix.to_string(),
                    next_hop: route.next_hop.to_string(),
                    as_path: route.as_path_flat(),
                    origin: route.origin.to_string(),
                }
            })
        })
        .collect();

    Ok(jsend_success(entries))
}
