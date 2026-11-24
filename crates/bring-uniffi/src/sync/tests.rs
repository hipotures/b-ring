use std::fs;

use bring_core::db::{Role, open_in_memory};
use bring_core::hash::hash_bytes;

use super::*;

fn db() -> Connection {
    open_in_memory(Role::Moon).unwrap()
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
    store::entry_by_ring_path(c, path).unwrap()
}

/// A `synced` row as left behind by a verified upload.
fn synced(c: &Connection, path: &str, local: &str, content: &[u8], file_id: i64) {
    c.execute(
        "INSERT INTO entries (ring_path, local_path, remote_file_id, size, mtime_ns, blake3, state)
         VALUES (?1, ?2, ?3, ?4, 1, ?5, 'synced')",
        params![
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
    apply_index_entry(&c, &remote(1, "DCIM/a.jpg", b"one", false)).unwrap();
    let r = get(&c, "DCIM/a.jpg").unwrap();
    assert_eq!(r.state, FileState::RemoteOnly);
    assert_eq!((r.remote_file_id, r.local_path), (Some(1), None));

    apply_index_entry(&c, &remote(1, "DCIM/a.jpg", b"two!", false)).unwrap();
    assert_eq!(get(&c, "DCIM/a.jpg").unwrap().size, 4);

    apply_index_entry(&c, &remote(1, "DCIM/a.jpg", b"", true)).unwrap();
    assert!(get(&c, "DCIM/a.jpg").is_none());
    // Tombstones for unknown paths are ignored.
    apply_index_entry(&c, &remote(9, "DCIM/zzz.jpg", b"", true)).unwrap();
}

#[test]
fn ring_side_delete_never_touches_local_copy() {
    let c = db();
    synced(&c, "DCIM/p/a.jpg", "/sd/DCIM/a.jpg", b"photo", 5);
    apply_index_entry(&c, &remote(5, "DCIM/p/a.jpg", b"", true)).unwrap();
    let r = get(&c, "DCIM/p/a.jpg").unwrap();
    assert_eq!(r.state, FileState::LocalOnly);
    assert!(r.suppressed, "never re-uploaded (§5.1)");
    assert_eq!(r.local_path.as_deref(), Some("/sd/DCIM/a.jpg"));
}

#[test]
fn synced_entry_follows_metadata_but_not_foreign_content() {
    let c = db();
    synced(&c, "DCIM/p/a.jpg", "/sd/a.jpg", b"photo", 5);
    apply_index_entry(&c, &remote(5, "DCIM/p/a.jpg", b"photo", false)).unwrap();
    let r = get(&c, "DCIM/p/a.jpg").unwrap();
    assert_eq!(r.state, FileState::Synced);
    assert_eq!(r.thumb, Some(hash_bytes(b"thumb")));

    // The desktop replaced the file: our version is no longer on the Ring.
    apply_index_entry(&c, &remote(5, "DCIM/p/a.jpg", b"edited", false)).unwrap();
    let r = get(&c, "DCIM/p/a.jpg").unwrap();
    assert_eq!((r.state, r.suppressed), (FileState::LocalOnly, true));
}

#[test]
fn only_files_new_to_this_device_count_as_received() {
    let c = db();
    assert!(apply_index_entry(&c, &remote(1, "DCIM/a.jpg", b"one", false)).unwrap());
    // The same file again (e.g. its thumbnail landed): not news.
    assert!(!apply_index_entry(&c, &remote(1, "DCIM/a.jpg", b"one", false)).unwrap());
    // Replaced on the Ring: news.
    assert!(apply_index_entry(&c, &remote(1, "DCIM/a.jpg", b"two!", false)).unwrap());
    // Our own upload echoed back with its thumbnail: not news.
    synced(&c, "DCIM/p/b.jpg", "/sd/b.jpg", b"photo", 5);
    assert!(!apply_index_entry(&c, &remote(5, "DCIM/p/b.jpg", b"photo", false)).unwrap());
    // Deletions are not news either.
    assert!(!apply_index_entry(&c, &remote(1, "DCIM/a.jpg", b"", true)).unwrap());
}

#[test]
fn pending_upload_keeps_its_path() {
    let c = db();
    store::insert_local(&c, "Inbox/a.pdf", "/sd/a.pdf", 3, 1, None).unwrap();
    apply_index_entry(&c, &remote(8, "Inbox/a.pdf", b"other", false)).unwrap();
    let r = get(&c, "Inbox/a.pdf").unwrap();
    assert_eq!(r.state, FileState::LocalOnly);
    assert_eq!(r.remote_file_id, None);
}

fn folder(dir: &Path, ring: &str) -> WatchedFolder {
    WatchedFolder {
        id: 1,
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

    let r = apply_scan(&mut c, &f).unwrap();
    assert_eq!((r.new_files, r.changed, r.gone), (2, 0, 0));
    let a = get(&c, "DCIM/pixel/Camera/a.jpg").unwrap();
    assert_eq!((a.state, a.blake3), (FileState::LocalOnly, None));
    assert!(get(&c, "DCIM/pixel/.pending-123.jpg").is_none());

    // Unchanged: nothing to do.
    assert_eq!(apply_scan(&mut c, &f).unwrap(), ScanReport::default());

    // b.jpg became synced; then it is edited → local-only again (§5.3).
    let b_local = dcim.join("Camera/b.jpg");
    c.execute(
        "UPDATE entries SET state = 'synced', remote_file_id = 4, blake3 = ?1
         WHERE ring_path = 'DCIM/pixel/Camera/b.jpg'",
        [hash_bytes(b"bbb")],
    )
    .unwrap();
    fs::write(&b_local, b"bbb, edited").unwrap();
    let r = apply_scan(&mut c, &f).unwrap();
    assert_eq!(r.changed, 1);
    let b = get(&c, "DCIM/pixel/Camera/b.jpg").unwrap();
    assert_eq!(
        (b.state, b.blake3, b.size),
        (FileState::LocalOnly, None, 11)
    );

    // Deleting: a synced file becomes remote-only, a local-only one vanishes.
    c.execute(
        "UPDATE entries SET state = 'synced', remote_file_id = 4
         WHERE ring_path = 'DCIM/pixel/Camera/b.jpg'",
        [],
    )
    .unwrap();
    fs::remove_file(&b_local).unwrap();
    fs::remove_file(dcim.join("Camera/a.jpg")).unwrap();
    let r = apply_scan(&mut c, &f).unwrap();
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
    apply_scan(&mut c, &f).unwrap();
    fs::rename(tmp.path().join("sdcard"), tmp.path().join("ejected")).unwrap();
    let r = apply_scan(&mut c, &f).unwrap();
    assert_eq!((r.gone, r.unavailable_folders), (0, 1));
    assert!(get(&c, "DCIM/pixel/a.jpg").is_some());
}

#[test]
fn reappearing_file_takes_over_remote_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let dcim = tmp.path().join("DCIM");
    fs::create_dir_all(&dcim).unwrap();
    let mut c = db();
    apply_index_entry(&c, &remote(3, "DCIM/pixel/v.mp4", b"video", false)).unwrap();
    fs::write(dcim.join("v.mp4"), b"video").unwrap();
    let r = apply_scan(&mut c, &[folder(&dcim, "DCIM/pixel")]).unwrap();
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
    let row = |c: &Connection, ring: &str, local: &str, st: LocalStat, state: &str| {
        c.execute(
            "INSERT INTO entries (ring_path, local_path, remote_file_id, size, mtime_ns, blake3, state)
             VALUES (?1, ?2, 9, ?3, ?4, ?5, ?6)",
            params![ring, local, st.size, st.mtime_ns, hash_bytes(b"x"), state],
        )
        .unwrap();
        c.last_insert_rowid()
    };
    let (a, sa) = file("a.mp4", b"aaaa");
    let (b, sb) = file("b.mp4", b"bbbb");
    let (l, sl) = file("l.mp4", b"llll");
    let (p, sp) = file("p.mp4", b"pppp");
    let ia = row(&c, "V/a.mp4", &a, sa, "synced");
    // Edited after it was verified: kept.
    let ib = row(
        &c,
        "V/b.mp4",
        &b,
        LocalStat {
            size: sb.size,
            mtime_ns: sb.mtime_ns - 1,
        },
        "synced",
    );
    // Only on the phone: never deleted.
    let il = row(&c, "V/l.mp4", &l, sl, "local-only");
    // Synced but waiting in the outbox: kept.
    let ip = row(&c, "V/p.mp4", &p, sp, "synced");
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
