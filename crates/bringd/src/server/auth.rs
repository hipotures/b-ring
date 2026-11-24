//! Bearer-token authentication for device routes.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use bring_core::api::ErrorCode;

use super::{ApiError, AppState};
use crate::devices;

/// The authenticated device, inserted into request extensions.
#[derive(Debug, Clone)]
pub struct AuthDevice {
    pub id: i64,
    pub slug: String,
}

pub fn unauthorized() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthorized,
        "missing, unknown or revoked device token",
    )
}

/// Extracts the token from `Authorization: Bearer <token>`.
pub fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

/// Resolves a token to a live device and records activity.
pub async fn device_for_token(state: &AppState, token: String) -> Result<AuthDevice, ApiError> {
    let found = state
        .db
        .call(move |c| {
            let dev = devices::by_token(c, &token)?;
            if let Some(d) = &dev {
                devices::touch(c, d.id, devices::now_secs())?;
            }
            Ok::<_, bring_core::db::DbError>(dev)
        })
        .await?;
    found
        .map(|d| AuthDevice {
            id: d.id,
            slug: d.slug,
        })
        .ok_or_else(unauthorized)
}

/// Middleware: rejects requests without a valid device token.
pub async fn require_device(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = bearer(req.headers()).ok_or_else(unauthorized)?;
    let device = device_for_token(&state, token).await?;
    req.extensions_mut().insert(device);
    Ok(next.run(req).await)
}
