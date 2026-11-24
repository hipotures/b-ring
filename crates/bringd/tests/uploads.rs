//! Step 5 gate: resumable chunked uploads with verified atomic commit.

mod common;

use std::fs;
use std::os::unix::fs::MetadataExt;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use bring_core::api::{
    CHUNK_HASH_HEADER, CreateUploadRequest, CreateUploadResponse, ErrorBody, ErrorCode,
    IndexResponse, UploadAck, UploadState, UploadStatus,
};
use bring_core::hash::hash_bytes;
use bring_core::transfer::MIN_CHUNK_SIZE;
use bring_core::{Blake3Hash, RelPath};
use bringd::{devices, uploads};
use common::{TestRing, parse, send, start};

const CHUNK: u64 = MIN_CHUNK_SIZE; // 64 KiB keeps tests fast
const MTIME_NS: i64 = 1_700_000_000_123_456_789;

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn request(path: &str, content: &[u8]) -> CreateUploadRequest {
    CreateUploadRequest {
        path: RelPath::new(path).unwrap(),
        size: content.len() as u64,
        mtime_ns: Some(MTIME_NS),
        blake3: hash_bytes(content),
        chunk_size: Some(CHUNK),
    }
}

async fn create(t: &TestRing, req: &CreateUploadRequest) -> (StatusCode, CreateUploadResponse) {
    t.post_json("/api/v1/uploads", req).await
}

fn open_id(resp: &CreateUploadResponse) -> String {
    match resp {
        CreateUploadResponse::Open { upload_id, .. } => upload_id.clone(),
        other => panic!("expected an open upload, got {other:?}"),
    }
}

fn chunk_of(content: &[u8], idx: usize) -> &[u8] {
    let start = idx * CHUNK as usize;
    &content[start..content.len().min(start + CHUNK as usize)]
}

async fn put_chunk_raw(
    t: &TestRing,
    id: &str,
    idx: u32,
    body: &[u8],
    claimed: Option<Blake3Hash>,
) -> (StatusCode, Vec<u8>) {
    let mut b = Request::post(format!("/api/v1/uploads/{id}/chunks/{idx}"));
    if let Some(h) = claimed {
        b = b.header(CHUNK_HASH_HEADER, h.to_hex());
    }
    let (s, _, body) = t.call(b, body.to_vec()).await;
    (s, body)
}

async fn put_chunk(t: &TestRing, id: &str, content: &[u8], idx: usize) -> StatusCode {
    let c = chunk_of(content, idx);
    put_chunk_raw(t, id, idx as u32, c, Some(hash_bytes(c)))
        .await
        .0
}

async fn commit(t: &TestRing, id: &str) -> (StatusCode, Vec<u8>) {
    let (s, _, body) = t
        .call(
            Request::post(format!("/api/v1/uploads/{id}/commit")),
            Body::empty(),
        )
        .await;
    (s, body)
}

fn error_code(body: &[u8]) -> ErrorCode {
    parse::<ErrorBody>("error", StatusCode::OK, body).error
}

async fn upload(t: &TestRing, path: &str, content: &[u8]) -> UploadAck {
    let (_, resp) = create(t, &request(path, content)).await;
    let id = open_id(&resp);
    let n = content.len().div_ceil(CHUNK as usize);
    for i in 0..n {
        assert_eq!(put_chunk(t, &id, content, i).await, StatusCode::NO_CONTENT);
    }
    let (s, body) = commit(t, &id).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    parse("commit", s, &body)
}

#[tokio::test]
async fn full_cycle_with_retries_double_commit_and_dedupe() {
    let t = start().await;
    let content = data(150_000, 1); // 3 chunks
    let req = request("DCIM/pixel/IMG_0001.jpg", &content);

    let (s, resp) = create(&t, &req).await;
    assert_eq!(s, StatusCode::CREATED);
    let CreateUploadResponse::Open {
        upload_id: id,
        chunk_count,
        received,
        chunk_size,
    } = resp
    else {
        panic!()
    };
    assert_eq!((chunk_count, chunk_size, received.len()), (3, CHUNK, 0));

    // Out of order, with a retried chunk.
    for i in [1, 0, 0, 2] {
        assert_eq!(
            put_chunk(&t, &id, &content, i).await,
            StatusCode::NO_CONTENT
        );
    }
    let (s, body) = commit(&t, &id).await;
    assert_eq!(s, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let ack: UploadAck = parse("commit", s, &body);
    assert_eq!(ack.upload_id.as_deref(), Some(id.as_str()));
    assert_eq!(ack.path.as_str(), "DCIM/pixel/IMG_0001.jpg");
    assert_eq!(
        (ack.size, ack.blake3),
        (content.len() as u64, hash_bytes(&content))
    );

    // Durable, verified, timestamp preserved, staging cleaned.
    let on_disk = t.root.join("DCIM/pixel/IMG_0001.jpg");
    assert_eq!(fs::read(&on_disk).unwrap(), content);
    let meta = fs::metadata(&on_disk).unwrap();
    assert_eq!(meta.mtime() * 1_000_000_000 + meta.mtime_nsec(), MTIME_NS);
    assert_eq!(meta.nlink(), 1, "staging link removed after commit");
    assert_eq!(
        fs::read_dir(t.root.join(".bring/staging")).unwrap().count(),
        0
    );

    // Visible in the delta stream, attributed to the device.
    let (_, idx): (_, IndexResponse) = t.get_json("/api/v1/index").await;
    let e = idx.entries.iter().find(|e| e.id == ack.file_id).unwrap();
    assert_eq!(e.origin.as_deref(), Some("test-phone"));
    assert_eq!(e.blake3, Some(ack.blake3));
    assert_eq!(e.seq, ack.seq);

    // Double commit returns the same ACK.
    let (s, body) = commit(&t, &id).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(parse::<UploadAck>("commit", s, &body), ack);

    // Creating the same upload again: nothing to send.
    let (s, resp) = create(&t, &req).await;
    assert_eq!(s, StatusCode::OK);
    let CreateUploadResponse::Exists { ack: again } = resp else {
        panic!("{resp:?}")
    };
    assert_eq!((again.file_id, again.blake3), (ack.file_id, ack.blake3));

    // If the file disappears from the Ring, the old ACK is NOT repeated.
    fs::remove_file(&on_disk).unwrap();
    let (s, body) = commit(&t, &id).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(error_code(&body), ErrorCode::Conflict);
}

#[tokio::test]
async fn resume_after_restart() {
    let t = start().await;
    let content = data(200_000, 2); // 4 chunks
    let req = request("Download/big.bin", &content);
    let (_, resp) = create(&t, &req).await;
    let id = open_id(&resp);
    assert_eq!(
        put_chunk(&t, &id, &content, 0).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put_chunk(&t, &id, &content, 2).await,
        StatusCode::NO_CONTENT
    );

    let t = t.restart().await;
    let (s, st): (_, UploadStatus) = t.get_json(&format!("/api/v1/uploads/{id}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (st.state, st.received.as_slice()),
        (UploadState::Open, &[0, 2][..])
    );

    // Re-creating resumes the same upload.
    let (s, resp) = create(&t, &req).await;
    assert_eq!(s, StatusCode::OK);
    let CreateUploadResponse::Open {
        upload_id,
        received,
        ..
    } = resp
    else {
        panic!()
    };
    assert_eq!(
        (upload_id.as_str(), received.as_slice()),
        (id.as_str(), &[0, 2][..])
    );

    for i in [1, 3] {
        assert_eq!(
            put_chunk(&t, &id, &content, i).await,
            StatusCode::NO_CONTENT
        );
    }
    let (s, _) = commit(&t, &id).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(fs::read(t.root.join("Download/big.bin")).unwrap(), content);
}

#[tokio::test]
async fn bad_chunks_are_rejected() {
    let t = start().await;
    let content = data(100_000, 3); // 2 chunks
    let (_, resp) = create(&t, &request("Inbox/a.bin", &content)).await;
    let id = open_id(&resp);
    let c0 = chunk_of(&content, 0);

    // Corrupted in transit: body does not match the claimed hash.
    let mut corrupted = c0.to_vec();
    corrupted[10] ^= 0xFF;
    let (s, body) = put_chunk_raw(&t, &id, 0, &corrupted, Some(hash_bytes(c0))).await;
    assert_eq!(
        (s, error_code(&body)),
        (StatusCode::CONFLICT, ErrorCode::HashMismatch)
    );

    let (s, body) = put_chunk_raw(&t, &id, 0, &c0[..100], Some(hash_bytes(&c0[..100]))).await;
    assert_eq!(
        (s, error_code(&body)),
        (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::BadChunkLength)
    );
    let (s, _) = put_chunk_raw(&t, &id, 9, c0, Some(hash_bytes(c0))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = put_chunk_raw(&t, &id, 0, c0, None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (_, st): (_, UploadStatus) = t.get_json(&format!("/api/v1/uploads/{id}")).await;
    assert!(
        st.received.is_empty(),
        "rejected chunks must not be recorded"
    );

    assert_eq!(
        put_chunk(&t, &id, &content, 0).await,
        StatusCode::NO_CONTENT
    );
    let (s, body) = commit(&t, &id).await;
    assert_eq!(
        (s, error_code(&body)),
        (StatusCode::CONFLICT, ErrorCode::MissingChunks)
    );
    assert!(String::from_utf8_lossy(&body).contains("missing chunks: 1"));
}

#[tokio::test]
async fn whole_file_mismatch_fails_and_allows_retry() {
    let t = start().await;
    let content = data(100_000, 4);
    let mut req = request("DCIM/x.jpg", &content);
    req.blake3 = hash_bytes(b"something else entirely"); // Moon's hash is wrong
    let (_, resp) = create(&t, &req).await;
    let id = open_id(&resp);
    for i in 0..2 {
        assert_eq!(
            put_chunk(&t, &id, &content, i).await,
            StatusCode::NO_CONTENT
        );
    }
    let (s, body) = commit(&t, &id).await;
    assert_eq!(
        (s, error_code(&body)),
        (StatusCode::CONFLICT, ErrorCode::HashMismatch)
    );

    // Nothing reached the Ring; staging is gone; the upload is failed.
    assert!(!t.root.join("DCIM/x.jpg").exists());
    assert_eq!(
        fs::read_dir(t.root.join(".bring/staging")).unwrap().count(),
        0
    );
    let (_, st): (_, UploadStatus) = t.get_json(&format!("/api/v1/uploads/{id}")).await;
    assert_eq!(st.state, UploadState::Failed);
    let (s, _) = commit(&t, &id).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // The same request again creates a *new* upload (no unique-key clash).
    let (s, resp) = create(&t, &req).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_ne!(open_id(&resp), id);
}

#[tokio::test]
async fn never_overwrites_and_lost_ack_is_idempotent() {
    let t = start().await;
    fs::write(t.root.join("DCIM/x.jpg"), b"desktop original").unwrap();

    let phone = data(70_000, 5);
    let ack = upload(&t, "DCIM/x.jpg", &phone).await;
    assert_eq!(ack.path.as_str(), "DCIM/x (test-phone).jpg");
    assert_eq!(
        fs::read(t.root.join("DCIM/x.jpg")).unwrap(),
        b"desktop original"
    );
    assert_eq!(
        fs::read(t.root.join("DCIM/x (test-phone).jpg")).unwrap(),
        phone
    );

    // The Moon lost the ACK and asks again: same file, no duplicate.
    let (s, resp) = create(&t, &request("DCIM/x.jpg", &phone)).await;
    assert_eq!(s, StatusCode::OK);
    let CreateUploadResponse::Exists { ack: again } = resp else {
        panic!("{resp:?}")
    };
    assert_eq!(again.path, ack.path);

    let other = data(70_000, 6);
    let ack2 = upload(&t, "DCIM/x.jpg", &other).await;
    assert_eq!(ack2.path.as_str(), "DCIM/x (test-phone 2).jpg");
}

#[tokio::test]
async fn empty_file_abort_and_reserved_paths() {
    let t = start().await;
    let ack = upload(&t, "Inbox/empty.txt", b"").await;
    assert_eq!(ack.size, 0);
    assert_eq!(fs::read(t.root.join("Inbox/empty.txt")).unwrap(), b"");

    let content = data(1000, 7);
    let (_, resp) = create(&t, &request("Inbox/abort.bin", &content)).await;
    let id = open_id(&resp);
    let (s, _, _) = t
        .call(
            Request::delete(format!("/api/v1/uploads/{id}")),
            Body::empty(),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        fs::read_dir(t.root.join(".bring/staging")).unwrap().count(),
        0
    );
    let (s, _, _) = t
        .call(Request::get(format!("/api/v1/uploads/{id}")), Body::empty())
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _): (_, ErrorBody) = t
        .post_json("/api/v1/uploads", &request(".bring/staging/evil", b"x"))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn auth_and_ownership() {
    let t = start().await;
    let router = t.router();
    for uri in ["/api/v1/index", "/api/v1/uploads/x"] {
        let (s, _, _) = send(&router, Request::get(uri).body(Body::empty()).unwrap()).await;
        assert_eq!(s, StatusCode::UNAUTHORIZED, "{uri}");
    }

    let content = data(1000, 8);
    let (_, resp) = create(&t, &request("Inbox/mine.bin", &content)).await;
    let id = open_id(&resp);

    // Another device cannot see or touch it.
    let (_, other) = devices::register(&t.ring.db.lock(), [9; 16], "Other", 0).unwrap();
    let (s, _, _) = send(
        &router,
        Request::get(format!("/api/v1/uploads/{id}"))
            .header("authorization", format!("Bearer {other}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Revoked token is rejected.
    let me = devices::by_token(&t.ring.db.lock(), &t.token)
        .unwrap()
        .unwrap();
    devices::revoke(&t.ring.db.lock(), me.id, 1).unwrap();
    let (s, _, _) = t.call(Request::get("/api/v1/index"), Body::empty()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cleanup_removes_abandoned_uploads_and_orphans() {
    let t = start().await;
    let content = data(1000, 9);
    let (_, resp) = create(&t, &request("Inbox/old.bin", &content)).await;
    let id = open_id(&resp);
    let fresh_content = data(1000, 10);
    let (_, resp) = create(&t, &request("Inbox/fresh.bin", &fresh_content)).await;
    let fresh = open_id(&resp);
    fs::write(t.root.join(".bring/staging/orphan.bring.tmp"), b"junk").unwrap();

    t.ring
        .db
        .lock()
        .execute("UPDATE uploads SET updated_at = 0 WHERE id = ?1", [&id])
        .unwrap();
    let now = devices::now_secs();
    let removed = uploads::cleanup(&t.ring.uploads, now, uploads::ABANDON_AFTER).unwrap();
    assert_eq!(removed, 3, "stale row + its staging file + orphan");

    let mut left: Vec<String> = fs::read_dir(t.root.join(".bring/staging"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, [format!("{fresh}.bring.tmp")]);
}
