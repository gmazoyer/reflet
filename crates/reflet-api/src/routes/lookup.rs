use axum::Json;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use reflet_core::peer::PeerId;
use reflet_core::prefix::Prefix;
use reflet_core::route::BgpRoute;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize, IntoParams)]
pub struct LookupParams {
    /// Prefix to look up (e.g., "10.0.0.0/24" or "2001:db8::/32")
    pub prefix: String,
    /// Lookup type: "exact", "longest-match", or "subnets"
    #[param(default = "longest-match")]
    pub r#type: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct LookupResult {
    pub peer_id: PeerId,
    pub peer_name: String,
    #[schema(value_type = String)]
    pub matched_prefix: Option<String>,
    pub routes: Vec<BgpRoute>,
}

#[derive(Serialize, ToSchema)]
pub struct LookupResponse {
    pub query: String,
    pub lookup_type: String,
    pub results: Vec<LookupResult>,
}

/// GET /api/v1/lookup
#[utoipa::path(
    get,
    path = "/api/v1/lookup",
    params(LookupParams),
    responses(
        (status = 200, description = "Lookup results", body = LookupResponse),
        (status = 400, description = "Invalid prefix")
    ),
    tag = "lookup"
)]
pub async fn lookup(
    State(state): State<AppState>,
    Query(params): Query<LookupParams>,
) -> Result<Json<LookupResponse>, ApiError> {
    let lookup_type = params.r#type.as_deref().unwrap_or("longest-match");

    let prefix: Prefix = params
        .prefix
        .parse()
        .map_err(|e| ApiError::BadRequest(format!("invalid prefix: {e}")))?;

    let peer_name = |id: &str| -> String {
        state
            .peer_info(id)
            .map(|p| p.name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_string())
    };

    let results = match lookup_type {
        "exact" => {
            let matches = state.rib_store.lookup_exact(&prefix);
            matches
                .into_iter()
                .map(|(peer_id, routes)| {
                    let matched_prefix = routes.first().map(|r| r.prefix.to_string());
                    let name = peer_name(&peer_id);
                    LookupResult {
                        peer_id,
                        peer_name: name,
                        matched_prefix,
                        routes: routes
                            .into_iter()
                            .map(|r| state.annotate_route(state.sanitize_route(r)))
                            .collect(),
                    }
                })
                .collect()
        }
        "longest-match" => match &prefix {
            Prefix::V4(net) => {
                let matches = state.rib_store.lookup_lpm_v4(net);
                matches
                    .into_iter()
                    .map(|(peer_id, matched, routes)| {
                        let name = peer_name(&peer_id);
                        LookupResult {
                            peer_id,
                            peer_name: name,
                            matched_prefix: Some(matched.to_string()),
                            routes: routes
                                .into_iter()
                                .map(|r| state.annotate_route(state.sanitize_route(r)))
                                .collect(),
                        }
                    })
                    .collect()
            }
            Prefix::V6(net) => {
                let matches = state.rib_store.lookup_lpm_v6(net);
                matches
                    .into_iter()
                    .map(|(peer_id, matched, routes)| {
                        let name = peer_name(&peer_id);
                        LookupResult {
                            peer_id,
                            peer_name: name,
                            matched_prefix: Some(matched.to_string()),
                            routes: routes
                                .into_iter()
                                .map(|r| state.annotate_route(state.sanitize_route(r)))
                                .collect(),
                        }
                    })
                    .collect()
            }
        },
        "subnets" => match &prefix {
            Prefix::V4(net) => {
                let matches = state.rib_store.lookup_subnets_v4(net);
                matches
                    .into_iter()
                    .map(|(peer_id, routes)| {
                        let matched_prefix = routes.first().map(|r| r.prefix.to_string());
                        let name = peer_name(&peer_id);
                        LookupResult {
                            peer_id,
                            peer_name: name,
                            matched_prefix,
                            routes: routes
                                .into_iter()
                                .map(|r| state.annotate_route(state.sanitize_route(r)))
                                .collect(),
                        }
                    })
                    .collect()
            }
            Prefix::V6(net) => {
                let matches = state.rib_store.lookup_subnets_v6(net);
                matches
                    .into_iter()
                    .map(|(peer_id, routes)| {
                        let matched_prefix = routes.first().map(|r| r.prefix.to_string());
                        let name = peer_name(&peer_id);
                        LookupResult {
                            peer_id,
                            peer_name: name,
                            matched_prefix,
                            routes: routes
                                .into_iter()
                                .map(|r| state.annotate_route(state.sanitize_route(r)))
                                .collect(),
                        }
                    })
                    .collect()
            }
        },
        _ => {
            return Err(ApiError::BadRequest(format!(
                "invalid lookup type: {lookup_type}. Use 'exact', 'longest-match', or 'subnets'"
            )));
        }
    };

    Ok(Json(LookupResponse {
        query: params.prefix,
        lookup_type: lookup_type.to_string(),
        results,
    }))
}
