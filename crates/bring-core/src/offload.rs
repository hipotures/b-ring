//! Retention rule evaluated on a Moon after an upload attempt.
//!
//! This is the only place that decides whether a local file may be deleted.
//! It is pure logic so every branch can be tested exhaustively. The safety
//! invariant: [`Decision::may_delete_local`] is true only for a matching,
//! verified ACK on a file that has not changed since it was hashed.

use crate::model::{Blake3Hash, FileState};

/// Default offload threshold: 100 MiB.
pub const DEFAULT_OFFLOAD_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

/// Retention policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffloadPolicy {
    /// Files strictly larger than this are removed locally after a verified ACK.
    pub threshold_bytes: u64,
}

impl Default for OffloadPolicy {
    fn default() -> Self {
        Self {
            threshold_bytes: DEFAULT_OFFLOAD_THRESHOLD_BYTES,
        }
    }
}

/// What the Moon recorded when it hashed the file (before `POST /uploads`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashSnapshot {
    pub blake3: Blake3Hash,
    pub size: u64,
    pub mtime_ns: i64,
}

/// The current `stat` of the local file, taken right before deciding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalStat {
    pub size: u64,
    pub mtime_ns: i64,
}

/// The fields of the Ring's ACK that matter for retention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedAck {
    pub blake3: Blake3Hash,
    pub size: u64,
}

/// Why nothing may happen yet; the entry stays in the outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryReason {
    /// Network error, timeout, or any non-ACK response.
    NoAck,
    /// The ACK names a different hash than the one we computed.
    HashMismatch,
    /// The ACK names a different size than the one we hashed.
    SizeMismatch,
}

/// The outcome of the retention rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Verified and above the threshold: delete the local file → `remote-only`.
    Offload,
    /// Verified and at/below the threshold: keep the local file → `synced`.
    KeepSynced,
    /// Verified, but the local file is already gone (user deleted it) → `remote-only`.
    LocalGone,
    /// The local file changed after hashing. The Ring holds the *old* content;
    /// keep the file and re-hash/re-upload it (it becomes `local-only`).
    LocalChanged,
    /// Not verified. Keep everything and retry later.
    Retry(RetryReason),
}

impl Decision {
    /// True only when deleting the local file is allowed.
    pub fn may_delete_local(self) -> bool {
        matches!(self, Self::Offload)
    }

    /// The state the entry moves to, or `None` if it must stay as it is.
    pub fn resulting_state(self) -> Option<FileState> {
        match self {
            Self::Offload | Self::LocalGone => Some(FileState::RemoteOnly),
            Self::KeepSynced => Some(FileState::Synced),
            Self::LocalChanged => Some(FileState::LocalOnly),
            Self::Retry(_) => None,
        }
    }
}

/// Applies the retention rule.
///
/// - `ack`: `None` for any failed attempt (network error, timeout, 4xx/5xx).
/// - `current`: `None` if the local file no longer exists.
pub fn decide(
    policy: &OffloadPolicy,
    snapshot: &HashSnapshot,
    ack: Option<&VerifiedAck>,
    current: Option<&LocalStat>,
) -> Decision {
    let Some(ack) = ack else {
        return Decision::Retry(RetryReason::NoAck);
    };
    if ack.blake3 != snapshot.blake3 {
        return Decision::Retry(RetryReason::HashMismatch);
    }
    if ack.size != snapshot.size {
        return Decision::Retry(RetryReason::SizeMismatch);
    }
    let Some(current) = current else {
        return Decision::LocalGone;
    };
    if current.size != snapshot.size || current.mtime_ns != snapshot.mtime_ns {
        return Decision::LocalChanged;
    }
    if snapshot.size > policy.threshold_bytes {
        Decision::Offload
    } else {
        Decision::KeepSynced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = DEFAULT_OFFLOAD_THRESHOLD_BYTES;

    fn h(b: u8) -> Blake3Hash {
        Blake3Hash([b; 32])
    }

    fn snap(size: u64) -> HashSnapshot {
        HashSnapshot {
            blake3: h(1),
            size,
            mtime_ns: 1_000,
        }
    }

    fn ack(size: u64) -> VerifiedAck {
        VerifiedAck { blake3: h(1), size }
    }

    fn stat(size: u64) -> LocalStat {
        LocalStat {
            size,
            mtime_ns: 1_000,
        }
    }

    #[test]
    fn default_threshold_is_100_mib() {
        assert_eq!(OffloadPolicy::default().threshold_bytes, 104_857_600);
    }

    #[test]
    fn table() {
        use Decision::*;
        use RetryReason::*;
        let p = OffloadPolicy::default();
        let changed_mtime = LocalStat {
            size: T + 1,
            mtime_ns: 2_000,
        };
        type Case = (
            &'static str,
            HashSnapshot,
            Option<VerifiedAck>,
            Option<LocalStat>,
            Decision,
        );
        let cases: Vec<Case> = vec![
            (
                "threshold + 1 offloads",
                snap(T + 1),
                Some(ack(T + 1)),
                Some(stat(T + 1)),
                Offload,
            ),
            (
                "exactly threshold keeps",
                snap(T),
                Some(ack(T)),
                Some(stat(T)),
                KeepSynced,
            ),
            (
                "threshold - 1 keeps",
                snap(T - 1),
                Some(ack(T - 1)),
                Some(stat(T - 1)),
                KeepSynced,
            ),
            (
                "empty file keeps",
                snap(0),
                Some(ack(0)),
                Some(stat(0)),
                KeepSynced,
            ),
            ("no ack", snap(T + 1), None, Some(stat(T + 1)), Retry(NoAck)),
            ("no ack, file gone", snap(T + 1), None, None, Retry(NoAck)),
            (
                "hash mismatch",
                snap(T + 1),
                Some(VerifiedAck {
                    blake3: h(2),
                    size: T + 1,
                }),
                Some(stat(T + 1)),
                Retry(HashMismatch),
            ),
            (
                "size mismatch",
                snap(T + 1),
                Some(ack(T + 2)),
                Some(stat(T + 1)),
                Retry(SizeMismatch),
            ),
            (
                "changed mtime",
                snap(T + 1),
                Some(ack(T + 1)),
                Some(changed_mtime),
                LocalChanged,
            ),
            (
                "changed size",
                snap(T + 1),
                Some(ack(T + 1)),
                Some(stat(T + 5)),
                LocalChanged,
            ),
            (
                "local gone after ack",
                snap(T + 1),
                Some(ack(T + 1)),
                None,
                LocalGone,
            ),
            (
                "small file gone after ack",
                snap(10),
                Some(ack(10)),
                None,
                LocalGone,
            ),
        ];
        for (name, s, a, c, expected) in cases {
            assert_eq!(decide(&p, &s, a.as_ref(), c.as_ref()), expected, "{name}");
        }
    }

    #[test]
    fn custom_threshold() {
        let p = OffloadPolicy { threshold_bytes: 0 };
        assert_eq!(
            decide(&p, &snap(1), Some(&ack(1)), Some(&stat(1))),
            Decision::Offload
        );
        assert_eq!(
            decide(&p, &snap(0), Some(&ack(0)), Some(&stat(0))),
            Decision::KeepSynced
        );
    }

    #[test]
    fn only_offload_may_delete() {
        use Decision::*;
        for d in [
            Offload,
            KeepSynced,
            LocalGone,
            LocalChanged,
            Retry(RetryReason::NoAck),
            Retry(RetryReason::HashMismatch),
            Retry(RetryReason::SizeMismatch),
        ] {
            assert_eq!(d.may_delete_local(), d == Offload, "{d:?}");
        }
    }

    #[test]
    fn resulting_states() {
        assert_eq!(
            Decision::Offload.resulting_state(),
            Some(FileState::RemoteOnly)
        );
        assert_eq!(
            Decision::LocalGone.resulting_state(),
            Some(FileState::RemoteOnly)
        );
        assert_eq!(
            Decision::KeepSynced.resulting_state(),
            Some(FileState::Synced)
        );
        assert_eq!(
            Decision::LocalChanged.resulting_state(),
            Some(FileState::LocalOnly)
        );
        assert_eq!(Decision::Retry(RetryReason::NoAck).resulting_state(), None);
    }
}
