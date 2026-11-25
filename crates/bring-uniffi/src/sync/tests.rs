use std::fs;

use bring_core::db::{Role, open_in_memory};
use bring_core::hash::hash_bytes;

use super::*;

const RING_1: i64 = 1;
const RING_2: i64 = 2;

fn db() -> Connection {
    let c = open_in_memory(Role::Moon).unwrap();
    c.execute(
        "INSERT INTO rings (id, ring_id, name, lan_url, tls_cert_sha256, device_uuid, device_slug)
         VALUES (1, x'0102030405060708090a0b0c0d0e0f10', 'Primary', 'https://10.0.0.1:7443',
                 x'0000000000000000000000000000000000000000000000000000000000000000',
                 x'11111111111111111111111111111111', 'phone')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO rings (id, ring_id, name, lan_url, tls_cert_sha256, device_uuid, device_slug)
         VALUES (2, x'2102030405060708090a0b0c0d0e0f10', 'Secondary', 'https://10.0.0.2:7443',
                 x'1111111111111111111111111111111111111111111111111111111111111111',
                 x'22222222222222222222222222222222', 'phone')",
        [],
    )
    .unwrap();
    c
}

fn remote(id: i64, path: &str, content: &[u8], deleted: bool) -> IndexEntry {
    IndexEntry {
        id,
        path: RelPath::new(path).unwrap(),
        folder: path.split('/').next().unwrap().into(),
        size: content.len() as u64,
        mtime_ns: 7,
        mime: Some("image/jpeg".into()),
        blake3: Some(hash_bytes(content)),
        origin: None,
        thumb: Some(hash_bytes(b"thumb")),
        deleted,
        seq: id,
    }
}

fn get(c: &Connection, path: &str) -> Option<EntryRow> {
    store::entry_by_ring_path(c, RING_1, path).unwrap()
}

/// A `synced` row as left behind by a verified upload.
fn synced(c: &Connection, path: &str, local: &str, content: &[u8], file_id: i64) {
    c.execute(
        "INSERT INTO entries (ring_id, ring_path, local_path, remote_file_id, size, mtime_ns, blake3, state)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 'synced')",
        params![
            RING_1,
            path,
            local,
            file_id,
            content.len() as u64,
            hash_bytes(content)
        ],
    )
    .unwrap();
}

#[test]
fn remote_files_appear_update_and_disappear() {
    let c = db();
    apply_index_entry(&c, RING_1, &remote(1, "DCIM/a.jpg", b"one", false)).unwrap();
    let r = get(&c, "DCIM/a.jpg").unwrap();
    assert_eq!(r.state, FileState::RemoteOnly);
    assert_eq!((r.remote_file_id, r.local_path), (Some(1), None));

    apply_index_entry(&c, RING_1, &remote(1, "DCIM/a.jpg", b"two!", false)).unwrap();
    assert_eq!(get(&c, "DCIM/a.jpg").unwrap().size, 4);

    apply_index_entry(&c, RING_1, &remote(1, "DCIM/a.jpg", b"", true)).unwrap();
    assert!(get(&c, "DCIM/a.jpg").is_none());
    // Tombstones for unknown paths are ignored.
    apply_index_entry(&c, RING_1, &remote(9, "DCIM/zzz.jpg", b"", true)).unwrap();
}

#[test]
fn ring_side_delete_never_touches_local_copy() {
    let c = db();
    synced(&c, "DCIM/p/a.jpg", "/sd/DCIM/a.jpg", b"photo", 5);
    apply_index_entry(&c, RING_1, &remote(5, "DCIM/p/a.jpg", b"", true)).unwrap();
    let r = get(&c, "DCIM/p/a.jpg").unwrap();
    assert_eq!(r.state, FileState::LocalOnly);
    assert!(r.suppressed, "never re-uploaded (§5.1)");
    assert_eq!(r.local_path.as_deref(), Some("/sd/DCIM/a.jpg"));
}

#[test]
fn synced_entry_follows_metadata_but_not_foreign_content() {
    let c = db();
    synced(&c, "DCIM/p/a.jpg", "/sd/a.jpg", b"photo", 5);
    apply_index_entry(&c, RING_1, &remote(5, "DCIM/p/a.jpg", b"photo", false)).unwrap();
    let r = get(&c, "DCIM/p/a.jpg").unwrap();
    assert_eq!(r.state, FileState::Synced);
    assert_eq!(r.thumb, Some(hash_bytes(b"thumb")));

    // The desktop replaced the file: our version is no longer on the Ring.
    apply_index_entry(&c, RING_1, &remote(5, "DCIM/p/a.jpg", b"edited", false)).unwrap();
    let r = get(&c, "DCIM/p/a.jpg").unwrap();
    assert_eq!((r.state, r.suppressed), (FileState::LocalOnly, true));
}

#[test]
fn only_files_new_to_this_device_count_as_received() {
    let c = db();
    assert!(apply_index_entry(&c, RING_1, &remote(1, "DCIM/a.jpg", b"one", false)).unwrap());
    // The same file again (e.g. its thumbnail landed): not news.
    assert!(!apply_index_entry(&c, RING_1, &remote(1, "DCIM/a.jpg", b"one", false)).unwrap());
    // Replaced on the Ring: news.
    assert!(apply_index_entry(&c, RING_1, &remote(1, "DCIM/a.jpg", b"two!", false)).unwrap());
    // Our own upload echoed back with its thumbnail: not news.
    synced(&c, "DCIM/p/b.jpg", "/sd/b.jpg", b"photo", 5);
    assert!(!apply_index_entry(&c, RING_1, &remote(5, "DCIM/p/b.jpg", b"photo", false)).unwrap());
    // Deletions are not news either.
    assert!(!apply_index_entry(&c, RING_1, &remote(1, "DCIM/a.jpg", b"", true)).unwrap());
}

#[test]
fn pending_upload_keeps_its_path() {
    let c = db();
    store::insert_local(&c, RING_1, "Inbox/a.pdf", "/sd/a.pdf", 3, 1, None).unwrap();
    apply_index_entry(&c, RING_1, &remote(8, "Inbox/a.pdf", b"other", false)).unwrap();
    let r = get(&c, "Inbox/a.pdf").unwrap();
    assert_eq!(r.state, FileState::LocalOnly);
    assert_eq!(r.remote_file_id, None);
}

fn folder(dir: &Path, ring: &str) -> WatchedFolder {
    WatchedFolder {
        id: 1,
        ring_id: "0102030405060708090a0b0c0d0e0f10".into(),
        local_dir: dir.to_str().unwrap().into(),
        ring_folder: ring.into(),
        enabled: true,
    }
}

#[test]
fn scan_finds_new_changed_and_deleted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dcim = tmp.path().join("DCIM");
    fs::create_dir_all(dcim.join("Camera")).unwrap();
    fs::write(dcim.join("Camera/a.jpg"), b"aaa").unwrap();
    fs::write(dcim.join("Camera/b.jpg"), b"bbb").unwrap();
    fs::write(dcim.join(".pending-123.jpg"), b"partial").unwrap();
    let f = [folder(&dcim, "DCIM/pixel")];
    let mut c = db();

    let r = apply_scan(&mut c, RING_1, &f).unwrap();
    assert_eq!((r.new_files, r.changed, r.gone), (2, 0, 0));
    let a = get(&c, "DCIM/pixel/Camera/a.jpg").unwrap();
    assert_eq!((a.state, a.blake3), (FileState::LocalOnly, None));
    assert!(get(&c, "DCIM/pixel/.pending-123.jpg").is_none());

    // Unchanged: nothing to do.
    assert_eq!(apply_scan(&mut c, RING_1, &f).unwrap(), ScanReport::default());

    // b.jpg became synced; then it is edited → local-only again (§5.3).
    let b_local = dcim.join("Camera/b.jpg");
    c.execute(
        "UPDATE entries SET state = 'synced', remote_file_id = 4, blake3 = ?1
         WHERE ring_id = ?2 AND ring_path = 'DCIM/pixel/Camera/b.jpg'",
        params![hash_bytes(b"bbb"), RING_1],
    )
    .unwrap();
    fs::write(&b_local, b"bbb, edited").unwrap();
    let r = apply_scan(&mut c, RING_1, &f).unwrap();
    assert_eq!(r.changed, 1);
    let b = get(&c, "DCIM/pixel/Camera/b.jpg").unwrap();
    assert_eq!(
        (b.state, b.blake3, b.size),
        (FileState::LocalOnly, None, 11)
    );

    // Deleting: a synced file becomes remote-only, a local-only one vanishes.
    c.execute(
        "UPDATE entries SET state = 'synced', remote_file_id = 4
         WHERE ring_id = ?1 AND ring_path = 'DCIM/pixel/Camera/b.jpg'",
        [RING_1],
    )
    .unwrap();
    fs::remove_file(&b_local).unwrap();
    fs::remove_file(dcim.join("Camera/a.jpg")).unwrap();
    let r = apply_scan(&mut c, RING_1, &f).unwrap();
    assert_eq!(r.gone, 2);
    let b = get(&c, "DCIM/pixel/Camera/b.jpg").unwrap();
    assert_eq!((b.state, b.local_path), (FileState::RemoteOnly, None));
    assert!(get(&c, "DCIM/pixel/Camera/a.jpg").is_none());
}

#[test]
fn unmounted_folder_is_not_a_mass_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let sd = tmp.path().join("sdcard/DCIM");
    fs::create_dir_all(&sd).unwrap();
    fs::write(sd.join("a.jpg"), b"aaa").unwrap();
    let f = [folder(&sd, "DCIM/pixel")];
    let mut c = db();
    apply_scan(&mut c, RING_1, &f).unwrap();
    fs::rename(tmp.path().join("sdcard"), tmp.path().join("ejected")).unwrap();
    let r = apply_scan(&mut c, RING_1, &f).unwrap();
    assert_eq!((r.gone, r.unavailable_folders), (0, 1));
    assert!(get(&c, "DCIM/pixel/a.jpg").is_some());
}

#[test]
fn reappearing_file_takes_over_remote_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let dcim = tmp.path().join("DCIM");
    fs::create_dir_all(&dcim).unwrap();
    let mut c = db();
    apply_index_entry(&c, RING_1, &remote(3, "DCIM/pixel/v.mp4", b"video", false)).unwrap();
    fs::write(dcim.join("v.mp4"), b"video").unwrap();
    let r = apply_scan(&mut c, RING_1, &[folder(&dcim, "DCIM/pixel")]).unwrap();
    assert_eq!(r.new_files, 1);
    let e = get(&c, "DCIM/pixel/v.mp4").unwrap();
    assert_eq!(e.state, FileState::LocalOnly);
    assert!(e.local_path.is_some());
}

#[test]
fn free_local_deletes_only_verified_synced_copies() {
    let tmp = tempfile::tempdir().unwrap();
    let file = |name: &str, body: &[u8]| {
        let p = tmp.path().join(name);
        fs::write(&p, body).unwrap();
        let st = local_stat(&p).unwrap().unwrap();
        (p.to_str().unwrap().to_owned(), st)
    };
    let mut c = db();
    let row = |c: &Connection, ring_id: i64, ring: &str, local: &str, st: LocalStat, state: &str| {
        c.execute(
            "INSERT INTO entries (ring_id, ring_path, local_path, remote_file_id, size, mtime_ns, blake3, state)
             VALUES (?1, ?2, ?3, 9, ?4, ?5, ?6, ?7)",
            params![ring_id, ring, local, st.size, st.mtime_ns, hash_bytes(b"x"), state],
        )
        .unwrap();
        c.last_insert_rowid()
    };
    let (a, sa) = file("a.mp4", b"aaaa");
    let (b, sb) = file("b.mp4", b"bbbb");
    let (l, sl) = file("l.mp4", b"llll");
    let (p, sp) = file("p.mp4", b"pppp");
    let ia = row(&c, RING_1, "V/a.mp4", &a, sa, "synced");
    // Edited after it was verified: kept.
    let ib = row(
        &c,
        RING_1,
        "V/b.mp4",
        &b,
        LocalStat {
            size: sb.size,
            mtime_ns: sb.mtime_ns - 1,
        },
        "synced",
    );
    // Only on the phone: never deleted.
    let il = row(&c, RING_1, "V/l.mp4", &l, sl, "local-only");
    // Synced but waiting in the outbox: kept.
    let ip = row(&c, RING_1, "V/p.mp4", &p, sp, "synced");
    c.execute(
        "INSERT INTO outbox (entry_id, hashed_size, hashed_mtime_ns) VALUES (?1, 4, 0)",
        [ip],
    )
    .unwrap();

    let r = free_local(&mut c, &[ia, ib, il, ip, 999]).unwrap();
    assert_eq!(
        r,
        FreeReport {
            freed: 1,
            freed_bytes: 4,
            skipped: 4
        }
    );
    assert!(!Path::new(&a).exists());
    let e = get(&c, "V/a.mp4").unwrap();
    assert_eq!(
        (e.state, e.local_path, e.remote_file_id),
        (FileState::RemoteOnly, None, Some(9))
    );
    for kept in [&b, &l, &p] {
        assert!(Path::new(kept).exists(), "{kept} must stay");
    }
    assert_eq!(get(&c, "V/b.mp4").unwrap().state, FileState::Synced);
}

#[test]
fn shared_folder_keeps_local_file_until_all_rings_verify_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dcim = tmp.path().join("DCIM");
    fs::create_dir_all(&dcim).unwrap();
    let video = dcim.join("big.mp4");
    fs::write(&video, b"video-bytes").unwrap();
    let st = local_stat(&video).unwrap().unwrap();
    let v_str = video.to_str().unwrap();

    let mut c = db();
    // Both Ring 1 and Ring 2 watch the exact same local directory.
    c.execute(
        "INSERT INTO watched_folders (ring_id, local_dir, ring_folder, enabled)
         VALUES (?1, ?2, 'DCIM/phone', 1), (?3, ?2, 'DCIM/phone', 1)",
        params![RING_1, dcim.to_str().unwrap(), RING_2],
    )
    .unwrap();

    // Ring 1 synced it; Ring 2 hasn't even scanned it yet.
    c.execute(
        "INSERT INTO entries (ring_id, ring_path, local_path, remote_file_id, size, mtime_ns, blake3, state)
         VALUES (?1, 'DCIM/phone/big.mp4', ?2, 10, ?3, ?4, ?5, 'synced')",
        params![RING_1, v_str, st.size, st.mtime_ns, hash_bytes(b"video-bytes")],
    )
    .unwrap();
    let id1 = c.last_insert_rowid();

    assert!(other_ring_needs_local_file(&c, RING_1, v_str, st).unwrap());
    let r1 = free_local(&mut c, &[id1]).unwrap();
    assert_eq!((r1.freed, r1.skipped), (0, 1));
    assert!(video.exists(), "must not delete before Ring 2 uploads it");

    // Now Ring 2 also has it verified as synced.
    c.execute(
        "INSERT INTO entries (ring_id, ring_path, local_path, remote_file_id, size, mtime_ns, blake3, state)
         VALUES (?1, 'DCIM/phone/big.mp4', ?2, 20, ?3, ?4, ?5, 'synced')",
        params![RING_2, v_str, st.size, st.mtime_ns, hash_bytes(b"video-bytes")],
    )
    .unwrap();

    assert!(!other_ring_needs_local_file(&c, RING_1, v_str, st).unwrap());
    let r2 = free_local(&mut c, &[id1]).unwrap();
    assert_eq!((r2.freed, r2.skipped), (1, 0));
    assert!(!video.exists());
    // Both Rings now reflect remote-only.
    assert_eq!(
        store::entry_by_ring_path(&c, RING_1, "DCIM/phone/big.mp4")
            .unwrap()
            .unwrap()
            .state,
        FileState::RemoteOnly
    );
    assert_eq!(
        store::entry_by_ring_path(&c, RING_2, "DCIM/phone/big.mp4")
            .unwrap()
            .unwrap()
            .state,
        FileState::RemoteOnly
    );
}

#[test]
fn unwatched_local_and_held_entries_are_pruned_without_deleting_files() {
    let tmp = tempfile::tempdir().unwrap();
    let watched = tmp.path().join("Watched");
    let unwatched = tmp.path().join("OldFolder");
    let share = tmp.path().join("Share");
    fs::create_dir_all(&watched).unwrap();
    fs::create_dir_all(&unwatched).unwrap();
    fs::create_dir_all(&share).unwrap();

    let kept_held = watched.join("keep.jpg");
    let orphan_held = unwatched.join("old.jpg");
    let still_synced = unwatched.join("still_on_ring.jpg");
    let pending_share = share.join("shared.txt");
    for p in [&kept_held, &orphan_held, &still_synced, &pending_share] {
        fs::write(p, b"data").unwrap();
    }

    let mut c = db();
    // 1. Held entry inside a still-watched folder: MUST stay (never re-uploaded).
    synced(&c, "Watched/phone/keep.jpg", kept_held.to_str().unwrap(), b"data", 1);
    apply_index_entry(&c, RING_1, &remote(1, "Watched/phone/keep.jpg", b"", true)).unwrap();

    // 2. Held entry from a removed watched folder whose server copy was deleted: MUST be pruned.
    synced(&c, "OldFolder/phone/old.jpg", orphan_held.to_str().unwrap(), b"data", 2);
    apply_index_entry(&c, RING_1, &remote(2, "OldFolder/phone/old.jpg", b"", true)).unwrap();

    // 3. Synced entry from a removed watched folder whose server copy still exists: MUST stay.
    synced(
        &c,
        "OldFolder/phone/still_on_ring.jpg",
        still_synced.to_str().unwrap(),
        b"data",
        3,
    );

    // 4. Explicit Share upload outside watched folders (queued in outbox): MUST stay.
    let share_id = store::insert_local(
        &c,
        RING_1,
        "Inbox/shared.txt",
        pending_share.to_str().unwrap(),
        4,
        1,
        None,
    )
    .unwrap();
    c.execute(
        "INSERT INTO outbox (entry_id, hashed_size, hashed_mtime_ns) VALUES (?1, 4, 1)",
        [share_id],
    )
    .unwrap();

    let r = apply_scan(&mut c, RING_1, &[folder(&watched, "Watched/phone")]).unwrap();
    assert_eq!(r.gone, 1);

    assert!(get(&c, "Watched/phone/keep.jpg").is_some(), "watched Held stays");
    assert!(get(&c, "OldFolder/phone/old.jpg").is_none(), "unwatched Held pruned");
    assert!(get(&c, "OldFolder/phone/still_on_ring.jpg").is_some(), "still synced stays");
    assert!(get(&c, "Inbox/shared.txt").is_some(), "queued share stays");
    // Auto-prune never deletes files from phone storage.
    assert!(orphan_held.exists());
}

#[test]
fn stale_held_entry_merges_with_renamed_remote_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("0test");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("photo.jpg");
    fs::write(&file, b"same-bytes").unwrap();
    let st = local_stat(&file).unwrap().unwrap();
    let local = file.to_str().unwrap();

    let mut c = db();
    c.execute(
        "INSERT INTO entries (ring_id, ring_path, local_path, remote_file_id, size, mtime_ns, blake3, state, suppressed)
         VALUES (?1, 'sm-s918b/0test/photo.jpg', ?2, NULL, ?3, ?4, ?5, 'local-only', 1)",
        params![RING_1, local, st.size, st.mtime_ns, hash_bytes(b"same-bytes")],
    )
    .unwrap();
    apply_index_entry(&c, RING_1, &remote(20, "s23u/0test/photo.jpg", b"same-bytes", false))
        .unwrap();

    let r = apply_scan(&mut c, RING_1, &[folder(&dir, "s23u/0test")]).unwrap();
    assert_eq!(r.gone, 1);
    assert!(get(&c, "sm-s918b/0test/photo.jpg").is_none());
    let merged = get(&c, "s23u/0test/photo.jpg").unwrap();
    assert_eq!(merged.state, FileState::Synced);
    assert_eq!(merged.local_path.as_deref(), Some(local));
    assert!(!merged.suppressed);
}

#[test]
fn looks_thumbable_matches_images_and_videos_only() {
    assert!(looks_thumbable(Some("video/mp4"), "a/b.mp4"));
    assert!(looks_thumbable(Some("image/jpeg"), "a/b.jpg"));
    assert!(looks_thumbable(None, "a/BVR 2026-11-20.MP4"));
    assert!(!looks_thumbable(Some("image/svg+xml"), "a/icon.svg"));
    assert!(!looks_thumbable(Some("video/vnd.dlna.mpeg-tts"), "a/prompts.ts"));
    assert!(!looks_thumbable(Some("application/pdf"), "a/doc.pdf"));
    assert!(!looks_thumbable(None, "a/notes.txt"));
}
