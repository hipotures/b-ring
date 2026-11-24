//! Streaming BLAKE3 hashing.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::model::Blake3Hash;

/// Hashes an in-memory buffer.
pub fn hash_bytes(data: &[u8]) -> Blake3Hash {
    blake3::hash(data).into()
}

/// Hashes everything `reader` yields. Returns the digest and the byte count.
pub fn hash_reader(reader: impl Read) -> io::Result<(Blake3Hash, u64)> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(reader)?;
    Ok((hasher.finalize().into(), hasher.count()))
}

/// Hashes a file on disk. Returns the digest and the number of bytes hashed.
pub fn hash_file(path: &Path) -> io::Result<(Blake3Hash, u64)> {
    hash_reader(File::open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn file_and_bytes_agree() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&data).unwrap();
        let (h, n) = hash_file(f.path()).unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(h, hash_bytes(&data));
    }

    #[test]
    fn known_vector() {
        // BLAKE3 of the empty input.
        assert_eq!(
            hash_bytes(b"").to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }
}
