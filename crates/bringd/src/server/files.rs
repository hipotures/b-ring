//! File content streaming with HTTP Range support.

use std::io::SeekFrom;
use std::os::unix::fs::MetadataExt;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use bring_core::api::{ErrorCode, StreamUrlResponse};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use super::auth::{self, AuthDevice, unauthorized};
use super::{ApiError, AppState};
use crate::devices::{self, now_secs};
use crate::store::{self, DiskStat, FileRow};
use crate::stream::{self, RangeSpec, StreamClaims};

/// Lifetime of a signed stream URL.
pub const STREAM_URL_TTL_SECS: i64 = 6 * 3600;

#[derive(Debug, Deserialize)]
pub struct ContentQuery {
    st: Option<String>,
}

/// Accepts either a device bearer token or a valid `?st=` stream token.
async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    st: Option<String>,
    file_id: i64,
) -> Result<(), ApiError> {
    if let Some(token) = auth::bearer(headers) {
        auth::device_for_token(state, token).await?;
        return Ok(());
    }
    let st = st.ok_or_else(unauthorized)?;
    let claims = stream::verify_token(&state.stream_key, &st, now_secs())
        .filter(|c| c.file_id == file_id)
        .ok_or_else(unauthorized)?;
    let device_id = claims.device_id;
    let active = state
        .db
        .call(move |c| devices::by_id(c, device_id))
        .await?
        .is_some_and(|d| d.revoked_at.is_none());
    if active { Ok(()) } else { Err(unauthorized()) }
}

async fn live_file(state: &AppState, id: i64) -> Result<FileRow, ApiError> {
    state
        .db
        .call(move |c| store::file_by_id(c, id))
        .await?
        .filter(|r| !r.deleted)
        .ok_or_else(|| ApiError::not_found("no such file"))
}

/// Does an `If-None-Match` value match our ETag (weak comparison)?
fn none_match(value: &HeaderValue, etag: &str) -> bool {
    let Ok(v) = value.to_str() else { return false };
    v.split(',').any(|t| {
        let t = t.trim();
        t == "*" || t.strip_prefix("W/").unwrap_or(t) == etag
    })
}

/// `GET|HEAD /files/{id}/content`
pub async fn content(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<ContentQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    authorize(&state, &headers, q.st, id).await?;
    let row = live_file(&state, id).await?;
    let fs_path = row.path.to_fs_path(&state.uploads.root);

    let mut file = match tokio::fs::File::open(&fs_path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::not_found("file is gone from the Ring"));
        }
        Err(e) => return Err(ApiError::internal(e)),
    };
    // Refuse anything that is not the indexed regular file (e.g. replaced by a
    // symlink after indexing): the opened inode must be the path's own inode.
    let meta = file.metadata().await.map_err(ApiError::internal)?;
    let lmeta = tokio::fs::symlink_metadata(&fs_path)
        .await
        .map_err(ApiError::internal)?;
    if !lmeta.is_file() || lmeta.ino() != meta.ino() || lmeta.dev() != meta.dev() {
        return Err(ApiError::not_found("not a regular file"));
    }
    if DiskStat::of(&meta) != row.stat() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            ErrorCode::Conflict,
            "file changed on the Ring; index update pending",
        ));
    }

    let size = row.size;
    let etag = row.blake3.map(|h| format!("\"{h}\""));
    let mime = row.mime.as_deref().unwrap_or("application/octet-stream");

    if let (Some(etag), Some(inm)) = (&etag, headers.get(header::IF_NONE_MATCH))
        && none_match(inm, etag)
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag.clone())]).into_response());
    }

    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    // If-Range: honour the Range only if the client's validator is our
    // (strong) ETag; otherwise send the whole, current representation.
    let range = match headers.get(header::IF_RANGE) {
        None => range,
        Some(v) => range.filter(|_| {
            etag.as_deref()
                .is_some_and(|e| v.as_bytes() == e.as_bytes())
        }),
    };

    let mut resp = Response::builder()
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private");
    if let Some(e) = &etag {
        resp = resp.header(header::ETAG, e);
    }
    let (status, start, len) = match stream::parse_range(range, size) {
        RangeSpec::Full => (StatusCode::OK, 0, size),
        RangeSpec::Partial { start, end } => {
            resp = resp.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
            (StatusCode::PARTIAL_CONTENT, start, end - start + 1)
        }
        RangeSpec::Unsatisfiable => {
            let mut r = ApiError::new(
                StatusCode::RANGE_NOT_SATISFIABLE,
                ErrorCode::RangeNotSatisfiable,
                format!("range not satisfiable for {size} bytes"),
            )
            .into_response();
            r.headers_mut().insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes */{size}")).expect("ascii"),
            );
            return Ok(r);
        }
    };
    file.seek(SeekFrom::Start(start))
        .await
        .map_err(ApiError::internal)?;
    let body = Body::from_stream(ReaderStream::new(file.take(len)));
    resp.status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, len)
        .body(body)
        .map_err(ApiError::internal)
}

/// `POST /files/{id}/stream-url`
pub async fn stream_url(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Path(id): Path<i64>,
) -> Result<Json<StreamUrlResponse>, ApiError> {
    live_file(&state, id).await?;
    let expires_at = now_secs() + STREAM_URL_TTL_SECS;
    let token = stream::sign_token(
        &state.stream_key,
        StreamClaims {
            file_id: id,
            device_id: dev.id,
            expires_at,
        },
    );
    Ok(Json(StreamUrlResponse {
        url: format!("/api/v1/files/{id}/content?st={token}"),
        expires_at,
    }))
}
