//! Control protocol between `bring` (CLI) and `bringd`: JSON lines over a
//! Unix socket. One request per connection; `pair` is a conversation.
//!
//! ```text
//! bring → {"cmd":"pair"}
//! bringd → {"event":"pair_payload", ...}      (show QR + text)
//! bringd → {"event":"pair_sas", ...}          (a Moon completed the handshake)
//! bring → {"cmd":"pair_decision","approve":true}
//! bringd → {"event":"pair_done", ...} | {"event":"pair_rejected"}
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Requests from the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CtlRequest {
    Status,
    Devices,
    /// Revoke by slug or numeric id.
    Revoke {
        device: String,
    },
    /// Rename a device by slug or numeric id.
    Rename {
        device: String,
        name: String,
    },
    Pair,
    /// Answer to `PairSas` within a `Pair` conversation.
    PairDecision {
        approve: bool,
    },
}

/// A paired device as shown by the CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: i64,
    #[serde(default)]
    pub device_uuid: String,
    pub name: String,
    pub slug: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked: bool,
}

/// `bring status`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusInfo {
    pub version: String,
    pub ring_id: String,
    pub ring_root: String,
    pub lan_url: String,
    pub wan_url: Option<String>,
    pub tls_cert_sha256: String,
    pub files: u64,
    pub bytes: u64,
    pub pending_hashes: u64,
    pub devices: u64,
    pub open_uploads: u64,
}

/// Responses and events from the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CtlEvent {
    Status(StatusInfo),
    Devices {
        devices: Vec<DeviceInfo>,
    },
    Revoked {
        device: DeviceInfo,
    },
    Renamed {
        device: DeviceInfo,
    },
    /// Show this to the Moon (QR code and/or text for manual entry).
    PairPayload {
        text: String,
        lan_url: String,
        expires_in_secs: u64,
    },
    /// Ask the user whether the Moon shows the same code.
    PairSas {
        sas: String,
        device_name: String,
    },
    PairDone {
        device: DeviceInfo,
    },
    PairRejected,
    PairExpired,
    PairFailed {
        message: String,
    },
    Error {
        message: String,
    },
}

/// `$XDG_RUNTIME_DIR/bring/bringd.sock`, or `/tmp/bring-$USER/bringd.sock`.
pub fn default_socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("bring/bringd.sock"),
        _ => {
            let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
            PathBuf::from(format!("/tmp/bring-{user}/bringd.sock"))
        }
    }
}

/// Formats a SAS for display: `"123456"` → `"123 456"`.
pub fn format_sas(sas: &str) -> String {
    if sas.len() == 6 {
        format!("{} {}", &sas[..3], &sas[3..])
    } else {
        sas.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format() {
        let req = CtlRequest::PairDecision { approve: true };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"cmd":"pair_decision","approve":true}"#
        );
        assert_eq!(
            serde_json::from_str::<CtlRequest>(r#"{"cmd":"status"}"#).unwrap(),
            CtlRequest::Status
        );
        let ev = CtlEvent::PairSas {
            sas: "123456".into(),
            device_name: "Pixel".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.starts_with(r#"{"event":"pair_sas""#));
        assert_eq!(serde_json::from_str::<CtlEvent>(&json).unwrap(), ev);
        assert_eq!(format_sas("012345"), "012 345");
    }
}
