//! Background update loop: periodically check a selected site for a new
//! latest scan and download/cache/verify it, skipping ones already cached.
//!
//! # Poll interval
//!
//! [`DEFAULT_POLL_INTERVAL`] is 45 seconds. WSR-88D volumes complete every
//! roughly 4-10 minutes depending on the active Volume Coverage Pattern, so
//! polling much more often than every 30-60s would just spend bandwidth
//! re-listing a directory that has not changed, and this task does not
//! need sub-second (or even sub-minute) precision on "a new scan just
//! landed." 45s sits in the middle of that documented 30-60s range.
//!
//! # Cancellation
//!
//! The loop is a plain `async fn` intended to be `tokio::spawn`ed; it
//! checks the caller's [`CancellationToken`] both between polls and while
//! sleeping, so switching the selected site is just: cancel the old
//! token, spawn a new loop with a new one. It never leaves a stale loop
//! for an old site running.

use crate::cache::{download_and_cache, CacheError, CachedVolume, DownloadOutcome};
use crate::discovery::{DiscoveryError, S3Client};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// See the module docs for the rationale.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(45);

/// What happened on one poll of the background update loop, for a caller
/// to observe (logging, a UI status line, or a test asserting on
/// behavior) without scraping stdout.
#[derive(Debug)]
pub enum UpdateEvent {
    /// A new volume was found, downloaded, and cached.
    Downloaded(CachedVolume),
    /// The latest available volume was already cached; nothing to do.
    AlreadyCached { icao: String, key: String },
    /// No volumes are available yet for the site's current UTC day.
    NoneAvailable { icao: String },
    /// Discovery or download failed this poll. The loop keeps running --
    /// this is reported, not fatal, since the next poll may well succeed
    /// (e.g. a transient outage that outlasted this poll's own retries).
    Error(String),
}

/// Configuration for one run of the background update loop.
pub struct UpdateLoopConfig {
    pub s3: S3Client,
    pub cache_root: PathBuf,
    pub icao: String,
    pub poll_interval: Duration,
}

impl UpdateLoopConfig {
    pub fn new(s3: S3Client, cache_root: PathBuf, icao: impl Into<String>) -> Self {
        Self {
            s3,
            cache_root,
            icao: icao.into(),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

/// Run the background update loop until `cancel` is triggered.
///
/// Checks immediately on entry (so a freshly selected site gets its latest
/// scan right away), then again every `config.poll_interval`. Intended to
/// be spawned via `tokio::spawn(run_update_loop(...))`; cancel the token
/// to stop it (e.g. because the user switched to a different site).
pub async fn run_update_loop<F>(
    config: UpdateLoopConfig,
    mut on_event: F,
    cancel: CancellationToken,
) where
    F: FnMut(UpdateEvent) + Send,
{
    loop {
        if cancel.is_cancelled() {
            return;
        }

        let event = check_and_update_once(&config, &cancel).await;
        on_event(event);

        tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(config.poll_interval) => {}
        }
    }
}

async fn check_and_update_once(
    config: &UpdateLoopConfig,
    cancel: &CancellationToken,
) -> UpdateEvent {
    let latest = match config.s3.discover_latest(&config.icao, cancel).await {
        Ok(Some(latest)) => latest,
        Ok(None) => {
            return UpdateEvent::NoneAvailable {
                icao: config.icao.clone(),
            }
        }
        Err(DiscoveryError::Cancelled) => {
            return UpdateEvent::Error("cancelled during discovery".to_string())
        }
        Err(e) => return UpdateEvent::Error(e.to_string()),
    };

    match download_and_cache(&config.s3, &config.cache_root, &latest, cancel).await {
        Ok(DownloadOutcome::Downloaded(cached)) => UpdateEvent::Downloaded(cached),
        Ok(DownloadOutcome::AlreadyCached(cached)) => UpdateEvent::AlreadyCached {
            icao: config.icao.clone(),
            key: cached.key,
        },
        Err(CacheError::Cancelled) => UpdateEvent::Error("cancelled during download".to_string()),
        Err(e) => UpdateEvent::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A background loop must stop promptly once cancelled, rather than
    /// running forever or requiring a full `poll_interval` to notice.
    #[tokio::test(flavor = "multi_thread")]
    async fn loop_stops_promptly_on_cancellation() {
        let cancel = CancellationToken::new();
        let events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let config = UpdateLoopConfig {
            // An unroutable/reserved address (per RFC 5737 documentation
            // range) so discovery fails fast with a connection error
            // rather than depending on any real network endpoint; this
            // test only exercises loop/cancellation plumbing, not real
            // discovery.
            s3: S3Client::new("http://192.0.2.1:1").unwrap(),
            cache_root: std::env::temp_dir().join("radar-cache-test-update-loop-unused"),
            icao: "KTLX".to_string(),
            poll_interval: Duration::from_secs(3600), // long enough that only cancellation ends the test
        };

        let events_clone = Arc::clone(&events);
        let cancel_clone = cancel.clone();
        let handle = tokio::spawn(async move {
            run_update_loop(
                config,
                move |event| events_clone.lock().unwrap().push(format!("{event:?}")),
                cancel_clone,
            )
            .await;
        });

        // Give the loop a moment to perform its immediate first check
        // (which will fail fast against the unroutable address) and reach
        // the sleep/select point, then cancel it.
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("loop must stop promptly after cancellation, not hang")
            .expect("loop task must not panic");

        assert!(
            !events.lock().unwrap().is_empty(),
            "the immediate first check should have reported at least one event before cancellation"
        );
    }
}
