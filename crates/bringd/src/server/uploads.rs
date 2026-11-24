use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::{Extension, Json};
use bring_core::Blake3Hash;
use bring_core::api::{
    CHUNK_HASH_HEADER, CreateUploadRequest, CreateUploadResponse, UploadAck, UploadStatus,
};

use super::auth::AuthDevice;
use super::{ApiError, AppState};
use crate::notify::UploadEvent;
use crate::uploads;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(ApiError::internal)?
}

/// `POST /uploads`
pub async fn create(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Json(req): Json<CreateUploadRequest>,
) -> Result<(StatusCode, Json<CreateUploadResponse>), ApiError> {
    let up = state.uploads.clone();
    let (created, resp) = blocking(move || uploads::create(&up, &dev, &req)).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(resp)))
}

/// `GET /uploads/{id}`
pub async fn status(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Path(id): Path<String>,
) -> Result<Json<UploadStatus>, ApiError> {
    let up = state.uploads.clone();
    Ok(Json(
        blocking(move || uploads::status(&up, &dev, &id)).await?,
    ))
}

/// `POST /uploads/{id}/chunks/{index}`
pub async fn put_chunk(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Path((id, index)): Path<(String, u32)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let claimed = headers
        .get(CHUNK_HASH_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Blake3Hash::from_hex(v).ok())
        .ok_or_else(|| ApiError::bad_request(format!("missing or invalid {CHUNK_HASH_HEADER}")))?;
    let up = state.uploads.clone();
    let _guard = up.lock(&id).await;
    blocking(move || uploads::put_chunk(&up, &dev, &id, index, claimed, &body)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /uploads/{id}/commit`
pub async fn commit(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Path(id): Path<String>,
) -> Result<Json<UploadAck>, ApiError> {
    let up = state.uploads.clone();
    let _guard = up.lock(&id).await;
    let slug = dev.slug.clone();
    let committed = blocking(move || uploads::commit(&up, &dev, &id)).await?;
    if committed.new_file {
        // Thumbnail generation (the hash is already known).
        state.hashq.enqueue(committed.ack.path.clone());
        state.notifier.upload(UploadEvent {
            path: committed.ack.path.to_string(),
            device: slug,
            size: committed.ack.size,
        });
    }
    Ok(Json(committed.ack))
}

/// `DELETE /uploads/{id}`
pub async fn abort(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let up = state.uploads.clone();
    let _guard = up.lock(&id).await;
    blocking(move || uploads::abort(&up, &dev, &id)).await?;
    Ok(StatusCode::NO_CONTENT)
}
