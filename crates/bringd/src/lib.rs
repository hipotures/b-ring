//! `bringd` — the B-Ring daemon running on the Linux workstation (Ring).
//!
//! The binary (`main.rs`) is a thin wrapper; everything lives here so
//! integration tests can drive the real services.

pub mod config;
pub mod hasher;
pub mod ring;
pub mod scanner;
pub mod server;
pub mod store;
pub mod thumbs;
pub mod watcher;

pub use config::Config;
pub use ring::Ring;
