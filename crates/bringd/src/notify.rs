//! Desktop notifications for incoming uploads (`notify-rust`, D-Bus).
//!
//! Uploads arriving close together are coalesced into one notification, so a
//! phone syncing 500 photos produces one bubble, not 500.

use std::time::Duration;

use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Uploads within this window are shown together.
const COALESCE: Duration = Duration::from_secs(3);
const MAX_LISTED: usize = 5;

/// One committed upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadEvent {
    pub path: String,
    pub device: String,
    pub size: u64,
}

/// Handle for reporting uploads; a disabled notifier drops everything.
#[derive(Clone, Default)]
pub struct Notifier {
    tx: Option<mpsc::UnboundedSender<UploadEvent>>,
}

impl Notifier {
    pub fn disabled() -> Self {
        Self { tx: None }
    }

    /// Starts the coalescing task. Must run inside a Tokio runtime.
    pub fn spawn() -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<UploadEvent>();
        tokio::spawn(async move {
            let mut failed_before = false;
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                let deadline = tokio::time::Instant::now() + COALESCE;
                while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, rx.recv()).await {
                    batch.push(ev);
                }
                let (summary, body) = summarize(&batch);
                let shown = tokio::task::spawn_blocking(move || {
                    notify_rust::Notification::new()
                        .appname("B-Ring")
                        .summary(&summary)
                        .body(&body)
                        .icon("folder-download")
                        .timeout(notify_rust::Timeout::Milliseconds(6000))
                        .show()
                        .map(|_| ())
                })
                .await;
                match shown {
                    Ok(Ok(())) => debug!(files = batch.len(), "notification shown"),
                    // Headless machines have no notification daemon: say so once.
                    Ok(Err(e)) if !failed_before => {
                        warn!(error = %e, "desktop notification failed (no notification daemon?)");
                        failed_before = true;
                    }
                    Ok(Err(e)) => debug!(error = %e, "desktop notification failed"),
                    Err(e) => warn!(error = %e, "notification task panicked"),
                }
            }
        });
        Self { tx: Some(tx) }
    }

    pub fn upload(&self, ev: UploadEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(ev);
        }
    }
}

fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Title and body for a batch of uploads.
pub fn summarize(batch: &[UploadEvent]) -> (String, String) {
    let mut devices: Vec<&str> = batch.iter().map(|e| e.device.as_str()).collect();
    devices.sort_unstable();
    devices.dedup();
    let from = devices.join(", ");
    let total: u64 = batch.iter().map(|e| e.size).sum();
    if let [one] = batch {
        return (
            format!("Received from {from}"),
            format!("{} ({})", one.path, human_size(one.size)),
        );
    }
    let mut body: Vec<String> = batch
        .iter()
        .take(MAX_LISTED)
        .map(|e| e.path.clone())
        .collect();
    if batch.len() > MAX_LISTED {
        body.push(format!("… and {} more", batch.len() - MAX_LISTED));
    }
    (
        format!(
            "{} files ({}) received from {from}",
            batch.len(),
            human_size(total)
        ),
        body.join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(path: &str, device: &str, size: u64) -> UploadEvent {
        UploadEvent {
            path: path.into(),
            device: device.into(),
            size,
        }
    }

    #[test]
    fn single_and_batched_messages() {
        let (s, b) = summarize(&[ev("Inbox/a.pdf", "pixel-8", 2048)]);
        assert_eq!(s, "Received from pixel-8");
        assert_eq!(b, "Inbox/a.pdf (2.0 KiB)");

        let many: Vec<_> = (0..7)
            .map(|i| {
                ev(
                    &format!("DCIM/{i}.jpg"),
                    if i % 2 == 0 { "pixel-8" } else { "quest-3" },
                    1 << 20,
                )
            })
            .collect();
        let (s, b) = summarize(&many);
        assert_eq!(s, "7 files (7.0 MiB) received from pixel-8, quest-3");
        assert_eq!(b.lines().count(), MAX_LISTED + 1);
        assert!(b.ends_with("… and 2 more"));
    }

    #[tokio::test]
    async fn disabled_notifier_is_a_no_op() {
        Notifier::disabled().upload(ev("x", "y", 1));
    }
}
