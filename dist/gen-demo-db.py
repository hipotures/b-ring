#!/usr/bin/env python3
"""Generate a large fake Moon database (demo.db) for load-testing the Android app.

The schema is NOT copied here: the script applies the real migrations
(crates/bring-core/src/db/moon/*.sql, in the order listed in
crates/bring-core/src/db/mod.rs) and sets PRAGMA user_version like the app does.
When the schema changes, only the INSERT part below may need an update.

Usage:
  dist/gen-demo-db.py                         # 1,000,000 entries -> ./demo.db
  dist/gen-demo-db.py -n 10000000 -o /tmp/demo10m.db
  dist/gen-demo-db.py -n 100000 --seed 7

The first Ring ("home-nas") gets -n entries; the other two get n/20 and n/100.
Ring counters (transferred_files/bytes) are derived from the generated entries,
so the Sync activity header matches the data. Nothing here touches your real
moon.db. Push the result to the phone with dist/push-demo-db.sh.
"""
import argparse
import os
import random
import re
import sqlite3
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DB_DIR = ROOT / "crates/bring-core/src/db"

# (mime, extension, name prefix, min size, max size, weight)
KINDS = [
    ("image/jpeg", "jpg", "IMG", 800_000, 9_000_000, 40),
    ("image/heic", "heic", "IMG", 600_000, 5_000_000, 8),
    ("image/png", "png", "Screenshot", 80_000, 3_000_000, 10),
    ("video/mp4", "mp4", "VID", 15_000_000, 900_000_000, 8),
    ("application/pdf", "pdf", "doc", 40_000, 6_000_000, 14),
    ("audio/flac", "flac", "track", 20_000_000, 60_000_000, 4),
    ("audio/mp4", "m4a", "voice-note", 300_000, 8_000_000, 6),
    ("text/plain", "txt", "note", 500, 60_000, 10),
]
TOPS = [
    "pixel-9/Camera", "pixel-9/Camera", "pixel-9/Camera", "pixel-9/Screenshots",
    "pixel-9/Documents", "pixel-9/Recordings", "tablet/Books", "tablet/Sketches",
    "shared/Trips", "shared/Manuals", "music/ambient-sessions", "projects/3d-prints",
    "backups",
]
RINGS = [  # (name, lan_url, wan_url, offload threshold or None, share of -n as divisor)
    ("home-nas", "https://192.168.1.50:7443", "https://nas.example.com", 100 << 20, 1),
    ("workstation", "https://192.168.1.24:7443", None, 500 << 20, 20),
    ("archive-vps", "https://10.8.0.10:7443", "https://archive.example.org", -1, 100),
]
BATCH = 50_000
NS_PER_DAY = 86_400 * 1_000_000_000


def moon_migrations():
    """Migration files in the order the Rust code applies them."""
    src = (DB_DIR / "mod.rs").read_text()
    block = re.search(r"MOON_MIGRATIONS[^=]*=\s*&\[(.*?)\];", src, re.S)
    if not block:
        sys.exit("cannot find the Moon migration list in crates/bring-core/src/db/mod.rs")
    files = re.findall(r'include_str!\("([^"]+)"\)', block.group(1))
    return [DB_DIR / f for f in files]


def entries(rng, ring_db_id, count, now_ns):
    weights = [k[5] for k in KINDS]
    for i in range(count):
        mime, ext, prefix, lo, hi, _ = rng.choices(KINDS, weights)[0]
        top = TOPS[i % len(TOPS)]
        year = 2020 + (i // 211) % 7
        month = 1 + (i // 17) % 12
        day = 1 + (i * 7) % 28
        path = f"{top}/{year}/{month:02d}/{prefix}_{year}{month:02d}{day:02d}_{i:08d}.{ext}"
        size = int(lo + (hi - lo) * rng.random() ** 2)
        mtime = now_ns - rng.randrange(0, 6 * 365) * NS_PER_DAY - rng.randrange(NS_PER_DAY)
        r = rng.random()
        if r < 0.02:
            state, local, remote, blake = "local-only", f"/storage/emulated/0/{path}", None, None
        elif r < 0.40:
            state, local, remote, blake = "synced", f"/storage/emulated/0/{path}", i + 1, rng.randbytes(32)
        else:
            state, local, remote, blake = "remote-only", None, i + 1, rng.randbytes(32)
        yield (ring_db_id, path, local, remote, size, mtime, mime, blake, None, state, 0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-n", "--count", type=int, default=1_000_000, help="entries in the first Ring (default 1000000)")
    ap.add_argument("-o", "--out", default="demo.db", help="output file (default ./demo.db, replaced if present)")
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()

    out = Path(args.out)
    for suffix in ("", "-wal", "-shm", "-journal"):
        Path(str(out) + suffix).unlink(missing_ok=True)
    rng = random.Random(args.seed)
    now_ns = int(time.time()) * 1_000_000_000
    t0 = time.time()

    db = sqlite3.connect(out)
    db.execute("PRAGMA journal_mode=OFF")
    db.execute("PRAGMA synchronous=OFF")
    db.execute("PRAGMA foreign_keys=ON")
    migrations = moon_migrations()
    for i, path in enumerate(migrations):
        db.executescript(path.read_text())
        db.execute(f"PRAGMA user_version={i + 1}")
    print(f"schema: {len(migrations)} migrations applied", flush=True)

    total = 0
    for idx, (name, lan, wan, offload, divisor) in enumerate(RINGS):
        cur = db.execute(
            "INSERT INTO rings (ring_id, name, lan_url, wan_url, tls_cert_sha256, device_uuid,"
            " device_slug, offload_threshold_bytes) VALUES (?,?,?,?,?,?,?,?)",
            (rng.randbytes(16), name, lan, wan, rng.randbytes(32), b"\xa1" * 16, "pixel-9", offload),
        )
        ring_id = cur.lastrowid
        db.execute("INSERT INTO sync_cursor (ring_id, last_seq) VALUES (?, 0)", (ring_id,))
        for local, folder in (("DCIM/Camera", "pixel-9/Camera"), ("Pictures/Screenshots", "pixel-9/Screenshots")):
            db.execute(
                "INSERT INTO watched_folders (ring_id, local_dir, ring_folder) VALUES (?,?,?)",
                (ring_id, f"/storage/emulated/0/{local}", folder),
            )
        n = max(args.count // divisor, 5)
        batch = []
        for row in entries(rng, ring_id, n, now_ns):
            batch.append(row)
            if len(batch) >= BATCH:
                insert(db, batch)
                total += len(batch)
                batch.clear()
                print(f"  {name}: {total:,} entries", end="\r", flush=True)
        insert(db, batch)
        total += len(batch)
        print(f"  {name}: {n:,} entries        ", flush=True)

    # Local-only entries wait in the outbox, which is what makes them "queued" in the app.
    db.execute(
        "INSERT INTO outbox (entry_id, hashed_size, hashed_mtime_ns)"
        " SELECT id, size, mtime_ns FROM entries WHERE state = 'local-only'"
    )
    # Lifetime counters, derived from the entries like migration 0003's backfill.
    db.execute(
        "UPDATE rings SET"
        " transferred_files = (SELECT COUNT(*) FROM entries e WHERE e.ring_id = rings.id AND e.remote_file_id IS NOT NULL),"
        " transferred_bytes = (SELECT COALESCE(SUM(e.size), 0) FROM entries e WHERE e.ring_id = rings.id AND e.remote_file_id IS NOT NULL)"
    )
    db.commit()
    db.execute("ANALYZE")
    db.commit()
    db.close()
    mib = out.stat().st_size / (1 << 20)
    print(f"{out}: {total:,} entries, {mib:,.0f} MiB, {time.time() - t0:.0f} s")


def insert(db, rows):
    db.executemany(
        "INSERT INTO entries (ring_id, ring_path, local_path, remote_file_id, size, mtime_ns,"
        " mime, blake3, thumb_blake3, state, suppressed) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        rows,
    )


if __name__ == "__main__":
    main()
