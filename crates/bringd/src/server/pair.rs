//! Public pairing endpoints (authenticated by SPAKE2, not by a token).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bring_core::api::{PairConfirmRequest, PairExchangeRequest, PairExchangeResponse, PairPending};
use bring_core::pairing::PairingId;

use super::{ApiError, AppState};

fn session_id(s: &str) -> Result<PairingId, ApiError> {
    PairingId::from_hex(s).ok_or_else(|| ApiError::not_found("no such pairing session"))
}

fn pending() -> Response {
    (
        StatusCode::ACCEPTED,
        Json(PairPending {
            status: "awaiting_user".into(),
        }),
    )
        .into_response()
}

/// `POST /pair/{pairing_id}/exchange`
pub async fn exchange(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PairExchangeRequest>,
) -> Result<Json<PairExchangeResponse>, ApiError> {
    let id = session_id(&id)?;
    // SPAKE2 is fast; no need for the blocking pool.
    Ok(Json(state.pairing.exchange(&id, &req)?))
}

/// `POST /pair/{pairing_id}/confirm`
pub async fn confirm(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PairConfirmRequest>,
) -> Result<Response, ApiError> {
    let id = session_id(&id)?;
    state.pairing.confirm(&id, &req)?;
    Ok(pending())
}

/// `GET /pair/{pairing_id}/result`
pub async fn result(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = session_id(&id)?;
    Ok(match state.pairing.result(&id)? {
        Some(sealed) => Json(sealed).into_response(),
        None => pending(),
    })
}
