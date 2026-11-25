#!/usr/bin/env bash
# Capture the 12-shot Demo mode gallery for README.md from an unlocked Android
# phone connected via ADB. Automatically crops away the system status bar and
# navigation bar using the live Compose root bounds, and always disables Demo
# mode on exit.
#
# Usage:
#   ./dist/capture-gallery.sh [-s <adb-serial>] [-o <output-dir>] [--force]
#
# Defaults:
#   -s  first connected device from `$ADB_BIN devices` (or `$ADB_PHONE_IP:5555` if set in .env)
#   -o  docs/screenshots
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "$REPO_ROOT/.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "$REPO_ROOT/.env"
  set +a
fi

ADB="${ADB_BIN:-adb}"
SERIAL=""
OUT_DIR="$REPO_ROOT/docs/screenshots"
FORCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -s|--serial) SERIAL="$2"; shift 2 ;;
    -o|--out)    OUT_DIR="$2"; shift 2 ;;
    --force)     FORCE=1; shift ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$SERIAL" && -n "${SCREENSHOT_DEVICE:-}" ]]; then
  SERIAL="$SCREENSHOT_DEVICE"
fi
if [[ -z "$SERIAL" && -n "${ADB_PHONE_IP:-}" ]]; then
  cand="${ADB_PHONE_IP}:5555"
  if "$ADB" devices | awk 'NR>1 && $2=="device" {print $1}' | grep -qxF "$cand"; then
    SERIAL="$cand"
  fi
fi
if [[ -z "$SERIAL" ]]; then
  SERIAL="$("$ADB" devices | awk 'NR>1 && $2=="device" {print $1; exit}')"
fi
if [[ -z "$SERIAL" ]]; then
  echo "No connected ADB device found. Connect a phone or pass -s <serial>." >&2
  exit 1
fi

command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }
python3 -c "import PIL" 2>/dev/null || { echo "Python Pillow (PIL) is required for cropping" >&2; exit 1; }

adb_dev() { "$ADB" -s "$SERIAL" "$@"; }

if [[ "$FORCE" -eq 0 ]]; then
  active_sync="$(adb_dev shell dumpsys notification --noredact | grep -c 'NotificationRecord.*io\.github\.hipotures\.bring' || true)"
  if [[ "$active_sync" -gt 0 ]]; then
    echo "Refusing to run on $SERIAL: active sync/upload notification detected (use --force to override)." >&2
    exit 1
  fi
fi

if adb_dev shell "dumpsys window" | grep -q "mDreamingLockscreen=true"; then
  echo "Phone ($SERIAL) is locked. Unlock the screen and re-run." >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
TMP_DIR="$(mktemp -d)"
DEMO_ENABLED=0

dump_ui() {
  adb_dev shell "uiautomator dump /sdcard/bring_ui.xml >/dev/null && cat /sdcard/bring_ui.xml && rm -f /sdcard/bring_ui.xml" > "$TMP_DIR/ui.xml"
}

# Find center (x, y) of the first node matching attr=value (or prefix/substring with ~=)
find_xy() {
  local mode="$1" query="$2"
  python3 - "$TMP_DIR/ui.xml" "$mode" "$query" <<'PY'
import re, sys, xml.etree.ElementTree as ET
xml_path, mode, query = sys.argv[1], sys.argv[2], sys.argv[3]
root = ET.parse(xml_path).getroot()
for node in root.iter("node"):
    val = node.attrib.get("text" if mode in ("text", "text~") else "content-desc", "")
    hit = (val == query) if not mode.endswith("~") else (query in val)
    if hit:
        m = re.match(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.attrib.get("bounds", ""))
        if m:
            x1, y1, x2, y2 = map(int, m.groups())
            print((x1 + x2) // 2, (y1 + y2) // 2)
            sys.exit(0)
sys.exit(1)
PY
}

tap_node() {
  local mode="$1" query="$2"
  dump_ui
  local xy
  xy="$(find_xy "$mode" "$query")" || { echo "Could not find UI element ($mode=$query)" >&2; return 1; }
  # shellcheck disable=SC2086
  adb_dev shell input tap $xy
  sleep 0.65
}

# Detect app viewport bounds (excludes status bar & nav bar)
detect_crop_box() {
  dump_ui
  python3 - "$TMP_DIR/ui.xml" <<'PY'
import re, sys, xml.etree.ElementTree as ET
root = ET.parse(sys.argv[1]).getroot()
# First inner View whose top > 0 and bottom < screen height is the safeDrawingPadding container
full = None
for node in root.iter("node"):
    m = re.match(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.attrib.get("bounds", ""))
    if not m:
        continue
    x1, y1, x2, y2 = map(int, m.groups())
    if full is None:
        full = (x1, y1, x2, y2)
        continue
    if x1 == 0 and x2 == full[2] and y1 > 0 and y2 < full[3]:
        print(f"{x1},{y1},{x2},{y2}")
        sys.exit(0)
print(f"{full[0]},{full[1]},{full[2]},{full[3]}")
PY
}

CROP_BOX=""
cap() {
  local name="$1"
  sleep 0.85
  adb_dev exec-out screencap -p > "$TMP_DIR/raw.png"
  python3 - "$TMP_DIR/raw.png" "$OUT_DIR/$name" "$CROP_BOX" <<'PY'
import sys
from PIL import Image
src, dst, box = sys.argv[1], sys.argv[2], tuple(map(int, sys.argv[3].split(",")))
Image.open(src).crop(box).save(dst)
PY
  echo "  captured $name"
}

back() {
  adb_dev shell input keyevent 4
  sleep 0.55
}

cleanup() {
  if [[ "$DEMO_ENABLED" -eq 1 ]]; then
    echo "==> Restoring normal mode..."
    adb_dev shell am start -n io.github.hipotures.bring/.MainActivity >/dev/null 2>&1 || true
    sleep 0.8
    # Back out to root if on a subscreen
    for _ in 1 2 3; do
      dump_ui 2>/dev/null || break
      if find_xy desc "Settings" >/dev/null 2>&1; then break; fi
      back
    done
    if tap_node desc "Settings" 2>/dev/null; then
      tap_node text "Off" 2>/dev/null || true
      tap_node text "System" 2>/dev/null || true
      back
    fi
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "==> Launching B-Ring on $SERIAL..."
adb_dev shell am start -n io.github.hipotures.bring/.MainActivity >/dev/null
sleep 1.2

CROP_BOX="$(detect_crop_box)"
echo "==> App viewport crop box: $CROP_BOX"

# Open Settings -> enable Demo mode + Light theme
tap_node desc "Settings"
tap_node text "On"
DEMO_ENABLED=1
tap_node text "Light"
back

# Ensure we are at the root folder (Watch folder '+' only appears at root)
for _ in 1 2 3 4; do
  dump_ui
  if find_xy desc "Watch folder" >/dev/null 2>&1; then break; fi
  back
done

echo "==> Capturing Light theme..."
cap 01_light_home_nas.png

tap_node text "pixel-9"
cap 02_light_folder_pixel9.png

tap_node text "Camera"
cap 03_light_files_camera.png

back
back

tap_node text~ "LAN ·"
cap 05_light_activity.png
back

tap_node desc "Ring settings"
cap 06_light_ring_settings.png
# Scroll down to capture Watched folders + Offload settings
adb_dev shell input swipe 540 1800 540 300 350
sleep 0.5
cap 04_light_watched_folders.png
back

echo "==> Capturing Dark theme..."
tap_node desc "Settings"
tap_node text "Dark"
cap 10_dark_settings.png
back

cap 07_dark_home_nas.png

tap_node text "pixel-9"
tap_node text "Documents"
cap 08_dark_files_documents.png

back
back

tap_node text~ "LAN ·"
cap 09_dark_activity.png
back

echo "==> Capturing Coffee theme..."
tap_node desc "Settings"
tap_node text "Coffee"
back

tap_node text "archive-vps"
cap 11_coffee_home_archive.png

tap_node desc "Add Ring"
cap 12_coffee_add_ring.png
back

echo "==> Done! Saved 12 cropped screenshots to $OUT_DIR"
