//! `uniffi-bindgen` pinned to the workspace's `uniffi` version, so the
//! generated Kotlin always matches the scaffolding in `libbring_uniffi.so`.

fn main() {
    uniffi::uniffi_bindgen_main();
}
