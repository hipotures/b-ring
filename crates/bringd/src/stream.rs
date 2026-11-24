//! HTTP Range parsing and signed stream tokens.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

// ---------------------------------------------------------------------------
// Range
// ---------------------------------------------------------------------------

/// What to send for a `Range` header (RFC 9110 §14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
    /// No (usable) range: send the whole representation with `200`.
    Full,
    /// Inclusive byte range: send `206`.
    Partial { start: u64, end: u64 },
    /// `416` with `Content-Range: bytes */size`.
    Unsatisfiable,
}

/// Interprets a `Range` header for a resource of `size` bytes.
///
/// Malformed headers, other units and multi-range requests are ignored
/// (served in full), which RFC 9110 explicitly allows.
pub fn parse_range(header: Option<&str>, size: u64) -> RangeSpec {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return RangeSpec::Full;
    };
    if spec.contains(',') {
        return RangeSpec::Full;
    }
    let Some((first, last)) = spec.trim().split_once('-') else {
        return RangeSpec::Full;
    };
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        // Suffix range: the last N bytes.
        let Ok(n) = last.parse::<u64>() else {
            return RangeSpec::Full;
        };
        if n == 0 || size == 0 {
            return RangeSpec::Unsatisfiable;
        }
        return RangeSpec::Partial {
            start: size - n.min(size),
            end: size - 1,
        };
    }
    let Ok(start) = first.parse::<u64>() else {
        return RangeSpec::Full;
    };
    let end = if last.is_empty() {
        None
    } else {
        match last.parse::<u64>() {
            Ok(e) if e >= start => Some(e),
            _ => return RangeSpec::Full, // invalid: ignore the header
        }
    };
    if start >= size {
        return RangeSpec::Unsatisfiable;
    }
    RangeSpec::Partial {
        start,
        end: end.map_or(size - 1, |e| e.min(size - 1)),
    }
}

// ---------------------------------------------------------------------------
// Stream tokens
// ---------------------------------------------------------------------------

const TOKEN_CONTEXT: &[u8] = b"bring/stream-token/v1";

/// A verified stream token's claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamClaims {
    pub file_id: i64,
    pub device_id: i64,
    pub expires_at: i64,
}

fn mac(key: &[u8; 32], payload: &[u8; 24]) -> blake3::Hash {
    let mut h = blake3::Hasher::new_keyed(key);
    h.update(TOKEN_CONTEXT);
    h.update(payload);
    h.finalize()
}

/// `base64url(file_id || device_id || expires_at || MAC)`, MAC = keyed BLAKE3.
pub fn sign_token(key: &[u8; 32], claims: StreamClaims) -> String {
    let mut payload = [0u8; 24];
    payload[0..8].copy_from_slice(&claims.file_id.to_le_bytes());
    payload[8..16].copy_from_slice(&claims.device_id.to_le_bytes());
    payload[16..24].copy_from_slice(&claims.expires_at.to_le_bytes());
    let mut out = payload.to_vec();
    out.extend_from_slice(mac(key, &payload).as_bytes());
    URL_SAFE_NO_PAD.encode(out)
}

/// Verifies the MAC and expiry. Device revocation is checked by the caller.
pub fn verify_token(key: &[u8; 32], token: &str, now: i64) -> Option<StreamClaims> {
    let raw = URL_SAFE_NO_PAD.decode(token).ok()?;
    if raw.len() != 24 + 32 {
        return None;
    }
    let payload: [u8; 24] = raw[..24].try_into().ok()?;
    let tag: [u8; 32] = raw[24..].try_into().ok()?;
    // blake3::Hash equality is constant-time.
    if mac(key, &payload) != blake3::Hash::from_bytes(tag) {
        return None;
    }
    let i64_at = |o: usize| i64::from_le_bytes(payload[o..o + 8].try_into().expect("8 bytes"));
    let claims = StreamClaims {
        file_id: i64_at(0),
        device_id: i64_at(8),
        expires_at: i64_at(16),
    };
    (claims.expires_at > now).then_some(claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use RangeSpec::*;

    #[test]
    fn ranges() {
        let s = 10_000;
        let cases = [
            (None, Full),
            (
                Some("bytes=0-"),
                Partial {
                    start: 0,
                    end: 9999,
                },
            ),
            (
                Some("bytes=100-199"),
                Partial {
                    start: 100,
                    end: 199,
                },
            ),
            (
                Some("bytes=-500"),
                Partial {
                    start: 9500,
                    end: 9999,
                },
            ),
            (
                Some("bytes=-20000"),
                Partial {
                    start: 0,
                    end: 9999,
                },
            ),
            (
                Some("bytes=9990-20000"),
                Partial {
                    start: 9990,
                    end: 9999,
                },
            ),
            (
                Some("bytes=9999-9999"),
                Partial {
                    start: 9999,
                    end: 9999,
                },
            ),
            (Some("bytes=10000-"), Unsatisfiable),
            (Some("bytes=-0"), Unsatisfiable),
            (Some("bytes=0-1,5-6"), Full),
            (Some("bytes=5-1"), Full),
            (Some("bytes=abc"), Full),
            (Some("bytes=x-1"), Full),
            (Some("items=0-1"), Full),
        ];
        for (h, want) in cases {
            assert_eq!(parse_range(h, s), want, "{h:?}");
        }
        assert_eq!(parse_range(Some("bytes=0-"), 0), Unsatisfiable);
        assert_eq!(parse_range(Some("bytes=-1"), 0), Unsatisfiable);
        assert_eq!(parse_range(None, 0), Full);
    }

    #[test]
    fn tokens() {
        let key = [3u8; 32];
        let c = StreamClaims {
            file_id: 42,
            device_id: 7,
            expires_at: 1_000,
        };
        let t = sign_token(&key, c);
        assert_eq!(verify_token(&key, &t, 999), Some(c));
        assert_eq!(verify_token(&key, &t, 1_000), None, "expired");
        assert_eq!(verify_token(&[4u8; 32], &t, 0), None, "wrong key");

        let mut raw = URL_SAFE_NO_PAD.decode(&t).unwrap();
        raw[0] ^= 1; // claim another file
        assert_eq!(verify_token(&key, &URL_SAFE_NO_PAD.encode(&raw), 0), None);
        assert_eq!(verify_token(&key, "garbage", 0), None);
        assert_eq!(verify_token(&key, "", 0), None);
    }
}
