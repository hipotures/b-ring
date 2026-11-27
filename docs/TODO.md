# TODO

Open items, not scheduled. Add the date and what was measured when you close one.

## Android: baseline profile for the release build
- **Why:** cold start of the release build is still the slowest moment (first frames at ~400-450 ms were
  measured on the debug build; release was not measured separately). Compose without a baseline profile is
  compiled only after first use.
- **What:** add a baseline profile (`src/main/baseline-prof.txt`, or generate one with a Macrobenchmark
  module) covering the app, Compose and Coil, so the first launch is already compiled.
- **Measure first:** `am start -W` cold start and `dumpsys gfxinfo io.github.hipotures.bring` on the
  **release** build with `demo.db` (1M entries), before and after. Only worth it if release cold start is
  still noticeable.

## Index: state filter in very large flat folders
- **Why:** `files_for` with a state filter (Local / Synced / Remote) uses the `entries_dir`
  (`ring_id, dir, mtime_ns`) index and then filters rows by `state`. Measured on the phone: 66 ms for a
  folder of 2,757 files (the Local filter); it grows with the size of the folder.
- **What:** add an index that includes `state`, e.g. `entries(ring_id, dir, state, mtime_ns)`, as a new
  migration (`0006_...sql`; migrations are numbered in `crates/bring-core/src/db/mod.rs`), and check with
  `EXPLAIN QUERY PLAN` that `files_for` uses it.
- **Measure first:** a generated demo with one flat folder of several hundred thousand files
  (`dist/gen-demo-db.py` currently spreads files over year/month folders), then the `BringBrowse` log lines
  (`adb logcat -s BringBrowse`) with the filter on.
