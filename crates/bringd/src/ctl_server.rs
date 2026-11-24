//! Control socket for the `bring` CLI (JSON lines, see `bring_core::ctl`).
//!
//! The socket lives in a `0700` directory and is itself `0600`: only the
//! user running `bringd` can pair or revoke devices.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use bring_core::ctl::{CtlEvent, CtlRequest, DeviceInfo, StatusInfo};
use bring_core::db::DbError;
use rusqlite::Connection;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::{Instant, timeout_at};
use tracing::{info, warn};

use crate::devices::{self, Device};
use crate::pairing::{PairStatus, Pairing};
use crate::store::Db;

/// What the control socket needs to answer requests.
#[derive(Clone)]
pub struct CtlContext {
    pub db: Db,
    pub pairing: Pairing,
    pub ring_id: String,
    pub ring_root: PathBuf,
    pub tls_cert_sha256: String,
    pub wan_url: Option<String>,
}

/// Binds the socket, replacing a stale one but refusing to steal a live one.
pub async fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    if path.exists() {
        if UnixStream::connect(path).await.is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("{} is in use (is bringd already running?)", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Accepts connections forever.
pub async fn run(listener: UnixListener, ctx: CtlContext) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, ctx).await {
                        warn!(error = %e, "control connection failed");
                    }
                });
            }
            Err(e) => {
                warn!(error = %e, "control accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

async fn send(w: &mut OwnedWriteHalf, ev: &CtlEvent) -> io::Result<()> {
    let mut line = serde_json::to_vec(ev).map_err(io::Error::other)?;
    line.push(b'\n');
    w.write_all(&line).await
}

fn info(d: Device) -> DeviceInfo {
    DeviceInfo {
        id: d.id,
        name: d.name,
        slug: d.slug,
        created_at: d.created_at,
        last_seen_at: d.last_seen_at,
        revoked: d.revoked_at.is_some(),
    }
}

fn status(c: &Connection, ctx: &CtlContext) -> Result<StatusInfo, DbError> {
    let count = |sql: &str| -> Result<u64, DbError> { Ok(c.query_row(sql, [], |r| r.get(0))?) };
    Ok(StatusInfo {
        version: bring_core::VERSION.to_owned(),
        ring_id: ctx.ring_id.clone(),
        ring_root: ctx.ring_root.display().to_string(),
        lan_url: ctx.pairing.lan_url(),
        wan_url: ctx.wan_url.clone(),
        tls_cert_sha256: ctx.tls_cert_sha256.clone(),
        files: count("SELECT COUNT(*) FROM files WHERE deleted = 0")?,
        bytes: count("SELECT COALESCE(SUM(size), 0) FROM files WHERE deleted = 0")?,
        pending_hashes: count("SELECT COUNT(*) FROM files WHERE deleted = 0 AND blake3 IS NULL")?,
        devices: count("SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL")?,
        open_uploads: count("SELECT COUNT(*) FROM uploads WHERE state IN ('open', 'committing')")?,
    })
}

async fn handle(stream: UnixStream, ctx: CtlContext) -> io::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut lines = BufReader::new(r).lines();
    let Some(line) = lines.next_line().await? else {
        return Ok(());
    };
    let req: CtlRequest = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            return send(
                &mut w,
                &CtlEvent::Error {
                    message: format!("bad request: {e}"),
                },
            )
            .await;
        }
    };
    let db = ctx.db.clone();
    let reply = match req {
        CtlRequest::Status => {
            let c2 = ctx.clone();
            db.call(move |c| status(c, &c2)).await.map(CtlEvent::Status)
        }
        CtlRequest::Devices => db
            .call(|c| devices::list(c))
            .await
            .map(|d| CtlEvent::Devices {
                devices: d.into_iter().map(info).collect(),
            }),
        CtlRequest::Revoke { device } => {
            db.call(move |c| -> Result<CtlEvent, DbError> {
                let Some(d) = devices::find(c, &device)? else {
                    return Ok(CtlEvent::Error {
                        message: format!("no device {device:?}"),
                    });
                };
                devices::revoke(c, d.id, devices::now_secs())?;
                let d = devices::by_id(c, d.id)?.expect("exists");
                info!(device = %d.slug, "device revoked");
                Ok(CtlEvent::Revoked { device: info(d) })
            })
            .await
        }
        CtlRequest::Pair => return pair(&ctx, &mut lines, &mut w).await,
        CtlRequest::PairDecision { .. } => Ok(CtlEvent::Error {
            message: "no pairing in progress".into(),
        }),
    };
    let ev = reply.unwrap_or_else(|e| CtlEvent::Error {
        message: e.to_string(),
    });
    send(&mut w, &ev).await
}

/// The `pair` conversation. Closing the connection cancels the session.
async fn pair(
    ctx: &CtlContext,
    lines: &mut Lines<BufReader<OwnedReadHalf>>,
    w: &mut OwnedWriteHalf,
) -> io::Result<()> {
    let (payload, mut rx) = match ctx.pairing.start() {
        Ok(v) => v,
        Err(e) => {
            return send(w, &CtlEvent::PairFailed { message: e.message }).await;
        }
    };
    let id = payload.pairing_id;
    let ttl = ctx.pairing.ttl();
    let deadline = Instant::now() + ttl;
    send(
        w,
        &CtlEvent::PairPayload {
            text: payload.to_text(),
            lan_url: ctx.pairing.lan_url(),
            expires_in_secs: ttl.as_secs(),
        },
    )
    .await?;
    info!(pairing = %id.to_hex(), "pairing session opened");

    // Wait for a Moon to complete the handshake.
    let (sas, device_name) = loop {
        tokio::select! {
            changed = timeout_at(deadline, rx.changed()) => {
                match changed {
                    Err(_) => {
                        ctx.pairing.close(&id, PairStatus::Expired);
                        return send(w, &CtlEvent::PairExpired).await;
                    }
                    Ok(Err(_)) => return Ok(()), // session dropped
                    Ok(Ok(())) => {}
                }
                let status = rx.borrow_and_update().clone();
                match status {
                    PairStatus::AwaitingUser { sas, device_name } => break (sas, device_name),
                    PairStatus::TooManyAttempts => {
                        ctx.pairing.close(&id, PairStatus::TooManyAttempts);
                        return send(w, &CtlEvent::PairFailed {
                            message: "too many failed attempts".into(),
                        }).await;
                    }
                    PairStatus::Expired => return send(w, &CtlEvent::PairExpired).await,
                    _ => {}
                }
            }
            _ = lines.next_line() => {
                // The CLI hung up (or spoke out of turn): cancel.
                ctx.pairing.close(&id, PairStatus::Rejected);
                return Ok(());
            }
        }
    };
    send(w, &CtlEvent::PairSas { sas, device_name }).await?;

    let decision = match timeout_at(deadline, lines.next_line()).await {
        Ok(Ok(Some(line))) => match serde_json::from_str::<CtlRequest>(&line) {
            Ok(CtlRequest::PairDecision { approve }) => Some(approve),
            _ => None,
        },
        _ => None,
    };
    let Some(approve) = decision else {
        ctx.pairing.close(&id, PairStatus::Rejected);
        return send(w, &CtlEvent::PairRejected).await;
    };
    let pairing = ctx.pairing.clone();
    let decided = tokio::task::spawn_blocking(move || pairing.decide(&id, approve))
        .await
        .map_err(io::Error::other)?;
    let ev = match decided {
        Ok(Some(device)) => {
            info!(device = %device.slug, "device paired");
            CtlEvent::PairDone {
                device: info(device),
            }
        }
        Ok(None) => CtlEvent::PairRejected,
        Err(e) => CtlEvent::PairFailed { message: e.message },
    };
    send(w, &ev).await
}
