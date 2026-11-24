//! Chunked upload arithmetic shared by the Ring (validation) and the Moon
//! (slicing files).

use crate::model::{Blake3Hash, RelPath};

/// Default chunk size: 8 MiB.
pub const DEFAULT_CHUNK_SIZE: u64 = 8 * 1024 * 1024;
/// Smallest accepted chunk size: 64 KiB.
pub const MIN_CHUNK_SIZE: u64 = 64 * 1024;
/// Largest accepted chunk size: 64 MiB (bounds per-request memory).
pub const MAX_CHUNK_SIZE: u64 = 64 * 1024 * 1024;

/// Why an upload cannot be planned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("chunk size must be between {MIN_CHUNK_SIZE} and {MAX_CHUNK_SIZE} bytes")]
    ChunkSize,
    #[error("file is too large")]
    TooLarge,
}

/// How a file of `size` bytes is split into chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadPlan {
    pub size: u64,
    pub chunk_size: u64,
}

impl UploadPlan {
    pub fn new(size: u64, chunk_size: u64) -> Result<Self, PlanError> {
        if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size) {
            return Err(PlanError::ChunkSize);
        }
        if size > i64::MAX as u64 {
            return Err(PlanError::TooLarge);
        }
        let plan = Self { size, chunk_size };
        if plan.chunk_count_u64() > u64::from(u32::MAX) {
            return Err(PlanError::TooLarge);
        }
        Ok(plan)
    }

    fn chunk_count_u64(&self) -> u64 {
        self.size.div_ceil(self.chunk_size)
    }

    /// Number of chunks (0 for an empty file).
    pub fn chunk_count(&self) -> u32 {
        self.chunk_count_u64() as u32
    }

    /// `(offset, len)` of chunk `index`, or `None` if out of range.
    pub fn chunk_range(&self, index: u32) -> Option<(u64, u64)> {
        if index >= self.chunk_count() {
            return None;
        }
        let offset = u64::from(index) * self.chunk_size;
        Some((offset, self.chunk_size.min(self.size - offset)))
    }
}

/// Identifies "the same upload" for idempotent create/resume:
/// `blake3(path || 0x00 || size_le || hash)`.
pub fn client_key(path: &RelPath, size: u64, blake3: &Blake3Hash) -> Blake3Hash {
    let mut h = blake3::Hasher::new();
    h.update(path.as_str().as_bytes());
    h.update(&[0]);
    h.update(&size.to_le_bytes());
    h.update(blake3.as_bytes());
    h.finalize().into()
}

/// How many conflict names are tried before giving up.
pub const MAX_CONFLICT_NAMES: u32 = 1000;

/// Names tried for an upload, in order (never overwriting, §3.5): `a.jpg`, `a (slug).jpg`, `a (slug 2).jpg`, ...
pub fn conflict_candidates(path: &RelPath, slug: &str) -> impl Iterator<Item = RelPath> {
    let name = path.file_name().to_owned();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_owned(), name[i..].to_owned()),
        _ => (name.clone(), String::new()),
    };
    let path = path.clone();
    let slug = slug.to_owned();
    (1..=MAX_CONFLICT_NAMES).filter_map(move |n| match n {
        1 => Some(path.clone()),
        2 => path.with_file_name(&format!("{stem} ({slug}){ext}")).ok(),
        n => path
            .with_file_name(&format!("{stem} ({slug} {}){ext}", n - 1))
            .ok(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn chunking() {
        let p = UploadPlan::new(20 * MIB + 1, 8 * MIB).unwrap();
        assert_eq!(p.chunk_count(), 3);
        assert_eq!(p.chunk_range(0), Some((0, 8 * MIB)));
        assert_eq!(p.chunk_range(2), Some((16 * MIB, 4 * MIB + 1)));
        assert_eq!(p.chunk_range(3), None);

        let exact = UploadPlan::new(16 * MIB, 8 * MIB).unwrap();
        assert_eq!(exact.chunk_count(), 2);
        assert_eq!(exact.chunk_range(1), Some((8 * MIB, 8 * MIB)));

        let empty = UploadPlan::new(0, DEFAULT_CHUNK_SIZE).unwrap();
        assert_eq!(empty.chunk_count(), 0);
        assert_eq!(empty.chunk_range(0), None);
    }

    #[test]
    fn limits() {
        assert_eq!(
            UploadPlan::new(1, MIN_CHUNK_SIZE - 1),
            Err(PlanError::ChunkSize)
        );
        assert_eq!(
            UploadPlan::new(1, MAX_CHUNK_SIZE + 1),
            Err(PlanError::ChunkSize)
        );
        assert_eq!(
            UploadPlan::new(u64::MAX, MAX_CHUNK_SIZE),
            Err(PlanError::TooLarge)
        );
    }

    #[test]
    fn client_key_depends_on_all_inputs() {
        let p = RelPath::new("DCIM/a.jpg").unwrap();
        let h = Blake3Hash([1; 32]);
        let k = client_key(&p, 10, &h);
        assert_eq!(k, client_key(&p, 10, &h));
        assert_ne!(k, client_key(&RelPath::new("DCIM/b.jpg").unwrap(), 10, &h));
        assert_ne!(k, client_key(&p, 11, &h));
        assert_ne!(k, client_key(&p, 10, &Blake3Hash([2; 32])));
    }

    #[test]
    fn conflict_candidates_never_reuse_the_name() {
        let p = RelPath::new("DCIM/IMG_0042.jpg").unwrap();
        let c: Vec<String> = conflict_candidates(&p, "pixel-8")
            .take(4)
            .map(|p| p.to_string())
            .collect();
        assert_eq!(
            c,
            [
                "DCIM/IMG_0042.jpg",
                "DCIM/IMG_0042 (pixel-8).jpg",
                "DCIM/IMG_0042 (pixel-8 2).jpg",
                "DCIM/IMG_0042 (pixel-8 3).jpg"
            ]
        );
        let dot = RelPath::new(".profile").unwrap();
        assert_eq!(
            conflict_candidates(&dot, "q").nth(1).unwrap().as_str(),
            ".profile (q)"
        );
        let noext = RelPath::new("README").unwrap();
        assert_eq!(
            conflict_candidates(&noext, "q").nth(1).unwrap().as_str(),
            "README (q)"
        );
        assert_eq!(
            conflict_candidates(&p, "x").count(),
            MAX_CONFLICT_NAMES as usize
        );
    }
}
