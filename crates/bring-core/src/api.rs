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

// ---------------------------------------------------------------------------
// Uploads
// ---------------------------------------------------------------------------

/// Header carrying the BLAKE3 of a chunk body (lowercase hex).
pub const CHUNK_HASH_HEADER: &str = "x-bring-chunk-blake3";

/// `POST /uploads`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateUploadRequest {
    /// Requested location in the Ring.
    pub path: RelPath,
    pub size: u64,
    /// Modification time to preserve on the Ring, if known.
    pub mtime_ns: Option<i64>,
    /// BLAKE3 of the whole file, computed by the Moon before uploading.
    pub blake3: Blake3Hash,
    /// Defaults to 8 MiB.
    pub chunk_size: Option<u64>,
}

/// Lifecycle of an upload on the Ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadState {
    Open,
    Committing,
    Committed,
    Failed,
}

impl UploadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Committing => "committing",
            Self::Committed => "committed",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "committing" => Some(Self::Committing),
            "committed" => Some(Self::Committed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// The Ring's proof that a file is durably stored with a verified hash.
/// A Moon may only delete its local copy after receiving a matching ACK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadAck {
    /// `None` when the file was already present (dedupe, no upload needed).
    pub upload_id: Option<String>,
    pub file_id: i64,
    /// Final location (may differ from the requested path on conflicts).
    pub path: RelPath,
    pub size: u64,
    pub blake3: Blake3Hash,
    pub seq: i64,
}

/// Response of `POST /uploads`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CreateUploadResponse {
    /// `201` (new) or `200` (resumed): send the chunks not yet `received`.
    Open {
        upload_id: String,
        chunk_size: u64,
        chunk_count: u32,
        received: Vec<u32>,
    },
    /// `200`: the Ring already has this exact content; nothing to send.
    Exists { ack: UploadAck },
}

/// `GET /uploads/{id}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadStatus {
    pub upload_id: String,
    pub state: UploadState,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub received: Vec<u32>,
}
