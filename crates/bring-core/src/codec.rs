//! Small text encodings used on the wire.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Lowercase hex.
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(HEX[usize::from(b >> 4)]));
        s.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    s
}

/// Decodes hex (either case) into exactly `N` bytes.
pub fn hex_decode<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// base64url without padding.
pub fn b64_encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let b = [0u8, 1, 0xab, 0xff];
        assert_eq!(hex_encode(&b), "0001abff");
        assert_eq!(hex_decode::<4>("0001ABff"), Some(b));
        assert_eq!(hex_decode::<4>("0001ab"), None);
        assert_eq!(hex_decode::<2>("zzzz"), None);
        assert_eq!(b64_decode(&b64_encode(&b)).unwrap(), b);
        assert_eq!(b64_decode("!!"), None);
    }
}
