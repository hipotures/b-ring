//! Step 6 gate: Range streaming (`206`, `416`, `If-Range`, `ETag`) and
//! signed stream URLs for players that cannot send headers.

mod common;

use std::fs;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use bring_core::RelPath;
use bring_core::api::{ErrorBody, ErrorCode, StreamUrlResponse};
use bring_core::hash::hash_bytes;
use bringd::stream::{StreamClaims, sign_token};
use bringd::{devices, store};
use common::{TestRing, parse, send, start_with, wait_for};

const SIZE: usize = 10_000;

fn content() -> Vec<u8> {
    (0..SIZE).map(|i| (i * 7 % 256) as u8).collect()
}

/// Starts a Ring holding `Download/video.mp4`; returns it and the file id.
async fn ring_with_video(debounce_ms: u64) -> (TestRing, i64) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ring");
    fs::create_dir_all(root.join("Download")).unwrap();
    fs::write(root.join("Download/video.mp4"), content()).unwrap();
    let t = start_with(dir, |c| c.debounce_ms = debounce_ms).await;
    let p = RelPath::new("Download/video.mp4").unwrap();
    wait_for(&t.ring, "hashed", |c| {
        store::file_by_path(c, &p)
            .unwrap()
            .is_some_and(|r| r.blake3.is_some())
    })
    .await;
    let id = store::file_by_path(&t.ring.db.lock(), &p)
        .unwrap()
        .unwrap()
        .id;
    (t, id)
}

async fn get(
    t: &TestRing,
    id: i64,
    headers: &[(header::HeaderName, &str)],
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut b = Request::get(format!("/api/v1/files/{id}/content"));
    for (k, v) in headers {
        b = b.header(k, *v);
    }
    t.call(b, Body::empty()).await
}

#[tokio::test]
async fn range_requests() {
    let (t, id) = ring_with_video(100).await;
    let data = content();
    let etag = format!("\"{}\"", hash_bytes(&data));

    let (s, h, body) = get(&t, id, &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, data);
    assert_eq!(h[header::ACCEPT_RANGES], "bytes");
    assert_eq!(h[header::CONTENT_LENGTH], SIZE.to_string().as_str());
    assert_eq!(h[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(h[header::ETAG], etag.as_str());

    let cases: [(&str, usize, usize); 5] = [
        ("bytes=0-", 0, SIZE - 1),
        ("bytes=-500", SIZE - 500, SIZE - 1),
        ("bytes=100-199", 100, 199),
        ("bytes=9990-20000", 9990, SIZE - 1),
        ("bytes=9999-9999", 9999, 9999),
    ];
    for (range, start, end) in cases {
        let (s, h, body) = get(&t, id, &[(header::RANGE, range)]).await;
        assert_eq!(s, StatusCode::PARTIAL_CONTENT, "{range}");
        assert_eq!(body, &data[start..=end], "{range}");
        assert_eq!(
            h[header::CONTENT_RANGE],
            format!("bytes {start}-{end}/{SIZE}").as_str(),
            "{range}"
        );
        assert_eq!(
            h[header::CONTENT_LENGTH],
            (end - start + 1).to_string().as_str()
        );
    }

    for range in ["bytes=10000-", "bytes=-0"] {
        let (s, h, body) = get(&t, id, &[(header::RANGE, range)]).await;
        assert_eq!(s, StatusCode::RANGE_NOT_SATISFIABLE, "{range}");
        assert_eq!(h[header::CONTENT_RANGE], format!("bytes */{SIZE}").as_str());
        assert_eq!(
            parse::<ErrorBody>("416", s, &body).error,
            ErrorCode::RangeNotSatisfiable
        );
    }

    // Multi-range is served in full.
    let (s, _, body) = get(&t, id, &[(header::RANGE, "bytes=0-1,5-6")]).await;
    assert_eq!((s, body.len()), (StatusCode::OK, SIZE));

    // If-Range: matching ETag honours the range, anything else sends it all.
    let (s, _, body) = get(
        &t,
        id,
        &[(header::RANGE, "bytes=0-9"), (header::IF_RANGE, &etag)],
    )
    .await;
    assert_eq!((s, body.len()), (StatusCode::PARTIAL_CONTENT, 10));
    let (s, _, body) = get(
        &t,
        id,
        &[
            (header::RANGE, "bytes=0-9"),
            (header::IF_RANGE, "\"stale\""),
        ],
    )
    .await;
    assert_eq!((s, body.len()), (StatusCode::OK, SIZE));
    let weak = format!("W/{etag}");
    let (s, _, _) = get(
        &t,
        id,
        &[(header::RANGE, "bytes=0-9"), (header::IF_RANGE, &weak)],
    )
    .await;
    assert_eq!(s, StatusCode::OK, "weak validators never satisfy If-Range");

    // If-None-Match.
    let (s, h, body) = get(&t, id, &[(header::IF_NONE_MATCH, &etag)]).await;
    assert_eq!(s, StatusCode::NOT_MODIFIED);
    assert_eq!(h[header::ETAG], etag.as_str());
    assert!(body.is_empty());

    // HEAD: headers only.
    let (s, h, body) = t
        .call(
            Request::head(format!("/api/v1/files/{id}/content")),
            Body::empty(),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h[header::CONTENT_LENGTH], SIZE.to_string().as_str());
    assert!(body.is_empty());

    // Unknown file.
    let (s, _, _) = get(&t, id + 1000, &[]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stream_urls() {
    let (t, id) = ring_with_video(100).await;
    let router = t.router();
    let anon = |uri: String, range: Option<&'static str>| {
        let mut b = Request::get(uri);
        if let Some(r) = range {
            b = b.header(header::RANGE, r);
        }
        b.body(Body::empty()).unwrap()
    };

    // No credentials at all.
    let (s, _, _) = send(&router, anon(format!("/api/v1/files/{id}/content"), None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let (s, su): (_, StreamUrlResponse) = t
        .post_json(&format!("/api/v1/files/{id}/stream-url"), &())
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(su.expires_at > devices::now_secs());
    let (s, _, body) = send(&router, anon(su.url.clone(), Some("bytes=0-99"))).await;
    assert_eq!(
        (s, body.as_slice()),
        (StatusCode::PARTIAL_CONTENT, &content()[..100])
    );

    // Tampered token.
    let mut tampered = su.url.clone();
    let last = tampered.pop().unwrap();
    tampered.push(if last == 'A' { 'B' } else { 'A' });
    let (s, _, _) = send(&router, anon(tampered, None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // A token for one file does not open another.
    let other = su
        .url
        .replacen(&format!("/files/{id}/"), &format!("/files/{}/", id + 1), 1);
    let (s, _, _) = send(&router, anon(other, None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Expired token.
    let key: [u8; 32] = bring_core::db::ring_meta_blob(&t.ring.db.lock(), "stream_key").unwrap();
    let me = devices::by_token(&t.ring.db.lock(), &t.token)
        .unwrap()
        .unwrap();
    let expired = sign_token(
        &key,
        StreamClaims {
            file_id: id,
            device_id: me.id,
            expires_at: devices::now_secs() - 1,
        },
    );
    let (s, _, _) = send(
        &router,
        anon(format!("/api/v1/files/{id}/content?st={expired}"), None),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // Revoking the device kills its stream URLs.
    devices::revoke(&t.ring.db.lock(), me.id, 1).unwrap();
    let (s, _, _) = send(&router, anon(su.url, None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stale_index_is_refused() {
    // Long debounce: the watcher will not catch up during this test.
    let (t, id) = ring_with_video(60_000).await;
    fs::write(
        t.root.join("Download/video.mp4"),
        b"rewritten on the desktop",
    )
    .unwrap();
    let (s, _, body) = get(&t, id, &[]).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(
        parse::<ErrorBody>("409", s, &body).error,
        ErrorCode::Conflict
    );

    fs::remove_file(t.root.join("Download/video.mp4")).unwrap();
    let (s, _, _) = get(&t, id, &[]).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
