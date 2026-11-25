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
    /// Calling device's current slug on the Ring (lets CLI `bring rename`
    /// propagate back to the Moon on its next sync).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_slug: Option<String>,
}

/// `POST /rescan`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RescanResponse {
    /// Files queued for hashing or thumbnail generation.
    pub queued: u32,
}

/// `PATCH /device`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameDeviceRequest {
    pub name: String,
}

/// Response of `PATCH /device`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceResponse {
    pub name: String,
    pub slug: String,
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

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// Query parameter carrying a signed stream token.
pub const STREAM_TOKEN_PARAM: &str = "st";

/// `POST /files/{id}/stream-url`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamUrlResponse {
    /// Path and query, relative to the Ring base URL the client used
    /// (`/api/v1/files/{id}/content?st=...`). Works without headers.
    pub url: String,
    /// Unix seconds after which the URL stops working.
    pub expires_at: i64,
}

// ---------------------------------------------------------------------------
// Pairing
// ---------------------------------------------------------------------------

/// `POST /pair/{pairing_id}/exchange`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairExchangeRequest {
    /// Hex, 16 bytes, generated once per Moon installation.
    pub device_uuid: String,
    pub device_name: String,
    /// base64url SPAKE2 message (side A).
    pub spake_msg: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairExchangeResponse {
    /// base64url SPAKE2 message (side B).
    pub spake_msg: String,
    /// base64url HMAC proving the Ring derived the same key.
    pub ring_confirm: String,
}

/// `POST /pair/{pairing_id}/confirm`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairConfirmRequest {
    pub moon_confirm: String,
}

/// `202` body of `/confirm` and of a pending `/result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairPending {
    /// Always `"awaiting_user"`.
    pub status: String,
}

/// `200` body of `GET /pair/{pairing_id}/result`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairResultResponse {
    /// base64url(nonce || ChaCha20-Poly1305 ciphertext of [`PairingResult`]).
    pub sealed: String,
}

/// What the Ring hands a newly paired Moon (encrypted inside `sealed`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResult {
    pub device_token: String,
    pub device_slug: String,
    /// Hex.
    pub ring_id: String,
    /// Hex SHA-256 of the Ring's LAN certificate: the TLS pin.
    pub tls_cert_sha256: String,
    pub lan_url: String,
    pub wan_url: Option<String>,
}

impl std::fmt::Debug for PairingResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingResult")
            .field("device_token", &"<redacted>")
            .field("device_slug", &self.device_slug)
            .field("ring_id", &self.ring_id)
            .field("tls_cert_sha256", &self.tls_cert_sha256)
            .field("lan_url", &self.lan_url)
            .field("wan_url", &self.wan_url)
            .finish()
    }
}
