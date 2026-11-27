# Android ADB & Development Cheatsheet

Practical commands for building, installing, and inspecting the B-Ring Android app (`io.github.hipotures.bring`) over **Wi-Fi ADB**.

Copy `.env.example` to `.env` (gitignored) with your local `ADB_BIN` and `ADB_PHONE_IP`, then load it:

```sh
[ -f .env ] && set -a && . ./.env && set +a
export ADB="${ADB_BIN:-${ANDROID_HOME:-$HOME/Android/Sdk}/platform-tools/adb}"
```

---

## 1. Connecting over Wi-Fi ADB

Enable **Developer options → Wireless debugging** on the phone while connected to the same Wi-Fi/VLAN subnet as the dev machine (make sure **AP isolation** is disabled on the router).

### Discover the current Wireless Debugging port (mDNS)

Wireless Debugging changes port after toggling it or rebooting the phone. Discover the live `<ip>:<port>` automatically:

```sh
$ADB mdns services
# Example:
# adb-<SERIAL>-xxxxxx  _adb-tls-connect._tcp  192.0.2.10:33237
```

### Connect

```sh
$ADB connect <PHONE_IP>:<PORT>
$ADB devices -l
```

One-liner that auto-connects to the first discovered `_adb-tls-connect._tcp` service:

```sh
ADDR=$($ADB mdns services | awk '/_adb-tls-connect\._tcp/ { print $3; exit }')
[ -n "$ADDR" ] && $ADB connect "$ADDR" && $ADB devices -l
```

### First-time pairing (new phone or after resetting ADB keys)

On the phone: **Developer options → Wireless debugging → Pair device with pairing code**:

```sh
$ADB pair <PHONE_IP>:<PAIRING_PORT>
# enter the 6-digit code, then connect to the main port shown on the Wireless debugging screen:
$ADB connect <PHONE_IP>:<CONNECT_PORT>
```

> **Note:** Avoid running `adb kill-server` unless strictly necessary (especially when USB passthrough is in use).

---

## 2. Building & Installing the APK

Use `--no-daemon --max-workers=1` on low-RAM / zero-swap machines so Gradle does not keep a multi-GB daemon resident.

### Full build (Rust `bring-uniffi` + Kotlin)

```sh
cd android
./gradlew --no-daemon --max-workers=1 testDebugUnitTest assembleDebug
```

### Fast Kotlin-only build (when Rust crates did not change)

```sh
cd android
./gradlew --no-daemon --max-workers=1 -x cargoAndroid -x cargoHost testDebugUnitTest assembleDebug
```

### Safe install (check for active uploads first)

Installing with `adb install -r` restarts the app process. Always verify no upload is currently active first:

```sh
SERIAL=$($ADB devices | awk '/device$/ && !/^emulator-/ { print $1; exit }')
ACTIVE=$($ADB -s "$SERIAL" shell dumpsys notification --noredact | grep -c "NotificationRecord.*pkg=io\.github\.hipotures\.bring" || true)
echo "active_upload_notifs=$ACTIVE"

# Install only when ACTIVE == 0:
$ADB -s "$SERIAL" install -r android/app/build/outputs/apk/debug/app-debug.apk
```

---

## 3. Debugging & Inspecting the App

Set `SERIAL` once (either `<PHONE_IP>:<PORT>` or a USB serial):

```sh
SERIAL=$($ADB devices | awk '/device$/ && !/^emulator-/ { print $1; exit }')
```

### Live logs

```sh
# B-Ring logs + crashes
$ADB -s "$SERIAL" logcat -v time -s bring:* AndroidRuntime:E

# Clear log buffer first
$ADB -s "$SERIAL" logcat -c
```

### Check active upload progress

```sh
$ADB -s "$SERIAL" shell dumpsys notification --noredact \
  | grep -A 35 "NotificationRecord.*pkg=io\.github\.hipotures\.bring" \
  | grep -E "android\.title=|android\.text=|android\.progress="
```

### Inspect the app's SQLite database (`moon.db`)

Debug builds allow `run-as io.github.hipotures.bring`:

```sh
mkdir -p /tmp/bring-inspect
$ADB -s "$SERIAL" exec-out run-as io.github.hipotures.bring cat files/moon.db > /tmp/bring-inspect/moon.db
$ADB -s "$SERIAL" exec-out run-as io.github.hipotures.bring cat files/moon.db-wal > /tmp/bring-inspect/moon.db-wal 2>/dev/null || true
$ADB -s "$SERIAL" exec-out run-as io.github.hipotures.bring cat files/moon.db-shm > /tmp/bring-inspect/moon.db-shm 2>/dev/null || true

python3 - <<'PY'
import sqlite3
c = sqlite3.connect('/tmp/bring-inspect/moon.db')
print('=== RINGS ===')
for r in c.execute('SELECT id, name, device_slug, transferred_files, transferred_bytes FROM rings'): print(r)
print('=== WATCHED FOLDERS ===')
for r in c.execute('SELECT id, ring_id, local_dir, ring_folder, enabled FROM watched_folders'): print(r)
print('=== ENTRIES BY STATE ===')
for r in c.execute('SELECT ring_id, state, suppressed, COUNT(*), ROUND(SUM(size)/1e9, 2) FROM entries GROUP BY ring_id, state, suppressed'): print(r)
PY
```

### Launch / restart / screenshot

```sh
# Launch app
$ADB -s "$SERIAL" shell am start -n io.github.hipotures.bring/.MainActivity

# Force-stop app (only when idle)
$ADB -s "$SERIAL" shell am force-stop io.github.hipotures.bring

# Grab a screenshot from the phone
$ADB -s "$SERIAL" exec-out screencap -p > /tmp/phone-screen.png
```

---

## 4. Ring Server Helpers

```sh
# Rebuild & safely restart bringd + bring CLI (refuses if uploads are active)
./dist/update-ring.sh --pull

# Inspect daemon state & paired phones (with 8-char device IDs)
bring status
bring devices
bring rename <id|slug> "<New Name>"
```

---

## 5. Background sync does not start on its own (Samsung)

**Symptom:** a new photo or video (for example one saved from TikTok into `DCIM/Camera`) does not sync for minutes. As soon as the app is opened, the sync starts. The phone is on Wi‑Fi and the screen can even be on.

**Cause:** Samsung's FreecessController freezes the app shortly after it leaves the foreground. Along with that it puts the app's UID into the `OEM_DENY_1` firewall chain, which cuts its network. JobScheduler then treats `CONNECTIVITY` as unsatisfied, so the WorkManager jobs (the MediaStore trigger and the 15‑min periodic round) never start. And only a running job unfreezes the app, so nothing moves until the app is opened.

Seen on 2026‑11‑26 on the S23 Ultra (One UI 9, it showed up after a system update about 1–2 months earlier) with the battery set to **Optimised**. `OEM_DENY_1` was also present on the S20 FE (One UI 5.1), but there it was not tested end to end.

**Diagnosis:**

```sh
PKG=io.github.hipotures.bring
UID=$($ADB -s "$SERIAL" shell cmd package list packages -U --user 0 $PKG | grep -oE 'uid:[0-9]+' | cut -d: -f2)
$ADB -s "$SERIAL" shell dumpsys connectivity trafficcontroller | grep -E "^\s+$UID .*MATCH"  # OEM_DENY_1_MATCH = blocked
$ADB -s "$SERIAL" shell dumpsys jobscheduler | grep -E "UID: $UID; Network"                  # blocked=REASON_OEM_DENY
$ADB -s "$SERIAL" shell dumpsys jobscheduler $PKG | grep -E "Unsatisfied constraints|Trigger content URIs"
$ADB -s "$SERIAL" logcat -d | grep -iE "Freecess.*($PKG|$UID)"                                 # UFZ ... reason: job
```

Force-run a job, ignoring its constraints (WorkManager jobs live in their own namespace):

```sh
$ADB -s "$SERIAL" shell cmd jobscheduler run -f -u 0 -n androidx.work.systemjobscheduler $PKG <job-id>
```

**Workaround:** go to Settings → Apps → B-Ring → Battery and pick **Unrestricted**. If the app is already frozen, the setting takes effect only after the next unfreeze (opening the app or a forced job run). After that the log shows `FreecessHandler: stop trying to freeze io.github.hipotures.bring`, and the media trigger fires within about 10 s of a file being saved (confirmed on the S23).

"Optimised" is the default after a fresh install, so the problem comes back after reinstalling and for new users. Possible code fix: drop the network constraint from the media and periodic requests and check unmetered Wi‑Fi inside `SyncWorker` (a running job unfreezes the app and gets network), and/or ask for the battery-optimisation exemption when pairing.

## Demo database (large fake index for load testing)

Demo mode normally shows a small hand-made sample (`DemoData.kt`). If a generated
`files/demo.db` is on the phone, Demo mode reads it through its own `Moon`
instead, so the real `moon.db` (and its backups) is never touched.

```sh
dist/gen-demo-db.py                  # 1,000,000 entries -> ./demo.db (gitignored)
dist/gen-demo-db.py -n 10000000 -o /tmp/demo10m.db
dist/push-demo-db.sh demo.db 192.168.2.100:5555   # debug build only (run-as)
```

The generator applies the real migrations from `crates/bring-core/src/db/moon/`
(order taken from `db/mod.rs`), so schema changes need no change there; only its
`INSERT`s may need updating. Ring counters are derived from the generated rows.
Remove the file (`run-as ... rm files/demo.db`) to get the hand-made sample back.
Demo mode has no local files behind it: `local_path`s do not exist on the phone
and sync/offload actions are disabled in Demo mode.

## Release and debug side by side

`dist/install-android.sh [adb-target]` builds and installs both variants:

| | package | label | notes |
|---|---|---|---|
| release | `io.github.hipotures.bring` | B-Ring | R8-minified, signed with the local debug keystore; upgrades in place, keeps real data; no `run-as` |
| debug | `io.github.hipotures.bring.debug` | B-Ring dbg | own empty data; `run-as` works |

Demo database: `dist/push-demo-db.sh` writes `demo.db` to
`/sdcard/Android/data/<package>/files/` of both (the app copies it into private storage).
The debug build has no Settings before pairing; to start it in Demo mode, write
`shared_prefs/prefs.xml` with `demo_mode=true` via `run-as io.github.hipotures.bring.debug`.
Speed comparisons: `adb shell dumpsys gfxinfo <package> reset`, do the same clicks in both,
then `dumpsys gfxinfo <package>`.
