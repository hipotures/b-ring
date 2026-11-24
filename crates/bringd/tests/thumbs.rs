//! Step 4 gate: thumbnails are generated for indexed media and served by
//! `GET /thumbs/{hash}`; missing ffmpeg means "no thumbnail", not an error.

mod common;

use std::fs;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use bring_core::RelPath;
use bring_core::hash::hash_bytes;
use bringd::store;
use common::{send, start_with, wait_for};

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 99]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

#[tokio::test]
async fn images_get_thumbnails_served_immutably() {
    let t = start_with(tempfile::tempdir().unwrap(), |c| {
        c.ffmpeg = PathBuf::from("/nonexistent/ffmpeg");
    })
    .await;
    let data = png(800, 600);
    fs::write(t.root.join("DCIM/photo.png"), &data).unwrap();
    fs::write(t.root.join("DCIM/clip.mp4"), b"not a real video").unwrap();
    fs::write(t.root.join("Download/notes.txt"), b"plain").unwrap();

    let path = RelPath::new("DCIM/photo.png").unwrap();
    wait_for(&t.ring, "thumbnail", |c| {
        store::file_by_path(c, &path)
            .unwrap()
            .is_some_and(|r| r.thumb.is_some())
    })
    .await;
    wait_for(&t.ring, "others hashed", |c| {
        ["DCIM/clip.mp4", "Download/notes.txt"].iter().all(|p| {
            store::file_by_path(c, &RelPath::new(p).unwrap())
                .unwrap()
                .is_some_and(|r| r.blake3.is_some())
        })
    })
    .await;

    let (row, clip, notes) = {
        let c = t.ring.db.lock();
        let get = |p: &str| {
            store::file_by_path(&c, &RelPath::new(p).unwrap())
                .unwrap()
                .unwrap()
        };
        (
            get("DCIM/photo.png"),
            get("DCIM/clip.mp4"),
            get("Download/notes.txt"),
        )
    };
    assert_eq!(row.blake3, Some(hash_bytes(&data)));
    assert_eq!(
        clip.thumb, None,
        "no ffmpeg -> no thumbnail, but still indexed"
    );
    assert_eq!(notes.thumb, None);

    let thumb = row.thumb.unwrap();
    let router = t.ring.router();
    let (status, headers, body) = send(
        &router,
        Request::get(format!("/api/v1/thumbs/{thumb}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    assert!(
        headers[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    assert_eq!(hash_bytes(&body), thumb, "content-addressed");
    let decoded = image::load_from_memory(&body).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (320, 240));

    let (status, _, _) = send(
        &router,
        Request::get(format!("/api/v1/thumbs/{}", "0".repeat(64)))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(
        &router,
        Request::get("/api/v1/thumbs/..%2F..%2Fetc%2Fpasswd")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
