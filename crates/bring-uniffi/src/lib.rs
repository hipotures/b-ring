//! UniFFI bindings: the Moon's sync engine, called from Kotlin.
//!
//! Proc-macro mode (no UDL). The API is **blocking**: Kotlin calls it from
//! `Dispatchers.IO` or a `Worker`; internally one Tokio runtime drives
//! `reqwest`. Long operations check a cancel flag ([`Moon::cancel`]) between
//! files and chunks.
//!
//! Device tokens are never stored here: Kotlin keeps them in the Android
//! Keystore and hands them over with [`Moon::set_ring_token`] after start.

uniffi::setup_scaffolding!();

mod channels;
mod client;
mod error;
mod pairing;
mod store;
mod sync;

use std::collections::HashMap;
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

pub use channels::SoundReceiver;
pub use error::MoonError;
pub use pairing::{PairedRing, PairingSas};
pub use store::WatchedFolder;
pub use sync::{FreeReport, IndexReport, OutboxReport, ScanReport, SyncReport, VerifyReport};

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

/// One paired Ring.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RingInfo {
    pub id: i64,
    pub ring_id: String,
    /// User-defined custom name (falls back to the LAN host if unset).
    pub name: String,
    pub lan_url: String,
    pub wan_url: Option<String>,
    /// Hex SHA-256 pin of the LAN certificate (also for Media3's OkHttp).
    pub tls_cert_sha256: String,
    pub device_slug: String,
    /// Hex-encoded 16-byte random device UUID assigned to this Moon for this Ring.
    pub device_id: String,
    /// Per-Ring offload threshold in bytes (`None` = default 100 MiB, `u64::MAX` = Never).
    pub offload_threshold_bytes: Option<u64>,
    /// Cumulative files transferred to this Ring (aggregate counter, survives pruning).
    pub transferred_files: u64,
    /// Cumulative bytes transferred to this Ring (aggregate counter, survives pruning).
    pub transferred_bytes: u64,
}

impl From<&store::RingRow> for RingInfo {
    fn from(r: &store::RingRow) -> Self {
        Self {
            id: r.id,
            ring_id: r.ring_id_hex(),
            name: r.display_name(),
            lan_url: r.lan_url.clone(),
            wan_url: r.wan_url.clone(),
            tls_cert_sha256: hex_encode(&r.pin),
            device_slug: r.device_slug.clone(),
            device_id: hex_encode(&r.device_uuid),
            offload_threshold_bytes: r.offload_threshold_bytes,
            transferred_files: r.transferred_files,
            transferred_bytes: r.transferred_bytes,
        }
    }
}

/// One file as the browser shows it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Entry {
    pub id: i64,
    pub ring_id: String,
    pub ring_path: String,
    pub local_path: Option<String>,
    pub remote_file_id: Option<i64>,
    pub size: u64,
    pub mtime_ns: i64,
    pub mime: Option<String>,
    pub blake3: Option<String>,
    /// Pass to [`Moon::thumbnail_for`].
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

    pub(crate) fn patch(&self, path: &str) -> RequestBuilder {
        self.http.patch(self.url(path)).bearer_auth(&self.token)
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

/// The Moon: local index, sync engine and multi-Ring client.
#[derive(uniffi::Object)]
pub struct Moon {
    db: Mutex<Connection>,
    rt: tokio::runtime::Runtime,
    /// Device tokens keyed by hex `ring_id`.
    tokens: Mutex<HashMap<String, String>>,
    /// Active route keyed by hex `ring_id`.
    routes: Mutex<HashMap<String, Route>>,
    /// HTTP clients keyed by hex `ring_id`.
    clients: Mutex<HashMap<String, Clients>>,
    pending: Mutex<Option<pairing::PendingPairing>>,
    /// Fallback policy when a Ring has no per-Ring threshold saved.
    policy: Mutex<OffloadPolicy>,
    /// Serialises sync operations (scan, index, outbox).
    work: Mutex<()>,
    cancel: AtomicBool,
}

impl Moon {
    pub(crate) fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().expect("db lock poisoned")
    }

    pub(crate) fn policy_for(&self, ring: &store::RingRow) -> OffloadPolicy {
        match ring.offload_threshold_bytes {
            Some(threshold_bytes) => OffloadPolicy { threshold_bytes },
            None => *self.policy.lock().expect("policy lock"),
        }
    }

    fn work(&self) -> MutexGuard<'_, ()> {
        self.work.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn default_ring_row(&self) -> Result<store::RingRow, MoonError> {
        store::ring(&self.db())?.ok_or(MoonError::NotPaired)
    }

    fn ring_row(&self, ring_id: &str) -> Result<store::RingRow, MoonError> {
        store::ring_by_hex(&self.db(), ring_id)?.ok_or(MoonError::NotPaired)
    }

    fn session_for(&self, ring_id: &str) -> Result<Session, MoonError> {
        let ring = self.ring_row(ring_id)?;
        let key = ring.ring_id_hex();
        let token = self
            .tokens
            .lock()
            .expect("tokens lock")
            .get(&key)
            .cloned()
            .ok_or(MoonError::NotPaired)?;
        let route = self
            .routes
            .lock()
            .expect("routes lock")
            .get(&key)
            .copied()
            .unwrap_or(Route::Lan);
        let mut clients = self.clients.lock().expect("clients lock");
        let rebuild = clients.get(&key).is_none_or(|c| c.pin != ring.pin);
        if rebuild {
            clients.insert(
                key.clone(),
                Clients {
                    pin: ring.pin,
                    lan: client::pinned(ring.pin)?,
                    wan: None,
                },
            );
        }
        let c = clients.get_mut(&key).expect("just inserted");
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
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Entry>, MoonError> {
        let db = self.db();
        let cols = store::ENTRY_COLS
            .split(", ")
            .map(|c| format!("e.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = db.prepare(&format!(
            "SELECT {cols}, r.ring_id, o.entry_id IS NOT NULL, o.last_error
             FROM entries e
             JOIN rings r ON r.id = e.ring_id
             LEFT JOIN outbox o ON o.entry_id = e.id
             WHERE {cond} ORDER BY e.ring_path"
        ))?;
        let rows = stmt.query_map(args, |r| {
            let e = store::entry_from_row(r)?;
            let rid: [u8; 16] = r.get(13)?;
            Ok(Entry {
                id: e.id,
                ring_id: hex_encode(&rid),
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
                upload_pending: r.get(14)?,
                last_error: r.get(15)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn probe_ring(&self, ring: &store::RingRow, timeout: Duration) -> Result<bool, MoonError> {
        let http = client::pinned(ring.pin)?;
        let url = format!("{}/api/v1/health", ring.lan_url.trim_end_matches('/'));
        let expected = ring.ring_id_hex();
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
            tokens: Mutex::default(),
            routes: Mutex::default(),
            clients: Mutex::default(),
            pending: Mutex::default(),
            policy: Mutex::default(),
            work: Mutex::default(),
            cancel: AtomicBool::new(false),
        }))
    }

    /// All paired Rings, oldest first.
    pub fn rings(&self) -> Result<Vec<RingInfo>, MoonError> {
        Ok(store::rings(&self.db())?
            .iter()
            .map(RingInfo::from)
            .collect())
    }

    /// The first paired Ring, if any (convenience for single-Ring callers).
    pub fn ring(&self) -> Result<Option<RingInfo>, MoonError> {
        Ok(store::ring(&self.db())?.as_ref().map(RingInfo::from))
    }

    /// Hands over the device token for `ring_id` from the Android Keystore.
    pub fn set_ring_token(&self, ring_id: String, token: String) {
        self.tokens
            .lock()
            .expect("tokens lock")
            .insert(ring_id, token);
    }

    /// Single-Ring convenience: sets the token on the first paired Ring.
    pub fn set_device_token(&self, token: String) {
        if let Ok(Some(r)) = store::ring(&self.db()) {
            self.set_ring_token(r.ring_id_hex(), token);
        }
    }

    /// Gives a paired Ring a custom display name (e.g. "Videos", "Screenshots").
    pub fn rename_ring(&self, ring_id: String, name: String) -> Result<RingInfo, MoonError> {
        let row = self.ring_row(&ring_id)?;
        let updated = store::rename_ring(&self.db(), row.id, &name)?;
        Ok(RingInfo::from(&updated))
    }

    /// Renames this device on the Ring (`PATCH /api/v1/device`), stores the
    /// Ring-assigned slug locally, and rewrites `<old_slug>/` prefixes in
    /// watched folders and entries to match the renamed directory on the Ring.
    pub fn rename_device_for(&self, ring_id: String, name: String) -> Result<RingInfo, MoonError> {
        let s = self.session_for(&ring_id)?;
        let req = bring_core::api::RenameDeviceRequest { name };
        let resp: bring_core::api::DeviceResponse =
            self.rt.block_on(s.json(s.patch("/device").json(&req)))?;
        let updated =
            store::set_device_slug(&mut self.db(), s.ring.id, &s.ring.device_slug, &resp.slug)?;
        Ok(RingInfo::from(&updated))
    }

    /// Forgets one paired Ring and its index. Local files are not touched.
    pub fn forget_ring_by_id(&self, ring_id: String) -> Result<(), MoonError> {
        let _w = self.work();
        let Some(row) = store::ring_by_hex(&self.db(), &ring_id)? else {
            return Ok(());
        };
        store::forget_ring_by_db_id(&mut self.db(), row.id)?;
        self.tokens.lock().expect("tokens lock").remove(&ring_id);
        self.routes.lock().expect("routes lock").remove(&ring_id);
        self.clients.lock().expect("clients lock").remove(&ring_id);
        Ok(())
    }

    /// Forgets all paired Rings and their indices. Local files are not touched.
    pub fn forget_ring(&self) -> Result<(), MoonError> {
        let _w = self.work();
        store::forget_ring(&mut self.db())?;
        self.tokens.lock().expect("tokens lock").clear();
        self.routes.lock().expect("routes lock").clear();
        self.clients.lock().expect("clients lock").clear();
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

    pub fn route_for(&self, ring_id: String) -> Route {
        self.routes
            .lock()
            .expect("routes lock")
            .get(&ring_id)
            .copied()
            .unwrap_or(Route::Lan)
    }

    pub fn route(&self) -> Route {
        self.default_ring_row()
            .map(|r| self.route_for(r.ring_id_hex()))
            .unwrap_or(Route::Lan)
    }

    pub fn set_route_for(&self, ring_id: String, route: Route) -> Result<(), MoonError> {
        let ring = self.ring_row(&ring_id)?;
        if route == Route::Wan && ring.wan_url.is_none() {
            return Err(MoonError::invalid("this Ring has no wan_url"));
        }
        self.routes
            .lock()
            .expect("routes lock")
            .insert(ring.ring_id_hex(), route);
        Ok(())
    }

    pub fn set_route(&self, route: Route) -> Result<(), MoonError> {
        let ring = self.default_ring_row()?;
        self.set_route_for(ring.ring_id_hex(), route)
    }

    /// `GET {lan_url}/health` over the pinned client, checking `ring_id`.
    pub fn probe_lan_for(&self, ring_id: String, timeout_ms: u32) -> Result<bool, MoonError> {
        let ring = self.ring_row(&ring_id)?;
        self.probe_ring(&ring, Duration::from_millis(timeout_ms.into()))
    }

    pub fn probe_lan(&self, timeout_ms: u32) -> Result<bool, MoonError> {
        let ring = self.default_ring_row()?;
        self.probe_ring(&ring, Duration::from_millis(timeout_ms.into()))
    }

    /// LAN detection (§4) for one Ring.
    pub fn select_route_for(
        &self,
        ring_id: String,
        unmetered_wifi: bool,
    ) -> Result<Option<Route>, MoonError> {
        let ring = self.ring_row(&ring_id)?;
        let route = if unmetered_wifi && self.probe_ring(&ring, LAN_PROBE_TIMEOUT)? {
            Some(Route::Lan)
        } else if ring.wan_url.is_some() {
            Some(Route::Wan)
        } else {
            None
        };
        if let Some(r) = route {
            self.routes
                .lock()
                .expect("routes lock")
                .insert(ring.ring_id_hex(), r);
        }
        Ok(route)
    }

    pub fn select_route(&self, unmetered_wifi: bool) -> Result<Option<Route>, MoonError> {
        let ring = self.default_ring_row()?;
        self.select_route_for(ring.ring_id_hex(), unmetered_wifi)
    }

    // --- Configuration -----------------------------------------------------

    /// Fallback offload threshold when a Ring has no per-Ring override.
    pub fn set_offload_threshold(&self, bytes: u64) {
        self.policy.lock().expect("policy lock").threshold_bytes = bytes;
    }

    /// Per-Ring offload threshold (`u64::MAX` = never delete local copies).
    pub fn set_ring_offload_threshold(&self, ring_id: String, bytes: u64) -> Result<(), MoonError> {
        let ring = self.ring_row(&ring_id)?;
        store::set_ring_offload(&self.db(), ring.id, bytes)
    }

    /// Watches `local_dir` for `ring_id`. Multiple Rings may watch the same
    /// `local_dir`. The default Ring folder is `<device_slug>/<relative_path>`
    /// (stripping standard Android shared-storage roots such as
    /// `/storage/emulated/0/`, or falling back to `<device_slug>/<basename>`).
    pub fn add_watched_folder_for(
        &self,
        ring_id: String,
        local_dir: String,
        ring_folder: Option<String>,
    ) -> Result<WatchedFolder, MoonError> {
        let ring = self.ring_row(&ring_id)?;
        let local_dir = local_dir.trim_end_matches('/').to_owned();
        let dir = Path::new(&local_dir);
        if !dir.is_absolute() {
            return Err(MoonError::invalid("local_dir must be absolute"));
        }
        let ring_folder = match ring_folder {
            Some(f) => f,
            None => default_ring_folder(&ring.device_slug, &local_dir)?,
        };
        let ring_folder = RelPath::new(&ring_folder)
            .map_err(|e| MoonError::invalid(format!("ring folder {ring_folder:?}: {e}")))?;
        if ring_folder.folder() == ".bring" {
            return Err(MoonError::invalid(".bring is reserved"));
        }
        let db = self.db();
        db.execute(
            "INSERT INTO watched_folders (ring_id, local_dir, ring_folder, enabled)
             VALUES (?1, ?2, ?3, 1)
             ON CONFLICT (ring_id, local_dir) DO UPDATE SET
               ring_folder = excluded.ring_folder, enabled = 1",
            params![ring.id, local_dir, ring_folder.as_str()],
        )?;
        store::watched_folders(&db, ring.id)?
            .into_iter()
            .find(|f| f.local_dir == local_dir)
            .ok_or_else(|| MoonError::storage("watched folder vanished"))
    }

    pub fn add_watched_folder(
        &self,
        local_dir: String,
        ring_folder: Option<String>,
    ) -> Result<WatchedFolder, MoonError> {
        let ring = self.default_ring_row()?;
        self.add_watched_folder_for(ring.ring_id_hex(), local_dir, ring_folder)
    }

    pub fn remove_watched_folder(&self, id: i64) -> Result<(), MoonError> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let removed: Option<(i64, String)> = {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT ring_id, local_dir FROM watched_folders WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        };
        tx.execute("DELETE FROM watched_folders WHERE id = ?1", [id])?;
        if let Some((ring_db_id, local_dir)) = removed {
            let remaining: Vec<String> = store::watched_folders(&tx, ring_db_id)?
                .into_iter()
                .filter(|f| f.enabled)
                .map(|f| f.local_dir)
                .collect();
            let candidates: Vec<(i64, String)> = {
                let mut stmt = tx.prepare(
                    "SELECT id, local_path FROM entries
                     WHERE ring_id = ?1 AND state = 'local-only' AND local_path IS NOT NULL",
                )?;
                stmt.query_map([ring_db_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?
            };
            let removed_root = Path::new(&local_dir);
            for (entry_id, local_path) in candidates {
                if sync::is_under(&local_path, removed_root)
                    && !remaining
                        .iter()
                        .any(|dir| sync::is_under(&local_path, Path::new(dir)))
                {
                    // Auto-prune local-only/held/queued entries from the
                    // unwatched folder without touching local files on disk.
                    tx.execute("DELETE FROM entries WHERE id = ?1", [entry_id])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn watched_folders_for(&self, ring_id: String) -> Result<Vec<WatchedFolder>, MoonError> {
        let Some(ring) = store::ring_by_hex(&self.db(), &ring_id)? else {
            return Ok(Vec::new());
        };
        store::watched_folders(&self.db(), ring.id)
    }

    pub fn watched_folders(&self) -> Result<Vec<WatchedFolder>, MoonError> {
        let Some(ring) = store::ring(&self.db())? else {
            return Ok(Vec::new());
        };
        store::watched_folders(&self.db(), ring.id)
    }

    // --- Sync ----------------------------------------------------------------

    fn enqueue_upload_inner(
        &self,
        ring_id: &str,
        local_path: &str,
        ring_path: &str,
        offload_after_upload: bool,
    ) -> Result<Entry, MoonError> {
        let _w = self.work();
        let ring = self.ring_row(ring_id)?;
        let target = RelPath::new(ring_path)
            .map_err(|e| MoonError::invalid(format!("{ring_path:?}: {e}")))?;
        if target.folder() == ".bring" {
            return Err(MoonError::invalid(".bring is reserved"));
        }
        let stat = sync::local_stat(Path::new(local_path))?
            .ok_or_else(|| MoonError::invalid(format!("{local_path} is not a file")))?;
        let id = {
            let db = self.db();
            match store::entry_by_local_path(&db, ring.id, local_path)? {
                Some(e) => e.id,
                None => {
                    let mut free = None;
                    for cand in transfer::conflict_candidates(&target, &ring.device_slug) {
                        if store::entry_by_ring_path(&db, ring.id, cand.as_str())?.is_none() {
                            free = Some(cand);
                            break;
                        }
                    }
                    let path = free.ok_or_else(|| MoonError::invalid("no free name"))?;
                    let id = store::insert_local(
                        &db,
                        ring.id,
                        path.as_str(),
                        local_path,
                        stat.size,
                        stat.mtime_ns,
                        None,
                    )?;
                    if offload_after_upload {
                        db.execute(
                            "UPDATE entries SET offload_after_upload = 1 WHERE id = ?1",
                            [id],
                        )?;
                    }
                    id
                }
            }
        };
        self.entry(id)
    }

    /// Queues one file outside watched folders for `ring_id`.
    pub fn enqueue_upload_for(
        &self,
        ring_id: String,
        local_path: String,
        ring_path: String,
    ) -> Result<Entry, MoonError> {
        self.enqueue_upload_inner(&ring_id, &local_path, &ring_path, false)
    }

    /// Queues one file outside watched folders (e.g. Share Sheet) for `ring_id`
    /// and deletes its staged local copy right after a verified ACK so it stays
    /// `remote-only` on the Ring regardless of file size.
    pub fn enqueue_upload_remote_for(
        &self,
        ring_id: String,
        local_path: String,
        ring_path: String,
    ) -> Result<Entry, MoonError> {
        self.enqueue_upload_inner(&ring_id, &local_path, &ring_path, true)
    }

    pub fn enqueue_upload(
        &self,
        local_path: String,
        ring_path: String,
    ) -> Result<Entry, MoonError> {
        let ring = self.default_ring_row()?;
        self.enqueue_upload_for(ring.ring_id_hex(), local_path, ring_path)
    }

    /// One entry by id.
    pub fn entry(&self, id: i64) -> Result<Entry, MoonError> {
        self.query_entries("e.id = ?1", &[&id])?
            .pop()
            .ok_or_else(|| MoonError::invalid(format!("no entry {id}")))
    }

    /// Pulls index changes from `ring_id`.
    pub fn sync_index_for(&self, ring_id: String) -> Result<IndexReport, MoonError> {
        let _w = self.work();
        self.cancel.store(false, Ordering::Relaxed);
        let s = self.session_for(&ring_id)?;
        self.rt.block_on(self.sync_index_inner(&s))
    }

    pub fn sync_index(&self) -> Result<IndexReport, MoonError> {
        let ring = self.default_ring_row()?;
        self.sync_index_for(ring.ring_id_hex())
    }

    /// Scans watched folders for `ring_id`.
    pub fn scan_local_for(&self, ring_id: String) -> Result<ScanReport, MoonError> {
        let _w = self.work();
        let ring = self.ring_row(&ring_id)?;
        self.scan_local_inner(ring.id)
    }

    pub fn scan_local(&self) -> Result<ScanReport, MoonError> {
        let ring = self.default_ring_row()?;
        self.scan_local_for(ring.ring_id_hex())
    }

    /// Hashes pending files, uploads them and applies the retention rule for `ring_id`.
    pub fn process_outbox_for(
        &self,
        ring_id: String,
        listener: Option<Arc<dyn TransferListener>>,
    ) -> Result<OutboxReport, MoonError> {
        let _w = self.work();
        self.cancel.store(false, Ordering::Relaxed);
        let s = self.session_for(&ring_id)?;
        self.rt
            .block_on(self.process_outbox_inner(&s, listener.as_ref()))
    }

    pub fn process_outbox(
        &self,
        listener: Option<Arc<dyn TransferListener>>,
    ) -> Result<OutboxReport, MoonError> {
        let ring = self.default_ring_row()?;
        self.process_outbox_for(ring.ring_id_hex(), listener)
    }

    /// Index → scan → outbox → index (to pick up our own uploads and thumbs) for `ring_id`.
    pub fn run_sync_for(
        &self,
        ring_id: String,
        listener: Option<Arc<dyn TransferListener>>,
    ) -> Result<SyncReport, MoonError> {
        let _w = self.work();
        self.cancel.store(false, Ordering::Relaxed);
        let s = self.session_for(&ring_id)?;
        self.rt.block_on(async {
            let mut index = self.sync_index_inner(&s).await?;
            let scan = self.scan_local_inner(s.ring.id)?;
            let (outbox, uploaded_ids) = self.process_outbox_tracked(&s, listener.as_ref()).await?;
            let after = self.sync_index_inner(&s).await?;
            index.received += after.received;
            index.cursor = after.cursor;
            self.wait_for_thumbs(&s, &uploaded_ids, &mut index).await;
            Ok(SyncReport {
                index,
                scan,
                outbox,
            })
        })
    }

    pub fn run_sync(
        &self,
        listener: Option<Arc<dyn TransferListener>>,
    ) -> Result<SyncReport, MoonError> {
        let ring = self.default_ring_row()?;
        self.run_sync_for(ring.ring_id_hex(), listener)
    }

    /// Full check & repair for `ring_id`.
    pub fn verify_sync_for(&self, ring_id: String) -> Result<VerifyReport, MoonError> {
        let _w = self.work();
        self.cancel.store(false, Ordering::Relaxed);
        let s = self.session_for(&ring_id)?;
        self.rt.block_on(self.verify_sync_inner(&s))
    }

    pub fn verify_sync(&self) -> Result<VerifyReport, MoonError> {
        let ring = self.default_ring_row()?;
        self.verify_sync_for(ring.ring_id_hex())
    }

    /// Deletes the local copies of the given `synced` entries (the files
    /// stay on the Ring; the entries become `remote-only`). Entries that are
    /// not synced, wait for an upload, changed locally, or are still needed by
    /// another Ring are skipped. Does not wait for a running sync.
    pub fn free_local(&self, ids: Vec<i64>) -> Result<FreeReport, MoonError> {
        sync::free_local(&mut self.db(), &ids)
    }

    /// Asks a running sync to stop at the next file or chunk boundary.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    // --- Browsing & streaming ---------------------------------------------

    /// Entries of `ring_id` under `prefix` (a Ring folder such as `DCIM`), or all.
    pub fn entries_for(
        &self,
        ring_id: String,
        prefix: Option<String>,
    ) -> Result<Vec<Entry>, MoonError> {
        let Some(ring) = store::ring_by_hex(&self.db(), &ring_id)? else {
            return Ok(Vec::new());
        };
        let prefix = prefix.map(|p| p.trim_end_matches('/').to_owned());
        self.query_entries(
            "e.ring_id = ?1 AND (?2 IS NULL OR e.ring_path = ?2
             OR (e.ring_path >= ?2 || '/' AND e.ring_path < ?2 || '0'))",
            &[&ring.id, &prefix],
        )
    }

    /// Entries of the first paired Ring (single-Ring convenience).
    pub fn entries(&self, prefix: Option<String>) -> Result<Vec<Entry>, MoonError> {
        let Some(ring) = store::ring(&self.db())? else {
            return Ok(Vec::new());
        };
        self.entries_for(ring.ring_id_hex(), prefix)
    }

    /// JPEG thumbnail bytes for an entry's `thumb` from `ring_id`.
    pub fn thumbnail_for(&self, ring_id: String, thumb: String) -> Result<Vec<u8>, MoonError> {
        if thumb.len() != 64 || !thumb.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(MoonError::invalid("bad thumbnail id"));
        }
        let s = self.session_for(&ring_id)?;
        self.rt.block_on(async {
            Ok(s.send(s.get(&format!("/thumbs/{thumb}")))
                .await?
                .bytes()
                .await?
                .to_vec())
        })
    }

    pub fn thumbnail(&self, thumb: String) -> Result<Vec<u8>, MoonError> {
        if thumb.len() != 64 || !thumb.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(MoonError::invalid("bad thumbnail id"));
        }
        let ring = self.default_ring_row()?;
        self.thumbnail_for(ring.ring_id_hex(), thumb)
    }

    /// `length` bytes of a Ring file from `offset` (HTTP Range) on `ring_id`.
    pub fn read_range_for(
        &self,
        ring_id: String,
        file_id: i64,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, MoonError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let s = self.session_for(&ring_id)?;
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

    pub fn read_range(&self, file_id: i64, offset: u64, length: u64) -> Result<Vec<u8>, MoonError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let ring = self.default_ring_row()?;
        self.read_range_for(ring.ring_id_hex(), file_id, offset, length)
    }

    /// URL + headers + pin for Media3 (`OkHttpDataSource`) on `ring_id`.
    pub fn media_source_for(
        &self,
        ring_id: String,
        file_id: i64,
    ) -> Result<MediaSource, MoonError> {
        let s = self.session_for(&ring_id)?;
        let lan = self.route_for(s.ring.ring_id_hex()) == Route::Lan;
        Ok(MediaSource {
            url: s.url(&format!("/files/{file_id}/content")),
            authorization: format!("Bearer {}", s.token),
            tls_cert_sha256: lan.then(|| hex_encode(&s.ring.pin)),
        })
    }

    pub fn media_source(&self, file_id: i64) -> Result<MediaSource, MoonError> {
        let ring = self.default_ring_row()?;
        self.media_source_for(ring.ring_id_hex(), file_id)
    }

    /// A signed URL for players that cannot send headers (WAN route).
    pub fn stream_url_for(&self, ring_id: String, file_id: i64) -> Result<String, MoonError> {
        let s = self.session_for(&ring_id)?;
        let r: StreamUrlResponse = self
            .rt
            .block_on(s.json(s.post(&format!("/files/{file_id}/stream-url"))))?;
        Ok(format!("{}{}", s.base, r.url))
    }

    pub fn stream_url(&self, file_id: i64) -> Result<String, MoonError> {
        let ring = self.default_ring_row()?;
        self.stream_url_for(ring.ring_id_hex(), file_id)
    }
}

/// Strips standard Android shared-storage prefixes so watched paths mirror the
/// phone's relative directory tree under `<device_slug>/...`.
fn strip_android_storage_root(local_dir: &str) -> Option<&str> {
    if let Some(rest) = local_dir.strip_prefix("/storage/emulated/") {
        let (user, tail) = rest.split_once('/')?;
        if !user.is_empty() && user.bytes().all(|b| b.is_ascii_digit()) {
            return Some(tail.trim_matches('/'));
        }
    }
    for prefix in ["/storage/self/primary/", "/sdcard/"] {
        if let Some(tail) = local_dir.strip_prefix(prefix) {
            return Some(tail.trim_matches('/'));
        }
    }
    None
}

fn default_ring_folder(device_slug: &str, local_dir: &str) -> Result<String, MoonError> {
    let suffix = match strip_android_storage_root(local_dir) {
        Some(rel) if !rel.is_empty() => rel,
        _ => Path::new(local_dir)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| MoonError::invalid("local_dir has no name"))?,
    };
    Ok(format!("{device_slug}/{suffix}"))
}

#[cfg(test)]
mod default_folder_tests {
    use super::*;

    #[test]
    fn device_first_default_ring_folders() {
        assert_eq!(
            default_ring_folder("sm-s918b", "/storage/emulated/0/Download").unwrap(),
            "sm-s918b/Download"
        );
        assert_eq!(
            default_ring_folder(
                "sm-s918b",
                "/storage/emulated/0/DCIM/BackgroundVideoRecorder/Videos"
            )
            .unwrap(),
            "sm-s918b/DCIM/BackgroundVideoRecorder/Videos"
        );
        assert_eq!(
            default_ring_folder("sm-g781b", "/sdcard/Download/0test").unwrap(),
            "sm-g781b/Download/0test"
        );
        assert_eq!(
            default_ring_folder("fake-moon", "/tmp/phone/DCIM").unwrap(),
            "fake-moon/DCIM"
        );
    }
}
