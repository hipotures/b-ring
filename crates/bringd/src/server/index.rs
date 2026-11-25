use axum::extract::{Extension, Query, State};
use axum::Json;
use bring_core::api::{INDEX_MAX_LIMIT, IndexResponse, RescanResponse};
use serde::Deserialize;
use tracing::info;

use super::auth::AuthDevice;
use super::{ApiError, AppState};
use crate::{scanner, store};

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(default)]
    since: i64,
    limit: Option<u32>,
}

/// `GET /index?since=&limit=` — delta sync.
pub async fn index(
    State(state): State<AppState>,
    Extension(dev): Extension<AuthDevice>,
    Query(q): Query<IndexQuery>,
) -> Result<Json<IndexResponse>, ApiError> {
    if q.since < 0 {
        return Err(ApiError::bad_request("since must be >= 0"));
    }
    let limit = q.limit.unwrap_or(INDEX_MAX_LIMIT).clamp(1, INDEX_MAX_LIMIT);
    let (entries, has_more) = state
        .db
        .call(move |c| store::index_since(c, q.since, limit))
        .await?;
    let next_since = entries.last().map_or(q.since, |e| e.seq);
    Ok(Json(IndexResponse {
        entries,
        next_since,
        has_more,
        device_slug: Some(dev.slug),
    }))
}

/// `POST /rescan` — reconcile the Ring root with the index and queue any
/// missing hashes or thumbnails for generation.
pub async fn rescan(State(state): State<AppState>) -> Result<Json<RescanResponse>, ApiError> {
    let root = state.root.clone();
    let db = state.db.clone();
    let to_hash = tokio::task::spawn_blocking(move || scanner::reconcile(&root, &db, None))
        .await
        .map_err(ApiError::internal)??;
    let queued = to_hash.len() as u32;
    state.hashq.enqueue_all(to_hash);
    info!(queued, "on-demand rescan queued");
    Ok(Json(RescanResponse { queued }))
}
