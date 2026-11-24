//! HTTP API DTOs (`/api/v1`). The single source of truth for the wire format,
//! shared by `bringd` (server) and `bring-uniffi` (client).

use serde::{Deserialize, Serialize};

use crate::model::{Blake3Hash, RelPath};

/// Prefix of every API route.
pub const API_PREFIX: &str = "/api/v1";

/// Machine-readable error codes (`{"error": "<code>"}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    NotFound,
    HashMismatch,
    MissingChunks,
    BadChunkLength,
    Conflict,
    Expired,
    Rejected,
    TooManyAttempts,
    RangeNotSatisfiable,
    Internal,
}

/// Body of every error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorCode,
    pub message: String,
}

/// `GET /health`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Hex-encoded 16-byte Ring identity.
    pub ring_id: String,
    /// `bringd` version.
    pub version: String,
    /// Wire protocol version.
    pub protocol: u32,
}

/// Maximum page size of `GET /index`.
pub const INDEX_MAX_LIMIT: u32 = 1000;

/// One file record in the delta sync stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: i64,
    pub path: RelPath,
    pub folder: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub mime: Option<String>,
    /// `None` while the Ring is still hashing the file.
    pub blake3: Option<Blake3Hash>,
    /// Slug of the uploading device; `None` = added on the Ring desktop.
    pub origin: Option<String>,
    pub thumb: Option<Blake3Hash>,
    pub deleted: bool,
    pub seq: i64,
}

/// `GET /index?since=&limit=`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexResponse {
    pub entries: Vec<IndexEntry>,
    /// Cursor for the next request (`since` if there were no entries).
    pub next_since: i64,
    pub has_more: bool,
}
