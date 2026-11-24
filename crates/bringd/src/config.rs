//! `bringd` configuration (`~/.config/bring/bringd.toml`).

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The Ring directory holding plain files. `~/` is expanded.
    pub ring_root: PathBuf,
    /// Index database, thumbnails and TLS material. `~/` is expanded.
    pub data_dir: PathBuf,
    /// LAN HTTPS listener address.
    pub lan_listen: SocketAddr,
    /// Optional plain-HTTP listener for a TLS-terminating reverse proxy.
    /// Bind it to loopback (or a tunnel address), never to the LAN.
    pub proxy_listen: Option<SocketAddr>,
    /// External HTTPS URL behind the reverse proxy, handed to Moons.
    pub wan_url: Option<String>,
    /// LAN IPv4 put into pairing payloads; auto-detected if unset.
    pub advertise_ip: Option<Ipv4Addr>,
    /// Control socket for `bring`; defaults to `$XDG_RUNTIME_DIR/bring/bringd.sock`.
    pub ctl_socket: Option<PathBuf>,
    /// Lifetime of a pairing code.
    pub pairing_ttl_secs: u64,
    /// Logical folders created on start.
    pub folders: Vec<String>,
    /// How long a path must be quiet before the watcher reports it.
    pub debounce_ms: u64,
    /// `ffmpeg` binary for video/HEIC thumbnails (optional at runtime).
    pub ffmpeg: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ring_root: home_dir().join("B-Ring"),
            data_dir: xdg_dir("XDG_DATA_HOME", ".local/share").join("bring"),
            lan_listen: SocketAddr::from(([0, 0, 0, 0], 7443)),
            proxy_listen: None,
            wan_url: None,
            advertise_ip: None,
            ctl_socket: None,
            pairing_ttl_secs: 300,
            folders: vec!["DCIM".into(), "Download".into(), "Inbox".into()],
            debounce_ms: 2000,
            ffmpeg: PathBuf::from("ffmpeg"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl Config {
    /// Default config file location.
    pub fn default_path() -> PathBuf {
        xdg_dir("XDG_CONFIG_HOME", ".config").join("bring/bringd.toml")
    }

    /// Loads `path`; a missing file yields the defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        Self::parse(&text).map_err(|source| ConfigError::Parse {
            path: path.to_owned(),
            source,
        })
    }

    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut cfg: Config = toml::from_str(text)?;
        cfg.ring_root = expand_home(&cfg.ring_root);
        cfg.data_dir = expand_home(&cfg.data_dir);
        Ok(cfg)
    }

    pub fn debounce(&self) -> Duration {
        Duration::from_millis(self.debounce_ms)
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("ring.db")
    }

    pub fn ctl_socket_path(&self) -> PathBuf {
        self.ctl_socket
            .clone()
            .unwrap_or_else(bring_core::ctl::default_socket_path)
    }

    pub fn tls_dir(&self) -> PathBuf {
        self.data_dir.join("tls")
    }

    pub fn thumbs_dir(&self) -> PathBuf {
        self.data_dir.join("thumbs")
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home_dir().join(fallback),
    }
}

fn expand_home(p: &Path) -> PathBuf {
    match p.strip_prefix("~") {
        Ok(rest) => home_dir().join(rest),
        Err(_) => p.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_default() {
        let c = Config::parse("").unwrap();
        assert!(c.ring_root.ends_with("B-Ring"));
        assert_eq!(c.folders, ["DCIM", "Download", "Inbox"]);
    }

    #[test]
    fn expands_home_and_rejects_unknown_keys() {
        let c = Config::parse("ring_root = \"~/Files\"\ndebounce_ms = 10").unwrap();
        assert_eq!(c.ring_root, home_dir().join("Files"));
        assert_eq!(c.debounce(), Duration::from_millis(10));
        assert!(Config::parse("bogus = 1").is_err());
    }
}
