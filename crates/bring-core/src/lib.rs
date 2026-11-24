//! Shared B-Ring domain logic.
//!
//! This crate must stay free of async runtimes (`tokio`, `axum`, ...) so it
//! compiles cleanly for Android through `bring-uniffi`. See
//! `docs/ARCHITECTURE.md`.

/// Version of the Ring <-> Moon HTTP protocol (served under `/api/v1`).
pub const PROTOCOL_VERSION: u32 = 1;

/// Version of the B-Ring crates.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_matches_api_prefix() {
        assert_eq!(format!("/api/v{PROTOCOL_VERSION}"), "/api/v1");
    }
}
