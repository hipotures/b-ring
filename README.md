# B-Ring

Self-hosted hybrid file space: Android devices (**Moons**) orbit a Linux
workstation (**Ring**) that keeps plain files on disk as the single source of
truth.

- Design: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- Workspace: `crates/bring-core`, `crates/bringd`, `crates/bring-cli` (`bring`),
  `crates/bring-uniffi`; Android client in `android/` (later).

## Running the Ring

```sh
cargo install --path crates/bringd --locked
cargo install --path crates/bring-cli --locked
install -Dm644 dist/systemd/bringd.service ~/.config/systemd/user/bringd.service
systemctl --user daemon-reload
systemctl --user enable --now bringd
loginctl enable-linger "$USER"      # optional: keep running without a login

bring status
bring pair                          # QR code + code for manual entry
bring devices
bring revoke <slug|id>
```

`bringd` works without a config file. To change defaults, create
`~/.config/bring/bringd.toml`:

```toml
ring_root   = "~/B-Ring"                 # plain files live here
data_dir    = "~/.local/share/bring"     # index, thumbnails, TLS identity
lan_listen  = "0.0.0.0:7443"             # HTTPS (self-signed, pinned by Moons)
# proxy_listen = "127.0.0.1:7480"        # plain HTTP for a TLS reverse proxy
# wan_url      = "https://files.example.org"
# advertise_ip = "192.168.1.10"          # LAN IP put into pairing codes
folders     = ["DCIM", "Download", "Inbox"]
debounce_ms = 2000                       # quiet time before a change is indexed
ffmpeg      = "ffmpeg"                   # video/HEIC thumbnails (optional)
notifications = true                     # desktop notification on uploads
pairing_ttl_secs = 300
```

## Development

```sh
cargo test --workspace
# Manual smoke test of the real binary (shows a desktop notification):
cargo test -p bringd --test smoke -- --ignored --nocapture
```
