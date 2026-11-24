use axum::Json;
use axum::extract::State;
use bring_core::api::HealthResponse;

use super::AppState;

/// `GET /health` — unauthenticated LAN probe.
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ring_id: state.ring_id.clone(),
        version: bring_core::VERSION.to_owned(),
        protocol: bring_core::PROTOCOL_VERSION,
    })
}
