//! HTTP API (`/api/v1`).

mod health;
mod index;
mod thumbs;

use axum::Json;
use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bring_core::api::{ErrorBody, ErrorCode};
use tracing::error;

use crate::store::Db;
use crate::thumbs::ThumbStore;

/// State shared by all handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    /// Hex-encoded Ring identity.
    pub ring_id: String,
    pub thumbs: ThumbStore,
}

/// Builds the API router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health::health))
        .route("/api/v1/index", get(index::index))
        .route("/api/v1/thumbs/{hash}", get(thumbs::thumb))
        .with_state(state)
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
