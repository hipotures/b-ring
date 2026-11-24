//! `bringd` — the B-Ring daemon.

use std::path::PathBuf;
use std::process::ExitCode;

use bringd::serve::{self, Servers};
use bringd::{Config, Ring};
use tracing::{error, info};

fn usage() -> ! {
    eprintln!("usage: bringd [--config <path>]");
    std::process::exit(2);
}

fn parse_args() -> PathBuf {
    let mut args = std::env::args().skip(1);
    let mut config = Config::default_path();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => config = args.next().map(PathBuf::from).unwrap_or_else(|| usage()),
            "--version" | "-V" => {
                println!("bringd {}", bring_core::VERSION);
                std::process::exit(0);
            }
            _ => usage(),
        }
    }
    config
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("cannot install SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
    info!("shutting down");
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_path = parse_args();
    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let ring = match Ring::start(&config).await {
        Ok(r) => r,
        Err(e) => {
            error!("cannot start: {e}");
            return ExitCode::FAILURE;
        }
    };
    info!(
        lan = %config.lan_listen,
        proxy = ?config.proxy_listen,
        ring_id = %ring.ring_id,
        tls_sha256 = %ring.tls.fingerprint_hex(),
        "bringd starting"
    );

    let servers = Servers::default();
    let stopper = servers.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        stopper.shutdown();
    });
    let served = serve::serve(
        ring.router(),
        &ring.tls,
        config.lan_listen,
        config.proxy_listen,
        servers,
    )
    .await;
    if let Err(e) = served {
        error!("{e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
