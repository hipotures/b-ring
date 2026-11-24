//! UniFFI bindings: the Moon's sync engine, called from Kotlin.
//!
//! Proc-macro mode (no UDL). The API is **blocking**: Kotlin calls it from
//! `Dispatchers.IO` or a `Worker`; internally one Tokio runtime drives
//! `reqwest`. Long operations check a cancel flag ([`Moon::cancel`]) between
//! files and chunks.
//!
//! The device token is never stored here: Kotlin keeps it in the Android
//! Keystore and hands it over with [`Moon::set_device_token`] after start.

uniffi::setup_scaffolding!();

mod client;
mod error;
mod pairing;
mod store;
mod sync;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bring_core::api::{HealthResponse, StreamUrlResponse};
use bring_core::codec::hex_encode;
use bring_core::db::{self, Role};
use bring_core::offload::OffloadPolicy;
use bring_core::{FileState, RelPath, transfer};
use reqwest::header::RANGE;
use reqwest::{Client, RequestBuilder, StatusCode};
use rusqlite::{Connection, params};
use serde::de::DeserializeOwned;

pub use error::MoonError;
pub use pairing::{PairedRing, PairingSas};
pub use store::WatchedFolder;
pub use sync::{IndexReport, OutboxReport, ScanReport, SyncReport};

/// The version of the core library (shown in the app's "about" screen).
#[uniffi::export]
pub fn core_version() -> String {
    bring_core::VERSION.to_owned()
}

/// True if `code` is a well-formed pairing code (checksum included). Lets the
/// manual-entry field flag typos before they burn a pairing attempt.
#[uniffi::export]
pub fn is_valid_pairing_code(code: String) -> bool {
    bring_core::pairing::BootstrapPayload::from_text(&code).is_ok()
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Which listener requests go to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Route {
    /// `lan_url`, pinned self-signed certificate.
    Lan,
    /// `wan_url` behind a reverse proxy with a real certificate.
    Wan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum EntryState {
    LocalOnly,
    Synced,
    RemoteOnly,
}

impl From<FileState> for EntryState {
    fn from(s: FileState) -> Self {
        match s {
            FileState::LocalOnly => Self::LocalOnly,
            FileState::Synced => Self::Synced,
            FileState::RemoteOnly => Self::RemoteOnly,
        }
    }
}

/// The paired Ring.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RingInfo {
    pub ring_id: String,
    pub lan_url: String,
    pub wan_url: Option<String>,
    /// Hex SHA-256 pin of the LAN certificate (also for Media3's OkHttp).
    pub tls_cert_sha256: String,
    pub device_slug: String,
}

impl From<&store::RingRow> for RingInfo {
    fn from(r: &store::RingRow) -> Self {
        Self {
            ring_id: hex_encode(&r.ring_id),
            lan_url: r.lan_url.clone(),
            wan_url: r.wan_url.clone(),
            tls_cert_sha256: hex_encode(&r.pin),
            device_slug: r.device_slug.clone(),
        }
    }
}

/// One file as the browser shows it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Entry {
    pub id: i64,
    pub ring_path: String,
    pub local_path: Option<String>,
    pub remote_file_id: Option<i64>,
    pub size: u64,
    pub mtime_ns: i64,
    pub mime: Option<String>,
    pub blake3: Option<String>,
    /// Pass to [`Moon::thumbnail`].
    pub thumb: Option<String>,
    pub state: EntryState,
    /// Never uploaded automatically again (§5.1).
    pub suppressed: bool,
    pub upload_pending: bool,
    pub last_error: Option<String>,
}

/// What Media3 needs to play a file over its own (pinned) OkHttp client.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaSource {
    pub url: String,
    /// Value of the `Authorization` header.
    pub authorization: String,
    /// Certificate pin (hex SHA-256) on the LAN route; `None` on WAN.
    pub tls_cert_sha256: Option<String>,
}

/// Progress of the current upload; called from the syncing thread.
#[uniffi::export(with_foreign)]
pub trait TransferListener: Send + Sync {
    fn on_upload_progress(&self, ring_path: String, sent_bytes: u64, total_bytes: u64);
}

/// Everything one authenticated request needs.
pub(crate) struct Session {
    pub ring: store::RingRow,
    base: String,
    token: String,
    http: Client,
}

impl Session {
    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{path}", self.base)
    }

    pub(crate) fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(self.url(path)).bearer_auth(&self.token)
    }

    pub(crate) fn post(&self, path: &str) -> RequestBuilder {
        self.http.post(self.url(path)).bearer_auth(&self.token)
    }

    pub(crate) async fn send(&self, req: RequestBuilder) -> Result<reqwest::Response, MoonError> {
        client::send(req).await
    }

    pub(crate) async fn json<T: DeserializeOwned>(
        &self,
        req: RequestBuilder,
    ) -> Result<T, MoonError> {
        client::json(req).await
    }
}

/// HTTP clients, rebuilt when the pin changes (re-pairing).
struct Clients {
    pin: [u8; 32],
    lan: Client,
    wan: Option<Client>,
}

/// Budget for the LAN `/health` probe. A cold probe (new TCP connection,
/// TLS handshake, first use of the crypto provider) on a phone whose Wi-Fi
/// is in power save can take well over 500 ms even at home; a missed probe
/// skips background sync, so the budget is generous.
const LAN_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

/// The Moon: local index, sync engine and Ring client.
#[derive(uniffi::Object)]
pub struct Moon {
    db: Mutex<Connection>,
    rt: tokio::runtime::Runtime,
    token: Mutex<Option<String>>,
    route: Mutex<Route>,
    clients: Mutex<Option<Clients>>,
    pending: Mutex<Option<pairing::PendingPairing>>,
    policy: Mutex<OffloadPolicy>,
    /// Serialises sync operations (scan, index, outbox).
    work: Mutex<()>,
    cancel: AtomicBool,
}

impl Moon {
    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().expect("db lock poisoned")
    }

    fn policy(&self) -> OffloadPolicy {
        *self.policy.lock().expect("policy lock")
    }

    fn work(&self) -> MutexGuard<'_, ()> {
        self.work.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn session(&self) -> Result<Session, MoonError> {
        let ring = store::ring(&self.db())?.ok_or(MoonError::NotPaired)?;
        let token = self
            .token
            .lock()
            .expect("token lock")
            .clone()
            .ok_or(MoonError::NotPaired)?;
        let route = *self.route.lock().expect("route lock");
        let mut clients = self.clients.lock().expect("clients lock");
        if clients.as_ref().is_none_or(|c| c.pin != ring.pin) {
            *clients = Some(Clients {
                pin: ring.pin,
                lan: client::pinned(ring.pin)?,
                wan: None,
            });
        }
        let c = clients.as_mut().expect("just set");
        let (base, http) = match route {
            Route::Lan => (ring.lan_url.clone(), c.lan.clone()),
            Route::Wan => {
                let url = ring
                    .wan_url
                    .clone()
                    .ok_or_else(|| MoonError::invalid("this Ring has no wan_url"))?;
                if c.wan.is_none() {
                    c.wan = Some(client::webpki()?);
                }
                (url, c.wan.clone().expect("just set"))
            }
        };
        Ok(Session {
            ring,
            base: base.trim_end_matches('/').to_owned(),
            token,
            http,
        })
    }

    fn query_entries(
        &self,
        cond: &str,
        arg: impl rusqlite::ToSql,
    ) -> Result<Vec<Entry>, MoonError> {
        let db = self.db();
        let cols = store::ENTRY_COLS
            .split(", ")
            .map(|c| format!("e.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = db.prepare(&format!(
            "SELECT {cols}, o.entry_id IS NOT NULL, o.last_error
             FROM entries e LEFT JOIN outbox o ON o.entry_id = e.id
             WHERE {cond} ORDER BY e.ring_path"
        ))?;
        let rows = stmt.query_map([arg], |r| {
            let e = store::entry_from_row(r)?;
            Ok(Entry {
                id: e.id,
                ring_path: e.ring_path,
                local_path: e.local_path,
                remote_file_id: e.remote_file_id,
                size: e.size,
                mtime_ns: e.mtime_ns,
                mime: e.mime,
                blake3: e.blake3.map(|h| h.to_hex()),
                thumb: e.thumb.map(|h| h.to_hex()),
                state: e.state.into(),
                suppressed: e.suppressed,
                upload_pending: r.get(11)?,
                last_error: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn probe(&self, timeout: Duration) -> Result<bool, MoonError> {
        let ring = store::ring(&self.db())?.ok_or(MoonError::NotPaired)?;
        let http = client::pinned(ring.pin)?;
        let url = format!("{}/api/v1/health", ring.lan_url.trim_end_matches('/'));
        let expected = hex_encode(&ring.ring_id);
        Ok(self.rt.block_on(async {
            let resp = tokio::time::timeout(timeout, async {
                client::json::<HealthResponse>(http.get(&url).timeout(timeout)).await
            })
            .await;
            matches!(resp, Ok(Ok(h)) if h.ring_id == expected)
        }))
    }
}

#[uniffi::export]
impl Moon {
    /// Opens (creating and migrating) the Moon database.
    #[uniffi::constructor]
    pub fn open(db_path: String) -> Result<Arc<Self>, MoonError> {
        // reqwest is built without a default provider; harmless if already set.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let conn = db::open(Path::new(&db_path), Role::Moon)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("bring-moon")
            .enable_all()
            .build()?;
        Ok(Arc::new(Self {
            db: Mutex::new(conn),
            rt,
            token: Mutex::default(),
            route: Mutex::new(Route::Lan),
            clients: Mutex::default(),
            pending: Mutex::default(),
            policy: Mutex::default(),
            work: Mutex::default(),
            cancel: AtomicBool::new(false),
        }))
    }

    /// The paired Ring, if any.
    pub fn ring(&self) -> Result<Option<RingInfo>, MoonError> {
        Ok(store::ring(&self.db())?.as_ref().map(RingInfo::from))
    }

    /// Hands over the token from the Android Keystore (after every start).
    pub fn set_device_token(&self, token: String) {
        *self.token.lock().expect("token lock") = Some(token);
    }

    /// Forgets the Ring and its index. Local files are not touched.
    pub fn forget_ring(&self) -> Result<(), MoonError> {
        let _w = self.work();
        store::forget_ring(&mut self.db())?;
        *self.token.lock().expect("token lock") = None;
        *self.clients.lock().expect("clients lock") = None;
        Ok(())
    }

    // --- Pairing ---------------------------------------------------------

    /// Starts pairing with the code from `bring pair` (QR or typed). Returns
    /// the SAS the user must compare with the one on the Ring.
    pub fn pair_start(&self, code: String, device_name: String) -> Result<PairingSas, MoonError> {
        self.pair_start_inner(&code, &device_name)
    }

    /// Waits up to `timeout_secs` for the user to approve on the Ring.
    /// [`MoonError::PairingPending`] means "not yet": call again.
    pub fn pair_finish(&self, timeout_secs: u32) -> Result<PairedRing, MoonError> {
        self.pair_finish_inner(Duration::from_secs(timeout_secs.into()))
    }

    pub fn pair_cancel(&self) {
        *self.pending.lock().expect("pending lock") = None;
    }

    // --- Routing ---------------------------------------------------------

    pub fn route(&self) -> Route {
        *self.route.lock().expect("route lock")
    }

    pub fn set_route(&self, route: Route) -> Result<(), MoonError> {
        if route == Route::Wan
            && store::ring(&self.db())?
                .ok_or(MoonError::NotPaired)?
                .wan_url
                .is_none()
        {
            return Err(MoonError::invalid("this Ring has no wan_url"));
        }
        *self.route.lock().expect("route lock") = route;
        Ok(())
    }

    /// `GET {lan_url}/health` over the pinned client, checking `ring_id`.
    pub fn probe_lan(&self, timeout_ms: u32) -> Result<bool, MoonError> {
        self.probe(Duration::from_millis(timeout_ms.into()))
    }

    /// LAN detection (§4): on unmetered Wi-Fi, try the LAN listener
    /// ([`LAN_PROBE_TIMEOUT`]);
    /// otherwise use the WAN route if there is one. Sets and returns the
    /// route, or `None` when the Ring is unreachable from here.
    pub fn select_route(&self, unmetered_wifi: bool) -> Result<Option<Route>, MoonError> {
        let route = if unmetered_wifi && self.probe(LAN_PROBE_TIMEOUT)? {
            Some(Route::Lan)
        } else if store::ring(&self.db())?
            .ok_or(MoonError::NotPaired)?
            .wan_url
            .is_some()
        {
            Some(Route::Wan)
        } else {
            None
        };
        if let Some(r) = route {
            *self.route.lock().expect("route lock") = r;
        }
        Ok(route)
    }

    // --- Configuration -----------------------------------------------------

    /// Files larger than this are deleted locally after a verified upload.
    pub fn set_offload_threshold(&self, bytes: u64) {
        self.policy.lock().expect("policy lock").threshold_bytes = bytes;
    }

    /// Watches `local_dir`. The default Ring folder is
    /// `<basename>/<device_slug>` (e.g. `DCIM/pixel-8`, §5).
    pub fn add_watched_folder(
        &self,
        local_dir: String,
        ring_folder: Option<String>,
    ) -> Result<WatchedFolder, MoonError> {
        let local_dir = local_dir.trim_end_matches('/').to_owned();
        let dir = Path::new(&local_dir);
        if !dir.is_absolute() {
            return Err(MoonError::invalid("local_dir must be absolute"));
        }
        let ring_folder = match ring_folder {
            Some(f) => f,
            None => {
                let ring = store::ring(&self.db())?.ok_or(MoonError::NotPaired)?;
                let base = dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| MoonError::invalid("local_dir has no name"))?;
                format!("{base}/{}", ring.device_slug)
            }
        };
        let ring_folder = RelPath::new(&ring_folder)
            .map_err(|e| MoonError::invalid(format!("ring folder {ring_folder:?}: {e}")))?;
        if ring_folder.folder() == ".bring" {
            return Err(MoonError::invalid(".bring is reserved"));
        }
        let db = self.db();
        db.execute(
            "INSERT INTO watched_folders (local_dir, ring_folder, enabled) VALUES (?1, ?2, 1)
             ON CONFLICT (local_dir) DO UPDATE SET ring_folder = excluded.ring_folder, enabled = 1",
            params![local_dir, ring_folder.as_str()],
        )?;
        store::watched_folders(&db)?
            .into_iter()
            .find(|f| f.local_dir == local_dir)
            .ok_or_else(|| MoonError::storage("watched folder vanished"))
    }

    pub fn remove_watched_folder(&self, id: i64) -> Result<(), MoonError> {
        self.db()
            .execute("DELETE FROM watched_folders WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn watched_folders(&self) -> Result<Vec<WatchedFolder>, MoonError> {
        store::watched_folders(&self.db())
    }

    // --- Sync ----------------------------------------------------------------

    /// Queues one file outside the watched folders (Share Sheet → `Inbox`).
    /// If `ring_path` is taken, the next free conflict name is used (the
    /// same `name (slug).ext` rule as the Ring). Returns the entry.
    pub fn enqueue_upload(
        &self,
        local_path: String,
        ring_path: String,
    ) -> Result<Entry, MoonError> {
        let _w = self.work();
        let target = RelPath::new(&ring_path)
            .map_err(|e| MoonError::invalid(format!("{ring_path:?}: {e}")))?;
        if target.folder() == ".bring" {
            return Err(MoonError::invalid(".bring is reserved"));
        }
        let stat = sync::local_stat(Path::new(&local_path))?
            .ok_or_else(|| MoonError::invalid(format!("{local_path} is not a file")))?;
        let id = {
            let db = self.db();
            match store::entry_by_local_path(&db, &local_path)? {
                Some(e) => e.id,
                None => {
                    let slug = store::ring(&db)?.ok_or(MoonError::NotPaired)?.device_slug;
                    let mut free = None;
                    for cand in transfer::conflict_candidates(&target, &slug) {
                        if store::entry_by_ring_path(&db, cand.as_str())?.is_none() {
                            free = Some(cand);
                            break;
                        }
                    }
                    let path = free.ok_or_else(|| MoonError::invalid("no free name"))?;
                    store::insert_local(
                        &db,
                        path.as_str(),
                        &local_path,
                        stat.size,
                        stat.mtime_ns,
                        None,
                    )?
                }
            }
        };
        self.entry(id)
    }

    /// One entry by id.
    pub fn entry(&self, id: i64) -> Result<Entry, MoonError> {
        self.query_entries("e.id = ?1", id)?
            .pop()
            .ok_or_else(|| MoonError::invalid(format!("no entry {id}")))
    }

    /// Pulls index changes from the Ring.
    pub fn sync_index(&self) -> Result<IndexReport, MoonError> {
        let _w = self.work();
        self.cancel.store(false, Ordering::Relaxed);
        let s = self.session()?;
        self.rt.block_on(self.sync_index_inner(&s))
    }

    /// Scans the watched folders for new, changed and deleted files.
    pub fn scan_local(&self) -> Result<ScanReport, MoonError> {
        let _w = self.work();
        self.scan_local_inner()
    }

    /// Hashes pending files, uploads them and applies the retention rule.
    pub fn process_outbox(
        &self,
        listener: Option<Arc<dyn TransferListener>>,
    ) -> Result<OutboxReport, MoonError> {
        let _w = self.work();
        self.cancel.store(false, Ordering::Relaxed);
        let s = self.session()?;
        self.rt
            .block_on(self.process_outbox_inner(&s, listener.as_ref()))
    }

    /// Index → scan → outbox → index (to pick up our own uploads).
    pub fn run_sync(
        &self,
        listener: Option<Arc<dyn TransferListener>>,
    ) -> Result<SyncReport, MoonError> {
        let _w = self.work();
        self.cancel.store(false, Ordering::Relaxed);
        let s = self.session()?;
        self.rt.block_on(async {
            let mut index = self.sync_index_inner(&s).await?;
            let scan = self.scan_local_inner()?;
            let outbox = self.process_outbox_inner(&s, listener.as_ref()).await?;
            let after = self.sync_index_inner(&s).await?;
            index.received += after.received;
            index.cursor = after.cursor;
            Ok(SyncReport {
                index,
                scan,
                outbox,
            })
        })
    }

    /// Asks a running sync to stop at the next file or chunk boundary.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    // --- Browsing & streaming ---------------------------------------------

    /// Entries under `prefix` (a Ring folder such as `DCIM`), or all.
    pub fn entries(&self, prefix: Option<String>) -> Result<Vec<Entry>, MoonError> {
        let prefix = prefix.map(|p| p.trim_end_matches('/').to_owned());
        self.query_entries(
            "?1 IS NULL OR e.ring_path = ?1
             OR (e.ring_path >= ?1 || '/' AND e.ring_path < ?1 || '0')",
            prefix,
        )
    }

    /// JPEG thumbnail bytes for an entry's `thumb`.
    pub fn thumbnail(&self, thumb: String) -> Result<Vec<u8>, MoonError> {
        if thumb.len() != 64 || !thumb.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(MoonError::invalid("bad thumbnail id"));
        }
        let s = self.session()?;
        self.rt.block_on(async {
            Ok(s.send(s.get(&format!("/thumbs/{thumb}")))
                .await?
                .bytes()
                .await?
                .to_vec())
        })
    }

    /// `length` bytes of a Ring file from `offset` (HTTP Range).
    pub fn read_range(&self, file_id: i64, offset: u64, length: u64) -> Result<Vec<u8>, MoonError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let s = self.session()?;
        let end = offset
            .checked_add(length - 1)
            .ok_or_else(|| MoonError::invalid("range overflows"))?;
        self.rt.block_on(async {
            let resp = s
                .send(
                    s.get(&format!("/files/{file_id}/content"))
                        .header(RANGE, format!("bytes={offset}-{end}")),
                )
                .await?;
            let status = resp.status();
            let body = resp.bytes().await?;
            match status {
                StatusCode::PARTIAL_CONTENT => Ok(body.to_vec()),
                // Whole file returned (tiny file, range covered all of it).
                StatusCode::OK if offset == 0 => {
                    Ok(body[..body.len().min(length as usize)].to_vec())
                }
                s => Err(MoonError::invalid(format!("unexpected {s} for a range"))),
            }
        })
    }

    /// URL + headers + pin for Media3 (`OkHttpDataSource`).
    pub fn media_source(&self, file_id: i64) -> Result<MediaSource, MoonError> {
        let s = self.session()?;
        let lan = self.route() == Route::Lan;
        Ok(MediaSource {
            url: s.url(&format!("/files/{file_id}/content")),
            authorization: format!("Bearer {}", s.token),
            tls_cert_sha256: lan.then(|| hex_encode(&s.ring.pin)),
        })
    }

    /// A signed URL for players that cannot send headers. Only useful on the
    /// WAN route (external players cannot pin the LAN certificate, §3.1).
    pub fn stream_url(&self, file_id: i64) -> Result<String, MoonError> {
        let s = self.session()?;
        let r: StreamUrlResponse = self
            .rt
            .block_on(s.json(s.post(&format!("/files/{file_id}/stream-url"))))?;
        Ok(format!("{}{}", s.base, r.url))
    }
}
