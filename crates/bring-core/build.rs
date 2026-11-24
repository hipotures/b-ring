//! Derives the B-Ring version from git: `<major>.<minor>.<commits>+<hash>`.
//!
//! The patch number is the commit count of HEAD, so every commit gets a new,
//! increasing version without editing Cargo.toml (the Android app uses the
//! same scheme in `android/app/build.gradle.kts`). Outside a git checkout
//! the Cargo version is used as is.

use std::path::Path;
use std::process::Command;

fn git(dir: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let pkg = std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION");

    let version = match (
        git(&dir, &["rev-list", "--count", "HEAD"]),
        git(&dir, &["rev-parse", "--short=7", "HEAD"]),
    ) {
        (Some(count), Some(hash)) => {
            let base: Vec<&str> = pkg.split('.').take(2).collect();
            format!("{}.{count}+{hash}", base.join("."))
        }
        _ => pkg,
    };
    println!("cargo:rustc-env=BRING_VERSION={version}");

    // Re-run when HEAD moves: a checkout, a commit, or packed refs.
    if let Some(git_dir) = git(&dir, &["rev-parse", "--absolute-git-dir"]) {
        let git_dir = Path::new(&git_dir);
        let mut watch = vec![git_dir.join("HEAD"), git_dir.join("packed-refs")];
        if let Some(head_ref) = git(&dir, &["symbolic-ref", "-q", "HEAD"]) {
            watch.push(git_dir.join(head_ref));
        }
        // A missing path would make Cargo re-run the script on every build.
        for path in watch.iter().filter(|p| p.exists()) {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
