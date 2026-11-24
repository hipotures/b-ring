//! Crockford base32 with a mod-37 check symbol.
//!
//! Bits are packed MSB-first, 5 bits per symbol; the final symbol is padded
//! with zero bits. The check symbol is the value of the data symbol string
//! (read as a base-32 number) modulo 37. Because 37 is prime and does not
//! divide any power of 32, every single-symbol substitution and every adjacent
//! transposition is detected.
//!
//! Decoding is forgiving the way Crockford intends: case-insensitive, `O`→`0`,
//! `I`/`L`→`1`, and hyphens/whitespace are ignored.

/// Data alphabet (values 0..32).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Check alphabet (values 0..37): data alphabet plus `*~$=U`.
const CHECK_ALPHABET: &[u8; 37] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ*~$=U";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Base32Error {
    #[error("input is empty")]
    Empty,
    #[error("invalid character {0:?}")]
    InvalidChar(char),
    #[error("checksum mismatch (typo?)")]
    Checksum,
    #[error("invalid length or non-zero padding")]
    Padding,
}

/// Encodes `data` into data symbols followed by one check symbol (no grouping).
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + 2);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut check: u32 = 0;
    let mut push = |v: u32, out: &mut String| {
        check = (check * 32 + v) % 37;
        out.push(char::from(ALPHABET[v as usize]));
    };
    for &b in data {
        acc = (acc << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            push((acc >> bits) & 31, &mut out);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        push((acc << (5 - bits)) & 31, &mut out);
    }
    out.push(char::from(CHECK_ALPHABET[check as usize]));
    out
}

/// Encodes and groups the result into blocks of `group` characters joined by `-`.
pub fn encode_grouped(data: &[u8], group: usize) -> String {
    let flat = encode(data);
    let chars: Vec<char> = flat.chars().collect();
    chars
        .chunks(group.max(1))
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

fn data_value(c: char) -> Option<u32> {
    let c = c.to_ascii_uppercase();
    let c = match c {
        'O' => '0',
        'I' | 'L' => '1',
        other => other,
    };
    ALPHABET
        .iter()
        .position(|&a| char::from(a) == c)
        .map(|p| p as u32)
}

fn check_value(c: char) -> Option<u32> {
    if let Some(v) = data_value(c) {
        return Some(v);
    }
    let c = c.to_ascii_uppercase();
    CHECK_ALPHABET[32..]
        .iter()
        .position(|&a| char::from(a) == c)
        .map(|p| p as u32 + 32)
}

/// Decodes a string produced by [`encode`] / [`encode_grouped`].
pub fn decode(input: &str) -> Result<Vec<u8>, Base32Error> {
    let symbols: Vec<char> = input
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .collect();
    let Some((&check_char, data_chars)) = symbols.split_last() else {
        return Err(Base32Error::Empty);
    };
    let expected_check = check_value(check_char).ok_or(Base32Error::InvalidChar(check_char))?;

    let mut out = Vec::with_capacity(data_chars.len() * 5 / 8);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut check: u32 = 0;
    for &c in data_chars {
        let v = data_value(c).ok_or(Base32Error::InvalidChar(c))?;
        check = (check * 32 + v) % 37;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    if check != expected_check {
        return Err(Base32Error::Checksum);
    }
    // Leftover bits must be the zero padding of the final symbol (< 5 bits).
    if bits >= 5 || acc != 0 {
        return Err(Base32Error::Padding);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_values() {
        // 0x00 -> "00" + check 0.
        assert_eq!(encode(&[0]), "000");
        // 0xFF -> 11111 111(00) -> "Z" "W", value = 31*32 + 28 = 1020; 1020 % 37 = 21 -> 'N'.
        assert_eq!(encode(&[0xFF]), "ZWN");
        assert_eq!(decode("ZWN").unwrap(), vec![0xFF]);
    }

    #[test]
    fn forgiving_decode() {
        let data = b"\x00\x01\x02hello";
        let enc = encode_grouped(data, 5)
            .to_lowercase()
            .replace('0', "o")
            .replace('1', "l");
        assert_eq!(decode(&format!("  {enc} ")).unwrap(), data);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(decode(""), Err(Base32Error::Empty));
        assert_eq!(decode("--"), Err(Base32Error::Empty));
        assert_eq!(decode("0U0"), Err(Base32Error::InvalidChar('U')));
        assert!(matches!(decode("ZWQ"), Err(Base32Error::Checksum)));
    }

    proptest! {
        #[test]
        fn roundtrip(data in prop::collection::vec(any::<u8>(), 0..64), group in 1usize..8) {
            prop_assert_eq!(decode(&encode(&data)).unwrap(), data.clone());
            prop_assert_eq!(decode(&encode_grouped(&data, group)).unwrap(), data);
        }

        #[test]
        fn single_substitution_detected(
            data in prop::collection::vec(any::<u8>(), 1..40),
            pos_seed in any::<usize>(),
            delta in 1u32..32,
        ) {
            let enc: Vec<char> = encode(&data).chars().collect();
            let data_len = enc.len() - 1;
            let pos = pos_seed % data_len;
            let old = data_value(enc[pos]).unwrap();
            let mut typo = enc.clone();
            typo[pos] = char::from(ALPHABET[((old + delta) % 32) as usize]);
            let typo: String = typo.into_iter().collect();
            prop_assert!(decode(&typo).is_err());
        }

        #[test]
        fn adjacent_transposition_detected(
            data in prop::collection::vec(any::<u8>(), 2..40),
            pos_seed in any::<usize>(),
        ) {
            let enc: Vec<char> = encode(&data).chars().collect();
            let pos = pos_seed % (enc.len() - 2);
            prop_assume!(enc[pos] != enc[pos + 1]);
            let mut typo = enc.clone();
            typo.swap(pos, pos + 1);
            let typo: String = typo.into_iter().collect();
            prop_assert!(decode(&typo).is_err());
        }
    }
}
