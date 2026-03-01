use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use reflet_core::asn::AsnInfo;

use crate::state::AppState;

/// Get all ASN info entries for tooltip annotation.
#[utoipa::path(
    get,
    path = "/api/v1/asns",
    tag = "asns",
    responses(
        (status = 200, description = "ASN info map keyed by ASN number", body = HashMap<String, AsnInfo>),
    )
)]
pub async fn get_asns(State(state): State<AppState>) -> Json<Arc<HashMap<u32, AsnInfo>>> {
    Json(Arc::clone(state.asn_store.read().unwrap().as_map()))
}
