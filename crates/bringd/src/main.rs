//! `bringd` — the B-Ring daemon running on the Linux workstation (Ring).
//!
//! Scaffold only; the server, watcher and index arrive in Phase 1 step 3.

fn main() {
    println!(
        "bringd {} (protocol v{})",
        bring_core::VERSION,
        bring_core::PROTOCOL_VERSION
    );
}
