# B-Ring Agent Instructions

## Local device & server config (`.env`)
- Read `.env` (gitignored; template in `.env.example`) for local paths and addresses (`ADB_BIN`, `ADB_PHONE_IP`, `ADB_PHONE_SERIAL`, `RING_DB`, etc.) instead of hardcoding or committing private IPs/serials.
- Never commit `.env` or put personal IPs, device serials, or Wi-Fi names into tracked files.

## Android ADB & APK workflow
- See `docs/ANDROID_DEV.md` for full Wi-Fi ADB, logcat, and `moon.db` inspection commands.
- Prefer **Wi-Fi ADB**: if the phone is not listed in `$ADB_BIN devices`, discover its current Wireless Debugging port via `$ADB_BIN mdns services` (matching `ADB_PHONE_IP` or `ADB_PHONE_SERIAL`) and connect with `$ADB_BIN connect <ip>:<port>`.
- **Never run `adb kill-server`** unless explicitly asked (can disrupt USB passthrough).
- Every build produces two variants, installed side by side: **release** (`io.github.hipotures.bring`, "B-Ring", minified, keeps the real data) and **debug** (`io.github.hipotures.bring.debug`, "B-Ring dbg", own data). Use `dist/install-android.sh [adb-target]` (add `--no-rust` when Rust crates did not change); it builds both and installs both. Compare speed between them, not against a single debug build.
- Before installing (the script checks this), make sure no upload is active (`dumpsys notification --noredact` has no `NotificationRecord` for `io.github.hipotures.bring`, and `RING_DB` has no `open`/`committing` uploads).
- Run Gradle with `--no-daemon --max-workers=1` (and add `-x cargoAndroid -x cargoHost` when Rust crates did not change) to keep RAM usage low on zero-swap machines.

## Ring daemon updates & versioning
- Use `./dist/update-ring.sh` to rebuild/reinstall `bringd` + `bring` and restart `bringd.service` safely (refuses if uploads are active unless `--force`).
- `.githooks/pre-commit` auto-bumps `[workspace.package].version` in `Cargo.toml` and updates `Cargo.lock` offline on every commit (`git config core.hooksPath .githooks`).
