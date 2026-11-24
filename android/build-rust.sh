#!/usr/bin/env bash
# Builds the Moon sync engine for Android (step 10b):
#   - libbring_uniffi.so for arm64-v8a  -> app/src/main/jniLibs/arm64-v8a/
#   - Kotlin bindings                    -> app/src/generated/kotlin/
# Both outputs are generated and gitignored.
#
# Target devices (phone, Meta Quest 3) are all arm64-v8a: no x86_64, no emulator.
#
# Requirements: the Android SDK with the pinned NDK below, `cargo-ndk`, and
# `rustup target add aarch64-linux-android`.
#
# Usage: android/build-rust.sh [--debug]
set -euo pipefail

NDK_VERSION="30.0.16248370"
# MANAGE_EXTERNAL_STORAGE (§4) exists from API 30.
MIN_SDK=30
TARGET_TRIPLE="aarch64-linux-android"

profile=release
if [[ "${1:-}" == "--debug" ]]; then
    profile=debug
elif [[ $# -gt 0 ]]; then
    echo "usage: $0 [--debug]" >&2
    exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname "$here")"
cd "$root"

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/$NDK_VERSION}"
if [[ ! -d "$ANDROID_NDK_HOME" ]]; then
    echo "NDK not found at $ANDROID_NDK_HOME" >&2
    echo "install it with: sdkmanager \"ndk;$NDK_VERSION\"" >&2
    exit 1
fi

jni_dir="$here/app/src/main/jniLibs"
kotlin_dir="$here/app/src/generated/kotlin"

cargo_profile_args=()
[[ "$profile" == release ]] && cargo_profile_args=(--release)

cargo ndk -t arm64-v8a -P "$MIN_SDK" -o "$jni_dir" \
    build "${cargo_profile_args[@]}" --locked -p bring-uniffi

# Library mode: the bindings are read from the metadata embedded in the very
# .so that ships, so they cannot drift from it.
lib="$root/target/$TARGET_TRIPLE/$profile/libbring_uniffi.so"
rm -rf "$kotlin_dir"
cargo run --quiet --locked -p bring-uniffi --features bindgen --bin uniffi-bindgen -- \
    generate "$lib" --language kotlin --out-dir "$kotlin_dir" --no-format

echo "native library: $jni_dir/arm64-v8a/libbring_uniffi.so"
echo "kotlin bindings: $kotlin_dir"
