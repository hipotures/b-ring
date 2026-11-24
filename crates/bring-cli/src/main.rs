//! `bring` — companion CLI for `bringd`.
//!
//! Scaffold only; `status`, `pair`, `devices` and `revoke` arrive in Phase 1
//! step 8.

fn main() {
    println!("bring {}", bring_core::VERSION);
}
