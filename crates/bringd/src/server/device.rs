//! Authenticated device self-management (`/api/v1/device`).

use axum::{Extension, Json, extract::State};
use bring_core::api::{DeviceResponse, RenameDeviceRequest};
use tracing::{info, warn};

use super::auth::AuthDevice;
use super::{ApiError, AppState};
use crate::{devices, scanner};

/// `PATCH /api/v1/device` — renames the calling device, updates its slug, and
/// moves/merges its top-level `<old_slug>` directory into `<new_slug>` on disk.
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
    if old_slug != updated.slug {
        let root = state.root.clone();
        let db = state.db.clone();
        let hashq = state.hashq.clone();
        let from = old_slug.clone();
        let to = updated.slug.clone();
        let moved = tokio::task::spawn_blocking(move || -> std::io::Result<bool> {
            let moved = devices::move_device_dir(&root, &from, &to)?;
            if moved {
                match scanner::reconcile(&root, &db, None) {
                    Ok(to_hash) => hashq.enqueue_all(to_hash),
                    Err(e) => warn!(error = %e, "reconcile after device rename failed"),
                }
            }
            Ok(moved)
        })
        .await
        .map_err(ApiError::internal)?;
        if let Err(e) = moved {
            warn!(error = %e, from = %old_slug, to = %updated.slug, "moving device dir failed");
        }
    }
    info!(from = %old_slug, to = %updated.slug, name = %updated.name, "device renamed");
    Ok(Json(DeviceResponse {
        name: updated.name,
        slug: updated.slug,
    }))
}
