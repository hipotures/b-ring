//! Core data model: validated Ring paths, BLAKE3 hashes and file states.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use unicode_normalization::{UnicodeNormalization, is_nfc};

// ---------------------------------------------------------------------------
// RelPath
// ---------------------------------------------------------------------------

/// A validated path relative to the Ring root.
///
/// Invariants (enforced by every constructor):
/// - non-empty, relative (no leading `/`), `/`-separated;
/// - no empty, `.` or `..` segments (so it can never escape the root);
/// - no NUL bytes;
/// - Unicode NFC.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(String);

/// Why a string is not a valid [`RelPath`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelPathError {
    #[error("path is empty")]
    Empty,
    #[error("path must be relative")]
    Absolute,
    #[error("path contains an empty segment")]
    EmptySegment,
    #[error("path contains a '.' or '..' segment")]
    DotSegment,
    #[error("path contains a NUL byte")]
    Nul,
    #[error("path is longer than {} bytes", RelPath::MAX_LEN)]
    TooLong,
    #[error("path is not valid UTF-8")]
    NotUtf8,
    #[error("path is not in Unicode NFC form")]
    NotNfc,
    #[error("path is outside the Ring root")]
    OutsideRoot,
}

impl RelPath {
    /// Maximum length in bytes (Linux `PATH_MAX`).
    pub const MAX_LEN: usize = 4096;

    /// Validates `s`, normalising it to NFC first.
    ///
    /// Use this for paths coming from clients; use [`RelPath::from_fs_path`]
    /// for names read from disk.
    pub fn new(s: &str) -> Result<Self, RelPathError> {
        let normalized: String = s.nfc().collect();
        Self::validate(&normalized)?;
        Ok(Self(normalized))
    }

    /// Builds a `RelPath` from an on-disk path under `root`.
    ///
    /// Unlike [`RelPath::new`] this does *not* normalise: a name that is not
    /// already NFC is rejected, because the normalised form would not refer to
    /// the same file on disk.
    pub fn from_fs_path(root: &Path, path: &Path) -> Result<Self, RelPathError> {
        let rel = path
            .strip_prefix(root)
            .map_err(|_| RelPathError::OutsideRoot)?;
        let s = rel.to_str().ok_or(RelPathError::NotUtf8)?;
        if !is_nfc(s) {
            return Err(RelPathError::NotNfc);
        }
        Self::validate(s)?;
        Ok(Self(s.to_owned()))
    }

    fn validate(s: &str) -> Result<(), RelPathError> {
        if s.is_empty() {
            return Err(RelPathError::Empty);
        }
        if s.starts_with('/') {
            return Err(RelPathError::Absolute);
        }
        if s.len() > Self::MAX_LEN {
            return Err(RelPathError::TooLong);
        }
        if s.contains('\0') {
            return Err(RelPathError::Nul);
        }
        for seg in s.split('/') {
            if seg.is_empty() {
                return Err(RelPathError::EmptySegment);
            }
            if seg == "." || seg == ".." {
                return Err(RelPathError::DotSegment);
            }
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// The logical folder: the first path segment (`DCIM`, `Download`, ...).
    pub fn folder(&self) -> &str {
        self.0.split('/').next().unwrap_or(&self.0)
    }

    /// The last path segment.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// The parent directory, or `None` for a top-level entry.
    pub fn parent(&self) -> Option<RelPath> {
        self.0.rsplit_once('/').map(|(p, _)| Self(p.to_owned()))
    }

    /// Returns a sibling path with the last segment replaced by `name`.
    pub fn with_file_name(&self, name: &str) -> Result<RelPath, RelPathError> {
        match self.parent() {
            Some(p) => RelPath::new(&format!("{}/{name}", p.0)),
            None => RelPath::new(name),
        }
    }

    /// True if `self` is `dir` itself or lies anywhere below it.
    pub fn is_within(&self, dir: &RelPath) -> bool {
        self.0 == dir.0
            || (self.0.len() > dir.0.len()
                && self.0.starts_with(&dir.0)
                && self.0.as_bytes()[dir.0.len()] == b'/')
    }

    /// The absolute on-disk location of this path under `root`.
    pub fn to_fs_path(&self, root: &Path) -> PathBuf {
        root.join(&self.0)
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for RelPath {
    type Err = RelPathError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl Serialize for RelPath {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        RelPath::new(&s).map_err(serde::de::Error::custom)
    }
}

impl ToSql for RelPath {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

impl FromSql for RelPath {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        RelPath::new(s).map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

// ---------------------------------------------------------------------------
// Blake3Hash
// ---------------------------------------------------------------------------

/// A 32-byte BLAKE3 digest. Serialised as lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Blake3Hash(pub [u8; 32]);

/// A string that is not 64 hex characters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid BLAKE3 hash: expected 64 hex characters")]
pub struct HashParseError;

impl Blake3Hash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push(char::from(HEX[usize::from(b >> 4)]));
            s.push(char::from(HEX[usize::from(b & 0x0f)]));
        }
        s
    }

    pub fn from_hex(s: &str) -> Result<Self, HashParseError> {
        let bytes = s.as_bytes();
        if bytes.len() != 64 {
            return Err(HashParseError);
        }
        let mut out = [0u8; 32];
        for (i, [hi, lo]) in bytes.as_chunks::<2>().0.iter().enumerate() {
            out[i] = (hex_val(*hi)? << 4) | hex_val(*lo)?;
        }
        Ok(Self(out))
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn hex_val(c: u8) -> Result<u8, HashParseError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(HashParseError),
    }
}

impl From<blake3::Hash> for Blake3Hash {
    fn from(h: blake3::Hash) -> Self {
        Self(*h.as_bytes())
    }
}

impl fmt::Debug for Blake3Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Blake3Hash({})", self.to_hex())
    }
}

impl fmt::Display for Blake3Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for Blake3Hash {
    type Err = HashParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl Serialize for Blake3Hash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Blake3Hash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

impl ToSql for Blake3Hash {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Blob(&self.0)))
    }
}

impl FromSql for Blake3Hash {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let blob = value.as_blob()?;
        let arr: [u8; 32] = blob.try_into().map_err(|_| FromSqlError::InvalidBlobSize {
            expected_size: 32,
            blob_size: blob.len(),
        })?;
        Ok(Self(arr))
    }
}

// ---------------------------------------------------------------------------
// FileState
// ---------------------------------------------------------------------------

/// Where a file lives, as shown on a Moon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileState {
    /// Only on the Moon (awaiting home Wi-Fi sync).
    #[serde(rename = "local-only")]
    LocalOnly,
    /// A verified copy exists on both the Moon and the Ring.
    #[serde(rename = "synced")]
    Synced,
    /// Only on the Ring (offloaded or added on the desktop).
    #[serde(rename = "remote-only")]
    RemoteOnly,
}

impl FileState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LocalOnly => "local-only",
            Self::Synced => "synced",
            Self::RemoteOnly => "remote-only",
        }
    }
}

/// A string that is not a known [`FileState`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown file state: {0:?}")]
pub struct FileStateParseError(pub String);

impl FromStr for FileState {
    type Err = FileStateParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local-only" => Ok(Self::LocalOnly),
            "synced" => Ok(Self::Synced),
            "remote-only" => Ok(Self::RemoteOnly),
            other => Err(FileStateParseError(other.to_owned())),
        }
    }
}

impl fmt::Display for FileState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl ToSql for FileState {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Text(
            self.as_str().as_bytes(),
        )))
    }
}

impl FromSql for FileState {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn relpath_accepts_normal_paths() {
        let p = RelPath::new("DCIM/Camera/IMG_0001.jpg").unwrap();
        assert_eq!(p.folder(), "DCIM");
        assert_eq!(p.file_name(), "IMG_0001.jpg");
        assert_eq!(p.parent().unwrap().as_str(), "DCIM/Camera");
        assert_eq!(RelPath::new("Inbox").unwrap().parent(), None);
    }

    #[test]
    fn relpath_rejects_escapes_and_garbage() {
        use RelPathError::*;
        for (input, err) in [
            ("", Empty),
            ("/etc/passwd", Absolute),
            ("a//b", EmptySegment),
            ("a/", EmptySegment),
            ("../x", DotSegment),
            ("a/../../x", DotSegment),
            ("a/./b", DotSegment),
            (".", DotSegment),
            ("a\0b", Nul),
        ] {
            assert_eq!(RelPath::new(input), Err(err), "input {input:?}");
        }
        assert_eq!(RelPath::new(&"a".repeat(5000)), Err(TooLong));
    }

    #[test]
    fn relpath_allows_dotfiles_and_dots_inside_names() {
        assert!(RelPath::new(".hidden/..file...").is_ok());
        assert!(RelPath::new("a/b.c/d").is_ok());
    }

    #[test]
    fn relpath_normalises_to_nfc() {
        let nfd = "Zdj\u{0119}cia/z\u{0061}\u{0301}.jpg"; // 'á' decomposed
        let p = RelPath::new(nfd).unwrap();
        assert!(is_nfc(p.as_str()));
        assert_eq!(p.as_str(), "Zdjęcia/z\u{00e1}.jpg");
    }

    #[test]
    fn relpath_from_fs_path_is_strict() {
        let root = Path::new("/home/u/B-Ring");
        let ok = RelPath::from_fs_path(root, Path::new("/home/u/B-Ring/DCIM/a.jpg")).unwrap();
        assert_eq!(ok.as_str(), "DCIM/a.jpg");
        assert_eq!(
            RelPath::from_fs_path(root, Path::new("/home/u/other/a.jpg")),
            Err(RelPathError::OutsideRoot)
        );
        assert_eq!(
            RelPath::from_fs_path(root, Path::new("/home/u/B-Ring/a\u{0301}.jpg")),
            Err(RelPathError::NotNfc)
        );
    }

    #[test]
    fn relpath_is_within() {
        let dir = RelPath::new("DCIM/Cam").unwrap();
        assert!(RelPath::new("DCIM/Cam").unwrap().is_within(&dir));
        assert!(RelPath::new("DCIM/Cam/a.jpg").unwrap().is_within(&dir));
        assert!(!RelPath::new("DCIM/Camera/a.jpg").unwrap().is_within(&dir));
        assert!(!RelPath::new("DCIM").unwrap().is_within(&dir));
    }

    #[test]
    fn hash_hex_roundtrip_and_serde() {
        let h = Blake3Hash::from(blake3::hash(b"b-ring"));
        assert_eq!(Blake3Hash::from_hex(&h.to_hex()).unwrap(), h);
        assert_eq!(Blake3Hash::from_hex(&h.to_hex().to_uppercase()).unwrap(), h);
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(json, format!("\"{}\"", h.to_hex()));
        assert!(Blake3Hash::from_hex("abc").is_err());
        assert!(Blake3Hash::from_hex(&"g".repeat(64)).is_err());
    }

    #[test]
    fn file_state_strings() {
        for s in [
            FileState::LocalOnly,
            FileState::Synced,
            FileState::RemoteOnly,
        ] {
            assert_eq!(s.as_str().parse::<FileState>().unwrap(), s);
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
    }

    fn segment() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9 _.\\-ąćęłńóśźżÉ]{1,12}".prop_filter("dot segment", |s| s != "." && s != "..")
    }

    proptest! {
        #[test]
        fn relpath_valid_segments_roundtrip(segs in prop::collection::vec(segment(), 1..6)) {
            let joined = segs.join("/");
            let p = RelPath::new(&joined).unwrap();
            let expected: String = joined.nfc().collect();
            prop_assert_eq!(p.as_str(), expected.as_str());
            prop_assert_eq!(p.segments().count(), segs.len());
        }

        #[test]
        fn relpath_never_escapes(s in "\\PC{0,40}") {
            if let Ok(p) = RelPath::new(&s) {
                prop_assert!(!p.as_str().starts_with('/'));
                prop_assert!(is_nfc(p.as_str()));
                for seg in p.segments() {
                    prop_assert!(!seg.is_empty() && seg != "." && seg != "..");
                }
                // Joining onto a root never leaves the root.
                let root = Path::new("/ring");
                prop_assert!(p.to_fs_path(root).starts_with(root));
            }
        }
    }
}
