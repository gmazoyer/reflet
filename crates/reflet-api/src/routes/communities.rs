use axum::Json;
use axum::extract::State;

use reflet_core::community::CommunityDefinitions;

use crate::state::AppState;

/// Get community definitions for annotation.
#[utoipa::path(
    get,
    path = "/api/v1/communities/definitions",
    tag = "communities",
    responses(
        (status = 200, description = "Community definitions", body = CommunityDefinitions),
    )
)]
pub async fn get_definitions(State(state): State<AppState>) -> Json<CommunityDefinitions> {
    Json(state.community_store.read().unwrap().definitions().clone())
}
