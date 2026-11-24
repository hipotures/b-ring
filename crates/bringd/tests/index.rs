//! Step 3 gate: the watcher keeps the index in sync with the disk, with
//! correct `seq` ordering and tombstones, exposed through `GET /index`.

mod common;

use std::fs;

use axum::http::StatusCode;
use bring_core::api::{HealthResponse, IndexResponse};
use bring_core::hash::hash_bytes;
use bring_core::{Blake3Hash, RelPath};
use bringd::store::{self, FileRow};
use common::{get_json, start, wait_for};
use rusqlite::Connection;

fn row(c: &Connection, path: &str) -> Option<FileRow> {
    store::file_by_path(c, &RelPath::new(path).unwrap()).unwrap()
}

fn hashed_as(c: &Connection, path: &str, h: Blake3Hash) -> bool {
    row(c, path).is_some_and(|r| !r.deleted && r.blake3 == Some(h))
}

fn is_tombstone(c: &Connection, path: &str) -> bool {
    row(c, path).is_some_and(|r| r.deleted)
}

#[tokio::test]
async fn watcher_tracks_create_modify_rename_delete() {
    let t = start().await;
    let ring = &t.ring;

    // Create.
    fs::write(t.root.join("DCIM/a.jpg"), b"one").unwrap();
    let h1 = hash_bytes(b"one");
    wait_for(ring, "a.jpg hashed", |c| hashed_as(c, "DCIM/a.jpg", h1)).await;
    let created = row(&ring.db.lock(), "DCIM/a.jpg").unwrap();
    assert_eq!(created.size, 3);
    assert_eq!(created.mime.as_deref(), Some("image/jpeg"));
    assert_eq!(created.origin_device_id, None);

    // Modify.
    fs::write(t.root.join("DCIM/a.jpg"), b"two!").unwrap();
    let h2 = hash_bytes(b"two!");
    wait_for(ring, "a.jpg rehashed", |c| hashed_as(c, "DCIM/a.jpg", h2)).await;
    let modified = row(&ring.db.lock(), "DCIM/a.jpg").unwrap();
    assert!(modified.seq > created.seq);
    assert_eq!(modified.id, created.id);

    // Rename.
    fs::rename(t.root.join("DCIM/a.jpg"), t.root.join("Inbox/b.jpg")).unwrap();
    wait_for(ring, "rename", |c| {
        is_tombstone(c, "DCIM/a.jpg") && hashed_as(c, "Inbox/b.jpg", h2)
    })
    .await;

    // Delete.
    fs::remove_file(t.root.join("Inbox/b.jpg")).unwrap();
    wait_for(ring, "delete", |c| is_tombstone(c, "Inbox/b.jpg")).await;

    // Directory created with content, then moved, then removed.
    fs::create_dir_all(t.root.join("Download/album/sub")).unwrap();
    fs::write(t.root.join("Download/album/sub/x.txt"), b"x").unwrap();
    wait_for(ring, "nested file", |c| {
        hashed_as(c, "Download/album/sub/x.txt", hash_bytes(b"x"))
    })
    .await;
    fs::rename(
        t.root.join("Download/album"),
        t.root.join("Download/renamed"),
    )
    .unwrap();
    wait_for(ring, "dir rename", |c| {
        is_tombstone(c, "Download/album/sub/x.txt")
            && hashed_as(c, "Download/renamed/sub/x.txt", hash_bytes(b"x"))
    })
    .await;
    fs::remove_dir_all(t.root.join("Download/renamed")).unwrap();
    wait_for(ring, "dir delete", |c| {
        is_tombstone(c, "Download/renamed/sub/x.txt")
    })
    .await;

    // Staging area is never indexed.
    fs::write(t.root.join(".bring/staging/u.bring.tmp"), b"partial").unwrap();

    // The delta stream: strictly increasing seq, one entry per row (latest
    // state), tombstones included, and paging that resumes where it stopped.
    let (status, all): (_, IndexResponse) = t.get_json("/api/v1/index").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!all.has_more);
    assert!(all.entries.windows(2).all(|w| w[0].seq < w[1].seq));
    let by_path = |p: &str| all.entries.iter().find(|e| e.path.as_str() == p).unwrap();
    assert!(by_path("DCIM/a.jpg").deleted);
    assert!(by_path("Inbox/b.jpg").deleted);
    assert!(by_path("Download/renamed/sub/x.txt").deleted);
    assert!(
        !all.entries
            .iter()
            .any(|e| e.path.as_str().starts_with(".bring"))
    );
    assert_eq!(all.next_since, all.entries.last().unwrap().seq);

    let (_, first): (_, IndexResponse) = t.get_json("/api/v1/index?limit=1").await;
    assert_eq!(first.entries.len(), 1);
    assert!(first.has_more);
    let (_, rest): (_, IndexResponse) = t
        .get_json(&format!("/api/v1/index?since={}", first.next_since))
        .await;
    assert_eq!(rest.entries.len(), all.entries.len() - 1);

    let (_, empty): (_, IndexResponse) = t
        .get_json(&format!("/api/v1/index?since={}", all.next_since))
        .await;
    assert!(empty.entries.is_empty());
    assert_eq!(empty.next_since, all.next_since);
}

#[tokio::test]
async fn startup_scan_reconciles_offline_changes() {
    let dir = tempfile::tempdir().unwrap();
    let config = common::test_config(dir.path());
    fs::create_dir_all(config.ring_root.join("DCIM")).unwrap();
    fs::write(config.ring_root.join("DCIM/keep.jpg"), b"k").unwrap();
    fs::write(config.ring_root.join("DCIM/gone.jpg"), b"g").unwrap();
    {
        let t = common::start_with(tempfile::tempdir().unwrap(), |c| *c = config.clone()).await;
        wait_for(&t.ring, "initial hashes", |c| {
            hashed_as(c, "DCIM/keep.jpg", hash_bytes(b"k"))
                && hashed_as(c, "DCIM/gone.jpg", hash_bytes(b"g"))
        })
        .await;
    }
    // While bringd is "down":
    fs::remove_file(config.ring_root.join("DCIM/gone.jpg")).unwrap();
    fs::write(config.ring_root.join("DCIM/new.jpg"), b"n").unwrap();

    let t = common::start_with(tempfile::tempdir().unwrap(), |c| *c = config.clone()).await;
    wait_for(&t.ring, "offline changes", |c| {
        is_tombstone(c, "DCIM/gone.jpg")
            && hashed_as(c, "DCIM/new.jpg", hash_bytes(b"n"))
            && hashed_as(c, "DCIM/keep.jpg", hash_bytes(b"k"))
    })
    .await;
}

#[tokio::test]
async fn health_reports_ring_id() {
    let t = start().await;
    let (status, h): (_, HealthResponse) = get_json(&t.ring.router(), "/api/v1/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.ring_id, t.ring.ring_id);
    assert_eq!(h.ring_id.len(), 32);
    assert_eq!(h.protocol, 1);
}

#[tokio::test]
async fn index_rejects_negative_since() {
    let t = start().await;
    let (status, body): (_, bring_core::api::ErrorBody) =
        t.get_json("/api/v1/index?since=-1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body.error, bring_core::api::ErrorCode::BadRequest);
}
