# B-Ring

Self-hosted hybrid file space: Android devices (**Moons**) orbit a Linux
workstation (**Ring**) that keeps plain files on disk as the single source of
truth.

- Design: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
- Workspace: `crates/bring-core`, `crates/bringd`, `crates/bring-cli` (`bring`),
  `crates/bring-uniffi`; Android client in `android/` (later).

```sh
cargo test --workspace
```
