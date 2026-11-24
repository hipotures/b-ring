//! Authenticated device self-management (`/api/v1/device`).

use axum::{Extension, Json, extract::State};
use bring_core::api::{DeviceResponse, RenameDeviceRequest};
use tracing::info;

use super::auth::AuthDevice;
use super::{ApiError, AppState};
use crate::devices;

/// `PATCH /api/v1/device` — renames the calling device and updates its slug.
pub async fn rename(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Json(req): Json<RenameDeviceRequest>,
) -> Result<Json<DeviceResponse>, ApiError> {
    let clean = devices::validate_name(&req.name)
        .map_err(ApiError::bad_request)?
        .to_owned();
    let old_slug = dev.slug.clone();
    let id = dev.id;
    let updated = state
        .db
        .call(move |c| devices::rename(c, id, &clean))
        .await?
        .ok_or_else(|| ApiError::not_found("device not found"))?;
    info!(from = %old_slug, to = %updated.slug, name = %updated.name, "device renamed");
    Ok(Json(DeviceResponse {
        name: updated.name,
        slug: updated.slug,
    }))
}
