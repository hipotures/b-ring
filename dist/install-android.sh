#!/usr/bin/env bash
# Builds both Android variants and installs them side by side on a phone:
#   release  io.github.hipotures.bring        "B-Ring"      (minified; keeps the real data)
#   debug    io.github.hipotures.bring.debug  "B-Ring dbg"  (its own data)
# Refuses while an upload is running (installing restarts the app), unless --force.
# Usage: dist/install-android.sh [adb-target] [--force] [--no-rust]   (target e.g. 192.168.2.100:5555)
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] && { set -a; . ./.env; set +a; }
adb=("${ADB_BIN:-adb}")
target=""; force=0; rust=1
for a in "$@"; do
    case "$a" in
        --force) force=1 ;;
        --no-rust) rust=0 ;;
        *) target="$a" ;;
    esac
done
[ -z "$target" ] && target="$("${adb[@]}" devices | awk 'NR>1 && $2=="device" {print $1; exit}')"
[ -z "$target" ] && { echo "no adb device; pass a target" >&2; exit 1; }
adb+=(-s "$target")

# Same rule as AGENTS.md: no notification record of the app, no open upload on the Ring.
busy=""
if "${adb[@]}" shell dumpsys notification --noredact | grep -q "NotificationRecord.*io.github.hipotures.bring"; then
    busy="the app has a notification on $target"
fi
if [ -z "$busy" ] && [ -n "${RING_DB:-}" ] && [ -f "$RING_DB" ]; then
    n="$(python3 -I -c '
import sqlite3, sys
c = sqlite3.connect("file:" + sys.argv[1] + "?mode=ro", uri=True)
print(c.execute("select count(*) from uploads where state in (\"open\", \"committing\")").fetchone()[0])
' "$RING_DB")"
    [ "$n" != 0 ] && busy="$n upload(s) open on the Ring"
fi
if [ "$force" = 0 ] && [ -n "$busy" ]; then
    echo "not installing: $busy (wait, or pass --force)" >&2
    exit 1
fi

gradle=(./gradlew --no-daemon --max-workers=1)
[ "$rust" = 0 ] && gradle+=(-x cargoAndroid -x cargoHost)
(cd android && "${gradle[@]}" :app:assembleDebug :app:assembleRelease)
for v in release debug; do
    "${adb[@]}" install -r "android/app/build/outputs/apk/$v/app-$v.apk"
done
echo "Installed release + debug on $target."
