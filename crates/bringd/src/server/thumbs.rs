use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use bring_core::Blake3Hash;

use super::{ApiError, AppState};

/// `GET /thumbs/{thumb_blake3}` — immutable, content-addressed JPEG.
pub async fn thumb(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let hash = Blake3Hash::from_hex(&hash)
        .map_err(|_| ApiError::bad_request("thumbnail id must be 64 hex characters"))?;
    let bytes = match tokio::fs::read(state.thumbs.path_of(&hash)).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::not_found("no such thumbnail"));
        }
        Err(e) => return Err(ApiError::internal(e)),
    };
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    ))
}
