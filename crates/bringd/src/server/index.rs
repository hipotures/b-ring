use axum::Json;
use axum::extract::{Query, State};
use bring_core::api::{INDEX_MAX_LIMIT, IndexResponse};
use serde::Deserialize;

use super::{ApiError, AppState};
use crate::store;

#[derive(Debug, Deserialize)]
pub struct IndexQuery {
    #[serde(default)]
    since: i64,
    limit: Option<u32>,
}

/// `GET /index?since=&limit=` — delta sync.
pub async fn index(
    State(state): State<AppState>,
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
    }))
}
