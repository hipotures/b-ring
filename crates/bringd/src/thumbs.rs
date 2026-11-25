//! Thumbnail generation and the content-addressed thumbnail store.
//!
//! - Images the `image` crate can decode are thumbnailed in-process, with the
//!   EXIF orientation applied (phone photos are often stored rotated).
//! - Videos, and image formats `image` cannot decode (HEIC/HEIF/AVIF), go
//!   through an `ffmpeg` subprocess that emits a single PNG frame.
//! - Output: JPEG, longest side at most [`THUMB_MAX`] px, stored as
//!   `<data_dir>/thumbs/<blake3-of-jpeg>.jpg`.
//!
//! Failures never propagate: an unthumbable file just has no thumbnail.

use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use bring_core::Blake3Hash;
use bring_core::hash::hash_bytes;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader};
use tracing::{debug, warn};

/// Longest side of a thumbnail, in pixels.
pub const THUMB_MAX: u32 = 320;
/// JPEG quality of thumbnails.
pub const JPEG_QUALITY: u8 = 80;
/// Upper bound for one ffmpeg run.
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(30);

/// How a file can be thumbnailed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Decodable by the `image` crate.
    Image,
    /// A still image only ffmpeg can decode.
    FfmpegImage,
    Video,
}

fn source_for(mime: &str) -> Option<Source> {
    match mime {
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/bmp" | "image/tiff" => {
            Some(Source::Image)
        }
        m if m.starts_with("image/") && m != "image/svg+xml" => Some(Source::FfmpegImage),
        // `.ts` is overwhelmingly TypeScript in dev folders; mime_guess tags it as MPEG-TTS.
        m if m.starts_with("video/") && m != "video/vnd.dlna.mpeg-tts" => Some(Source::Video),
        _ => None,
    }
}

/// True if files of this MIME type can get a thumbnail.
pub fn is_thumbable(mime: Option<&str>) -> bool {
    mime.and_then(source_for).is_some()
}

/// Content-addressed thumbnail store.
#[derive(Debug, Clone)]
pub struct ThumbStore {
    dir: PathBuf,
    ffmpeg: PathBuf,
}

impl ThumbStore {
    pub fn new(dir: PathBuf, ffmpeg: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir, ffmpeg })
    }

    /// Where the thumbnail with this hash lives.
    pub fn path_of(&self, hash: &Blake3Hash) -> PathBuf {
        self.dir.join(format!("{hash}.jpg"))
    }

    /// Generates and stores a thumbnail for `file`. Blocking.
    ///
    /// Returns `None` if the type is not supported or generation failed.
    pub fn generate(&self, file: &Path, mime: Option<&str>) -> Option<Blake3Hash> {
        let source = mime.and_then(source_for)?;
        let img = match source {
            Source::Image => decode_oriented(file)
                .map_err(|e| debug!(file = %file.display(), error = %e, "image decode failed"))
                .ok()
                // Some "jpegs" are really HEIC etc.; let ffmpeg try.
                .or_else(|| self.ffmpeg_frame(file, false)),
            Source::FfmpegImage => self.ffmpeg_frame(file, false),
            Source::Video => self.ffmpeg_frame(file, true),
        }?;
        let jpeg = match render(&img) {
            Ok(j) => j,
            Err(e) => {
                warn!(file = %file.display(), error = %e, "thumbnail encode failed");
                return None;
            }
        };
        let hash = hash_bytes(&jpeg);
        match self.store(&hash, &jpeg) {
            Ok(()) => Some(hash),
            Err(e) => {
                warn!(error = %e, "cannot store thumbnail");
                None
            }
        }
    }

    fn store(&self, hash: &Blake3Hash, jpeg: &[u8]) -> io::Result<()> {
        let dest = self.path_of(hash);
        if dest.exists() {
            return Ok(()); // content-addressed: same bytes already there
        }
        let tmp = self.dir.join(format!(".{hash}.{}.tmp", std::process::id()));
        std::fs::write(&tmp, jpeg)?;
        std::fs::rename(&tmp, &dest)
    }

    /// Extracts one frame with ffmpeg and decodes it.
    fn ffmpeg_frame(&self, file: &Path, video: bool) -> Option<DynamicImage> {
        // For videos, skip the first second (often black); fall back to the
        // very first frame for clips shorter than that.
        let attempts: &[Option<&str>] = if video { &[Some("1"), None] } else { &[None] };
        for seek in attempts {
            match self.run_ffmpeg(file, *seek) {
                Ok(png) if !png.is_empty() => match image::load_from_memory(&png) {
                    Ok(img) => return Some(img),
                    Err(e) => debug!(error = %e, "ffmpeg output not decodable"),
                },
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    debug!(ffmpeg = %self.ffmpeg.display(), "ffmpeg not available");
                    return None;
                }
                Err(e) => {
                    debug!(file = %file.display(), error = %e, "ffmpeg failed");
                }
            }
        }
        None
    }

    fn run_ffmpeg(&self, file: &Path, seek: Option<&str>) -> io::Result<Vec<u8>> {
        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-protocol_whitelist",
            "file",
        ]);
        if let Some(s) = seek {
            cmd.args(["-ss", s]);
        }
        cmd.arg("-i")
            .arg(file)
            .args([
                "-frames:v",
                "1",
                "-vf",
                // Downscale early to keep the PNG small; final size is ours.
                "scale=640:640:force_original_aspect_ratio=decrease",
                "-f",
                "image2pipe",
                "-c:v",
                "png",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            stdout.read_to_end(&mut buf).map(|_| buf)
        });
        let deadline = Instant::now() + FFMPEG_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(io::ErrorKind::TimedOut, "ffmpeg timed out"));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let out = reader
            .join()
            .map_err(|_| io::Error::other("reader thread panicked"))??;
        if !status.success() {
            return Err(io::Error::other(format!("ffmpeg exited with {status}")));
        }
        Ok(out)
    }
}

fn decode_oriented(file: &Path) -> image::ImageResult<DynamicImage> {
    let mut decoder = ImageReader::open(file)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation()?;
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// Scales down (never up) and encodes as JPEG.
fn render(img: &DynamicImage) -> image::ImageResult<Vec<u8>> {
    let small = if img.width() > THUMB_MAX || img.height() > THUMB_MAX {
        img.thumbnail(THUMB_MAX, THUMB_MAX)
    } else {
        img.clone()
    };
    let rgb = small.to_rgb8();
    let mut out = Cursor::new(Vec::new());
    JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY).encode_image(&rgb)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, Rgb, RgbImage};

    fn store(dir: &Path, ffmpeg: &str) -> ThumbStore {
        ThumbStore::new(dir.join("thumbs"), PathBuf::from(ffmpeg)).unwrap()
    }

    fn write_png(path: &Path, w: u32, h: u32, exif: Option<Vec<u8>>) {
        let img = RgbImage::from_fn(w, h, |x, _| Rgb([(x % 256) as u8, 80, 160]));
        let mut buf = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut buf);
        if let Some(exif) = exif {
            enc.set_exif_metadata(exif).unwrap();
        }
        enc.write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .unwrap();
        std::fs::write(path, buf).unwrap();
    }

    /// Minimal big-endian TIFF/EXIF block with only an Orientation tag.
    fn exif_orientation(value: u16) -> Vec<u8> {
        let mut e = b"MM\0\x2a\0\0\0\x08".to_vec(); // header, IFD at offset 8
        e.extend_from_slice(&1u16.to_be_bytes()); // one entry
        e.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
        e.extend_from_slice(&3u16.to_be_bytes()); // SHORT
        e.extend_from_slice(&1u32.to_be_bytes()); // count
        e.extend_from_slice(&value.to_be_bytes());
        e.extend_from_slice(&[0, 0]); // value padding
        e.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
        e
    }

    fn dims(store: &ThumbStore, h: &Blake3Hash) -> (u32, u32) {
        let img = image::open(store.path_of(h)).unwrap();
        (img.width(), img.height())
    }

    #[test]
    fn scales_down_keeping_aspect_ratio() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), "/nonexistent/ffmpeg");
        let src = d.path().join("wide.png");
        write_png(&src, 1000, 500, None);
        let h = s.generate(&src, Some("image/png")).unwrap();
        assert_eq!(dims(&s, &h), (320, 160));
        // Stored bytes are what the hash names, and are a JPEG.
        let bytes = std::fs::read(s.path_of(&h)).unwrap();
        assert_eq!(hash_bytes(&bytes), h);
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn never_upscales() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), "/nonexistent/ffmpeg");
        let src = d.path().join("small.png");
        write_png(&src, 100, 40, None);
        let h = s.generate(&src, Some("image/png")).unwrap();
        assert_eq!(dims(&s, &h), (100, 40));
    }

    #[test]
    fn applies_exif_orientation() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), "/nonexistent/ffmpeg");
        let src = d.path().join("rotated.png");
        write_png(&src, 640, 320, Some(exif_orientation(6))); // rotate 90° CW
        let h = s.generate(&src, Some("image/png")).unwrap();
        assert_eq!(dims(&s, &h), (160, 320));
    }

    #[test]
    fn unsupported_and_corrupt_inputs_yield_none() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), "/nonexistent/ffmpeg");
        let txt = d.path().join("a.txt");
        std::fs::write(&txt, b"hello").unwrap();
        assert_eq!(s.generate(&txt, Some("text/plain")), None);
        assert_eq!(s.generate(&txt, None), None);
        let bad = d.path().join("bad.jpg");
        std::fs::write(&bad, b"\xFF\xD8 definitely not a jpeg").unwrap();
        assert_eq!(s.generate(&bad, Some("image/jpeg")), None);
    }

    #[test]
    fn missing_ffmpeg_is_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), "/nonexistent/ffmpeg");
        let vid = d.path().join("clip.mp4");
        std::fs::write(&vid, b"not really a video").unwrap();
        assert_eq!(s.generate(&vid, Some("video/mp4")), None);
        assert_eq!(s.generate(&vid, Some("image/heic")), None);
    }

    fn have_ffmpeg() -> bool {
        Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    #[test]
    fn video_frame_via_ffmpeg() {
        if !have_ffmpeg() {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), "ffmpeg");
        for (name, secs) in [("long.mp4", "2"), ("short.mp4", "0.2")] {
            let vid = d.path().join(name);
            let ok = Command::new("ffmpeg")
                .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
                .arg(format!("testsrc=size=640x360:rate=10:duration={secs}"))
                .args(["-pix_fmt", "yuv420p"])
                .arg(&vid)
                .status()
                .unwrap()
                .success();
            assert!(ok, "could not create test video");
            let h = s
                .generate(&vid, Some("video/mp4"))
                .unwrap_or_else(|| panic!("no thumbnail for {name}"));
            assert_eq!(dims(&s, &h), (320, 180), "{name}");
        }
    }
}
