#!/usr/bin/env bash
# Builds the Moon sync engine for Android (step 10b):
#   - libbring_uniffi.so per ABI   -> app/src/main/jniLibs/<abi>/
#   - Kotlin bindings              -> app/src/generated/kotlin/
# Both outputs are generated and gitignored.
#
# Target devices (phone, Meta Quest 3) are all arm64-v8a, the default.
# BRING_ABIS="arm64-v8a x86_64" also builds for the x86_64 emulator (used to
# look at the UI; see README).
#
# Requirements: the Android SDK with the pinned NDK below, `cargo-ndk`, and
# `rustup target add aarch64-linux-android` (plus x86_64-linux-android for
# the emulator).
#
# Usage: [BRING_ABIS="arm64-v8a x86_64"] android/build-rust.sh [--debug]
set -euo pipefail

NDK_VERSION="30.0.16248370"
# MANAGE_EXTERNAL_STORAGE (§4) exists from API 30.
MIN_SDK=30

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

read -r -a abis <<< "${BRING_ABIS:-arm64-v8a}"
ndk_targets=()
for abi in "${abis[@]}"; do
    case "$abi" in
        arm64-v8a | x86_64) ndk_targets+=(-t "$abi") ;;
        *) echo "unsupported ABI: $abi" >&2; exit 2 ;;
    esac
done
# Drop ABIs from earlier runs that are not asked for now.
for old in "$jni_dir"/*/; do
    [[ -d "$old" ]] || continue
    keep=0
    for abi in "${abis[@]}"; do [[ "$(basename "$old")" == "$abi" ]] && keep=1; done
    [[ $keep == 1 ]] || rm -rf "$old"
done

cargo ndk "${ndk_targets[@]}" -P "$MIN_SDK" -o "$jni_dir" \
    build "${cargo_profile_args[@]}" --locked -p bring-uniffi

# Library mode: the bindings are read from the metadata embedded in the very
# .so that ships, so they cannot drift from it.
case "${abis[0]}" in
    arm64-v8a) triple=aarch64-linux-android ;;
    x86_64) triple=x86_64-linux-android ;;
esac
lib="$root/target/$triple/$profile/libbring_uniffi.so"
rm -rf "$kotlin_dir"
cargo run --quiet --locked -p bring-uniffi --features bindgen --bin uniffi-bindgen -- \
    generate "$lib" --language kotlin --out-dir "$kotlin_dir" --no-format

echo "native libraries: ${abis[*]} in $jni_dir"
echo "kotlin bindings: $kotlin_dir"
