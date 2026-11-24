//! The one error type Kotlin sees.

use std::error::Error as StdError;

use bring_core::api::ErrorBody;
use bring_core::db::DbError;
use bring_core::pairing::{PairingError, PayloadError};

// Fields are called `detail`, not `message`: UniFFI's Kotlin bindings also
// generate an `override val message` (Throwable), and the two would clash.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MoonError {
    #[error("not paired with a Ring")]
    NotPaired,
    /// Connection refused, timeout, TLS failure (including a pin mismatch).
    #[error("network error: {detail}")]
    Network { detail: String },
    /// The Ring answered with an error status.
    #[error("the Ring answered {status} ({code}): {detail}")]
    Server {
        status: u16,
        code: String,
        detail: String,
    },
    #[error("pairing failed: {detail}")]
    Pairing { detail: String },
    /// Not approved on the Ring yet; call `pair_finish` again.
    #[error("still waiting for approval on the Ring")]
    PairingPending,
    #[error("pairing was rejected on the Ring")]
    PairingRejected,
    #[error("the pairing session expired")]
    PairingExpired,
    /// Local database or filesystem error.
    #[error("storage error: {detail}")]
    Storage { detail: String },
    #[error("invalid input: {detail}")]
    Invalid { detail: String },
    #[error("cancelled")]
    Cancelled,
}

impl MoonError {
    pub(crate) fn invalid(m: impl Into<String>) -> Self {
        Self::Invalid { detail: m.into() }
    }

    pub(crate) fn storage(m: impl Into<String>) -> Self {
        Self::Storage { detail: m.into() }
    }

    /// The Ring's error code, if this is a server error.
    pub(crate) fn server_code(&self) -> Option<&str> {
        match self {
            Self::Server { code, .. } => Some(code),
            _ => None,
        }
    }

    pub(crate) fn status(&self) -> Option<u16> {
        match self {
            Self::Server { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Builds a server error from a non-2xx response body.
    pub(crate) fn from_response(status: u16, body: &[u8]) -> Self {
        match serde_json::from_slice::<ErrorBody>(body) {
            Ok(b) => Self::Server {
                status,
                code: serde_json::to_value(b.error)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
                detail: b.message,
            },
            Err(_) => Self::Server {
                status,
                code: String::new(),
                detail: String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned(),
            },
        }
    }
}

/// `e: cause: cause…` — reqwest's own `Display` hides the interesting part.
fn chain(e: &dyn StdError) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(c) = src {
        s.push_str(": ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}

impl From<reqwest::Error> for MoonError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network { detail: chain(&e) }
    }
}

impl From<rusqlite::Error> for MoonError {
    fn from(e: rusqlite::Error) -> Self {
        Self::storage(e.to_string())
    }
}

impl From<DbError> for MoonError {
    fn from(e: DbError) -> Self {
        Self::storage(e.to_string())
    }
}

impl From<std::io::Error> for MoonError {
    fn from(e: std::io::Error) -> Self {
        Self::storage(e.to_string())
    }
}

impl From<PairingError> for MoonError {
    fn from(e: PairingError) -> Self {
        Self::Pairing {
            detail: e.to_string(),
        }
    }
}

impl From<PayloadError> for MoonError {
    fn from(e: PayloadError) -> Self {
        Self::invalid(format!("pairing code: {e}"))
    }
}
