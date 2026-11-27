#!/usr/bin/env bash
# Puts a generated demo database (see dist/gen-demo-db.py) where the app finds it: the app's external
# files dir, /sdcard/Android/data/<package>/files/demo.db, which adb can write for both the
# release build and the debug build (no run-as needed). The app copies it into private storage on
# next use. The real moon.db is never touched.
# Usage: dist/push-demo-db.sh [demo.db] [adb-target] [package]
#   package defaults to both builds: io.github.hipotures.bring and io.github.hipotures.bring.debug
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f .env ] && { set -a; . ./.env; set +a; }
db="${1:-demo.db}"
target="${2:-}"
adb=("${ADB_BIN:-adb}")
[ -n "$target" ] && adb+=(-s "$target")
packages=("${3:-io.github.hipotures.bring}")
[ -z "${3:-}" ] && packages+=(io.github.hipotures.bring.debug)
for pkg in "${packages[@]}"; do
    dir="/sdcard/Android/data/$pkg/files"
    "${adb[@]}" shell "mkdir -p $dir"
    "${adb[@]}" push "$db" "$dir/demo.db"
done
echo "Pushed. In the app: Settings -> Demo mode Off, then On (or restart the app)."
