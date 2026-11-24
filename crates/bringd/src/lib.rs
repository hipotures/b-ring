//! `bringd` — the B-Ring daemon running on the Linux workstation (Ring).
//!
//! The binary (`main.rs`) is a thin wrapper; everything lives here so
//! integration tests can drive the real services.

pub mod config;
pub mod ctl_server;
pub mod devices;
pub mod hasher;
pub mod pairing;
pub mod ring;
pub mod scanner;
pub mod serve;
pub mod server;
pub mod store;
pub mod stream;
pub mod thumbs;
pub mod tls;
pub mod uploads;
pub mod watcher;

pub use config::Config;
pub use ring::Ring;
