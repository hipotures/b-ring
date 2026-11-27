#!/usr/bin/env bash
# Copies a generated demo database (see dist/gen-demo-db.py) into the app's private files dir
# as files/demo.db. The real moon.db is never touched. Needs the debug build (run-as).
# Usage: dist/push-demo-db.sh [demo.db] [adb-target]   (target e.g. 192.168.2.100:5555)
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] && { set -a; . ./.env; set +a; }
db="${1:-demo.db}"
target="${2:-}"
adb=("${ADB_BIN:-adb}")
[ -n "$target" ] && adb+=(-s "$target")
tmp=/data/local/tmp/bring-demo.db
"${adb[@]}" push "$db" "$tmp"
"${adb[@]}" shell "run-as io.github.hipotures.bring sh -c 'rm -f files/demo.db files/demo.db-wal files/demo.db-shm; cp $tmp files/demo.db'"
"${adb[@]}" shell rm -f "$tmp"
echo "Pushed. In the app: Settings -> Demo mode Off, then On (or restart the app)."
