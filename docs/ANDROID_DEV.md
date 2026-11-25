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
