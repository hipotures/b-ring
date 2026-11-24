#!/usr/bin/env bash
# Build & install bringd + bring CLI and safely restart the systemd user service.
#
# Usage:
#   ./dist/update-ring.sh [--pull] [--force]
#
#   --pull   Run `git pull --ff-only` in the repository before building.
#   --force  Restart bringd even if uploads are currently in progress.

set -euo pipefail

PULL=0
FORCE=0

for arg in "$@"; do
    case "$arg" in
        --pull)  PULL=1 ;;
        --force) FORCE=1 ;;
        -h|--help)
            sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "Unknown option: $arg (use --help)" >&2
            exit 2
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
UNIT_SRC="$REPO_ROOT/dist/systemd/bringd.service"
UNIT_DST="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/bringd.service"

check_uploads() {
    [ "$FORCE" -eq 1 ] && return 0
    command -v bring >/dev/null 2>&1 || return 0
    local status_out uploads
    status_out="$(bring status 2>/dev/null || true)"
    uploads="$(printf '%s\n' "$status_out" | awk '/^Uploads[[:space:]]+/ { print $2; exit }')"
    if [ -n "${uploads:-}" ] && [ "$uploads" -gt 0 ] 2>/dev/null; then
        echo "bringd has $uploads upload(s) in progress; refusing to restart." >&2
        echo "Wait for uploads to finish or re-run with --force." >&2
        return 1
    fi
}

if [ "$PULL" -eq 1 ]; then
    echo "==> Pulling latest changes..."
    git -C "$REPO_ROOT" pull --ff-only
fi

check_uploads

echo "==> Installing bringd and bring-cli..."
cargo install --path "$REPO_ROOT/crates/bringd" --locked
cargo install --path "$REPO_ROOT/crates/bring-cli" --locked

if [ ! -f "$UNIT_DST" ] || ! cmp -s "$UNIT_SRC" "$UNIT_DST"; then
    echo "==> Updating $UNIT_DST..."
    install -Dm644 "$UNIT_SRC" "$UNIT_DST"
    systemctl --user daemon-reload
fi

check_uploads

echo "==> Restarting bringd.service..."
if systemctl --user is-enabled bringd.service >/dev/null 2>&1; then
    systemctl --user restart bringd.service
else
    systemctl --user enable --now bringd.service
fi

# Wait briefly for control socket readiness.
for _ in 1 2 3 4 5 6 7 8 9 10; do
    if bring status >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

echo "==> Status:"
bring status
echo
bring devices
