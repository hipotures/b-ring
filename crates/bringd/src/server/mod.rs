//! HTTP API (`/api/v1`).

pub mod auth;
mod files;
mod health;
mod index;
mod pair;
mod thumbs;
mod uploads;

use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bring_core::api::{ErrorBody, ErrorCode};
use bring_core::transfer::MAX_CHUNK_SIZE;
use tracing::error;

use crate::hasher::HashQueue;
use crate::pairing::Pairing;
use crate::store::Db;
use crate::thumbs::ThumbStore;
use crate::uploads::Uploads;

/// State shared by all handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    /// Hex-encoded Ring identity.
    pub ring_id: String,
    pub thumbs: ThumbStore,
    pub uploads: Uploads,
    pub hashq: HashQueue,
    /// Key for signed stream URLs (`meta.stream_key`).
    pub stream_key: [u8; 32],
    pub pairing: Pairing,
}

/// Builds the API router.
pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/v1/health", get(health::health))
        // Bearer *or* signed `?st=` token; checked in the handler.
        .route("/api/v1/files/{id}/content", get(files::content))
        // Authenticated by SPAKE2 + SAS, not by a device token.
        .route("/api/v1/pair/{id}/exchange", post(pair::exchange))
        .route("/api/v1/pair/{id}/confirm", post(pair::confirm))
        .route("/api/v1/pair/{id}/result", get(pair::result));
    let device = Router::new()
        .route("/api/v1/index", get(index::index))
        .route("/api/v1/thumbs/{hash}", get(thumbs::thumb))
        .route("/api/v1/files/{id}/stream-url", post(files::stream_url))
        .route("/api/v1/uploads", post(uploads::create))
        .route(
            "/api/v1/uploads/{id}",
            get(uploads::status).delete(uploads::abort),
        )
        .route(
            "/api/v1/uploads/{id}/chunks/{index}",
            post(uploads::put_chunk).layer(DefaultBodyLimit::max(MAX_CHUNK_SIZE as usize)),
        )
        .route("/api/v1/uploads/{id}/commit", post(uploads::commit))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_device,
        ));
    public.merge(device).with_state(state)
}

/// An error rendered as `{"error": code, "message": ...}`.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: ErrorCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::BadRequest, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, message)
    }

    /// Logs the cause; the client only sees a generic message.
    pub fn internal(cause: impl std::fmt::Display) -> Self {
        error!(error = %cause, "internal error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "internal error",
        )
    }
}

impl From<bring_core::db::DbError> for ApiError {
    fn from(e: bring_core::db::DbError) -> Self {
        Self::internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: self.code,
            message: self.message,
        };
        (self.status, Json(body)).into_response()
    }
}
