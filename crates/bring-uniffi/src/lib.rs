//! UniFFI bindings exposing B-Ring to the Android client.
//!
//! Proc-macro mode: no UDL file. Scaffold only; the API client, sync engine
//! and pairing state machine arrive in Phase 1 step 10.

uniffi::setup_scaffolding!();

/// Version of the B-Ring core library linked into the app.
#[uniffi::export]
pub fn core_version() -> String {
    bring_core::VERSION.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_version_matches_crate() {
        assert_eq!(core_version(), env!("CARGO_PKG_VERSION"));
    }
}
