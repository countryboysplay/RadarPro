//! Download + atomic on-disk cache + verification.
//!
//! # Cache layout
//!
//! `{cache_root}/{ICAO}/{filename}` holds the raw, verified-decodable
//! Archive II bytes for one volume (`filename` is the object key's own
//! trailing filename, e.g. `KTLX20260912_000110_V06`), and
//! `{cache_root}/{ICAO}/{filename}.meta.json` is a small sidecar with the
//! metadata [`scan_history`] needs (site, key, start time, size). Scan
//! history is derived purely from these sidecar files on disk -- there is
//! no separate shared index file to keep consistent, which sidesteps any
//! need for cross-task locking around a single mutable index (multiple
//! sites/background loops only ever touch their own `{ICAO}/` subtree).
//!
//! # What "cache" holds here
//!
//! This is a cache of raw bytes that are *known to decode successfully*,
//! not a cache of decoded [`radar_types::Volume`] values held in memory.
//! Archive II bytes are already a compact on-disk representation; a decoded
//! `Volume` is a much larger in-memory structure (per-gate `Vec`s across
//! every radial of every sweep) that does not need to exist until a
//! consumer (a future renderer/analysis stage) actually wants it. Decoding
//! is therefore deferred to [`load_cached_volume`], which a caller can run
//! off its own hot/UI path (it takes no lock and touches no shared state
//! beyond reading one file), rather than this crate holding every cached
//! volume decoded and resident in memory at once.
//!
//! # Verification
//!
//! Bytes are never considered validly cached until
//! [`nexrad_level2::decode_volume`] succeeds on them. A download that
//! completes fully over HTTP but fails to decode (truncated, corrupted, or
//! an unsupported format) is retried at most once more (a fresh
//! `GetObject`, in case the corruption happened in transit) before being
//! reported as [`CacheError::VerificationFailed`] -- and in neither case is
//! a bad file ever renamed into the final cache path: bytes are verified
//! *before* the atomic write, not after.
//!
//! # Atomicity
//!
//! The final cache path is only ever reached via `write` (to a sibling
//! temp path in the same directory, so the later rename stays on one
//! filesystem) then `rename` (`tokio::fs::rename`, which -- like
//! `std::fs::rename` -- replaces an existing destination atomically on
//! both Unix `rename(2)` and Windows `MoveFileExW` with
//! `MOVEFILE_REPLACE_EXISTING`). A crash or cancellation between those two
//! steps leaves, at worst, an orphaned `*.tmp-*` file next to the cache
//! entry and never a partially-written file at the real cache path.

use crate::keys::DiscoveredVolume;
use crate::net::{self, BackoffPolicy};
use radar_types::{Timestamp, Volume};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// One extra full re-download-and-verify attempt after a decode/verify
/// failure, on the theory that the corruption may have happened in
/// transit rather than being inherent to the object -- but no more than
/// that: repeatedly re-downloading a genuinely bad object cannot fix it,
/// and per-attempt network-level transient failures already get their own
/// retries within each of these two attempts (see `net::BackoffPolicy`).
const MAX_VERIFY_ATTEMPTS: u32 = 2;

/// A short, fixed pause before a verify-triggered re-download. Not part of
/// the exponential backoff policy (a decode failure is not a network
/// signal), but a full-speed immediate re-request is needlessly impolite
/// to the server for what is, in the common case, going to be the same
/// bytes again.
const VERIFY_RETRY_PAUSE: Duration = Duration::from_secs(1);

/// Errors from downloading, verifying, and caching one volume.
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cancelled before completing the operation")]
    Cancelled,
    #[error("object not found (HTTP 404): {key}")]
    NotFound { key: String },
    #[error("download of {key} failed after {attempts} attempt(s): {source}")]
    DownloadRequestFailed {
        key: String,
        attempts: u32,
        #[source]
        source: reqwest::Error,
    },
    #[error("download of {key} returned HTTP {status} after {attempts} attempt(s)")]
    DownloadHttpStatus {
        key: String,
        status: u16,
        attempts: u32,
    },
    #[error(
        "downloaded {key} but it failed to decode/verify after {attempts} attempt(s): {source}"
    )]
    VerificationFailed {
        key: String,
        attempts: u32,
        #[source]
        source: nexrad_level2::DecodeError,
    },
    #[error("invalid object URL for key {key:?}: {message}")]
    InvalidUrl { key: String, message: String },
    #[error("local cache I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to (de)serialize cache sidecar metadata: {0}")]
    Metadata(#[from] serde_json::Error),
    #[error("discovery error while looking up an object to cache: {0}")]
    Discovery(#[from] crate::discovery::DiscoveryError),
}

/// One volume's metadata as recorded in the on-disk cache -- enough for a
/// future UI to browse recent scans without decoding anything (see
/// [`scan_history`]).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedVolume {
    pub icao: String,
    pub key: String,
    pub start_time: Timestamp,
    pub format_version: u8,
    pub file_path: PathBuf,
    pub size_bytes: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct SidecarMeta {
    icao: String,
    key: String,
    start_time_epoch_millis: i64,
    format_version: u8,
    size_bytes: u64,
}

fn paths_for(cache_root: &Path, discovered: &DiscoveredVolume) -> (PathBuf, PathBuf) {
    let dir = cache_root.join(&discovered.icao);
    let file_name = discovered.file_name();
    let data_path = dir.join(file_name);
    let meta_path = dir.join(format!("{file_name}.meta.json"));
    (data_path, meta_path)
}

/// Whether [`download_and_cache`] performed a fresh download/verify or
/// found the volume already cached and skipped straight to returning its
/// metadata. Both cases carry the same [`CachedVolume`]; the distinction
/// exists purely for callers (e.g. the background update loop) that want
/// to report/log which one happened.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadOutcome {
    Downloaded(CachedVolume),
    AlreadyCached(CachedVolume),
}

impl DownloadOutcome {
    pub fn into_cached_volume(self) -> CachedVolume {
        match self {
            DownloadOutcome::Downloaded(c) | DownloadOutcome::AlreadyCached(c) => c,
        }
    }

    pub fn cached_volume(&self) -> &CachedVolume {
        match self {
            DownloadOutcome::Downloaded(c) | DownloadOutcome::AlreadyCached(c) => c,
        }
    }
}

/// Download, verify, and atomically cache one discovered volume. If it is
/// already cached (a matching data file + sidecar already exist under
/// `cache_root`), returns the existing entry immediately without
/// re-downloading -- this is what lets the background update loop
/// (`crate::update_loop`) call this unconditionally and skip work for
/// volumes it already has.
pub async fn download_and_cache(
    s3: &crate::discovery::S3Client,
    cache_root: &Path,
    discovered: &DiscoveredVolume,
    cancel: &CancellationToken,
) -> Result<DownloadOutcome, CacheError> {
    let (data_path, meta_path) = paths_for(cache_root, discovered);

    if let Some(existing) = read_existing(&data_path, &meta_path).await? {
        return Ok(DownloadOutcome::AlreadyCached(existing));
    }

    if let Some(parent) = data_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let bytes = fetch_and_verify(s3, discovered, cancel).await?;

    let cached = write_cache_entry(&data_path, &meta_path, discovered, &bytes).await?;
    Ok(DownloadOutcome::Downloaded(cached))
}

/// Fetch `discovered`'s object bytes and confirm they decode, retrying a
/// verify failure once more per [`MAX_VERIFY_ATTEMPTS`]/module docs. Does
/// not touch the cache directory.
async fn fetch_and_verify(
    s3: &crate::discovery::S3Client,
    discovered: &DiscoveredVolume,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, CacheError> {
    let url = build_object_url(s3.bucket_url(), &discovered.key)?;

    let mut last_verify_error: Option<nexrad_level2::DecodeError> = None;

    for verify_attempt in 1..=MAX_VERIFY_ATTEMPTS {
        if cancel.is_cancelled() {
            return Err(CacheError::Cancelled);
        }

        let bytes = net::fetch_bytes(
            s3.http_client(),
            url.clone(),
            cancel,
            &BackoffPolicy::DEFAULT,
        )
        .await
        .map_err(|e| map_fetch_error(e, &discovered.key))?;

        match nexrad_level2::decode_volume(&bytes) {
            Ok(_) => return Ok(bytes),
            Err(decode_err) => {
                last_verify_error = Some(decode_err);
                if verify_attempt < MAX_VERIFY_ATTEMPTS {
                    tokio::select! {
                        biased;
                        () = cancel.cancelled() => return Err(CacheError::Cancelled),
                        () = tokio::time::sleep(VERIFY_RETRY_PAUSE) => {}
                    }
                }
            }
        }
    }

    Err(CacheError::VerificationFailed {
        key: discovered.key.clone(),
        attempts: MAX_VERIFY_ATTEMPTS,
        source: last_verify_error
            .expect("loop runs at least once and only exits here after setting this"),
    })
}

fn map_fetch_error(e: net::FetchError, key: &str) -> CacheError {
    match e {
        net::FetchError::Cancelled => CacheError::Cancelled,
        net::FetchError::Request { attempts, source } => CacheError::DownloadRequestFailed {
            key: key.to_string(),
            attempts,
            source,
        },
        net::FetchError::Status { status: 404, .. } => CacheError::NotFound {
            key: key.to_string(),
        },
        net::FetchError::Status {
            status, attempts, ..
        } => CacheError::DownloadHttpStatus {
            key: key.to_string(),
            status,
            attempts,
        },
    }
}

fn build_object_url(bucket_url: &str, key: &str) -> Result<reqwest::Url, CacheError> {
    let full = format!("{}/{}", bucket_url.trim_end_matches('/'), key);
    reqwest::Url::parse(&full).map_err(|e| CacheError::InvalidUrl {
        key: key.to_string(),
        message: e.to_string(),
    })
}

/// Write already-verified bytes into the cache atomically (data file +
/// sidecar metadata), and return the resulting [`CachedVolume`].
///
/// Split out from [`fetch_and_verify`] so atomic-write behavior is
/// directly unit-testable with synthetic bytes, with no network involved.
pub(crate) async fn write_cache_entry(
    data_path: &Path,
    meta_path: &Path,
    discovered: &DiscoveredVolume,
    bytes: &[u8],
) -> Result<CachedVolume, CacheError> {
    atomic_write(data_path, bytes).await?;

    let meta = SidecarMeta {
        icao: discovered.icao.clone(),
        key: discovered.key.clone(),
        start_time_epoch_millis: discovered.start_time.epoch_millis(),
        format_version: discovered.format_version,
        size_bytes: bytes.len() as u64,
    };
    let meta_json = serde_json::to_vec_pretty(&meta)?;
    atomic_write(meta_path, &meta_json).await?;

    Ok(CachedVolume {
        icao: meta.icao,
        key: meta.key,
        start_time: discovered.start_time,
        format_version: meta.format_version,
        file_path: data_path.to_path_buf(),
        size_bytes: meta.size_bytes,
    })
}

/// Write `contents` to a temp path beside `final_path`, then rename it into
/// place. `final_path` is never observed in a partially-written state: any
/// reader either sees no file, the previous file (if any), or the complete
/// new file.
async fn atomic_write(final_path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temp_path = temp_sibling_path(final_path);
    if let Err(e) = tokio::fs::write(&temp_path, contents).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(e);
    }
    if let Err(e) = tokio::fs::rename(&temp_path, final_path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(e);
    }
    Ok(())
}

fn temp_sibling_path(final_path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = final_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("cache-entry");
    final_path.with_file_name(format!("{file_name}.tmp-{pid}-{n}"))
}

/// Check whether `data_path`/`meta_path` already form a complete cache
/// entry. A missing or unreadable/corrupt sidecar is treated as "not
/// cached" (so `download_and_cache` will redo the download and overwrite
/// it), not as an error -- a half-written leftover from an earlier crash
/// must never block re-caching.
async fn read_existing(
    data_path: &Path,
    meta_path: &Path,
) -> Result<Option<CachedVolume>, CacheError> {
    let (Ok(data_exists), Ok(meta_bytes)) = (
        tokio::fs::try_exists(data_path).await,
        tokio::fs::read(meta_path).await,
    ) else {
        return Ok(None);
    };
    if !data_exists {
        return Ok(None);
    }
    let Ok(meta) = serde_json::from_slice::<SidecarMeta>(&meta_bytes) else {
        return Ok(None);
    };
    let size_bytes = match tokio::fs::metadata(data_path).await {
        Ok(m) => m.len(),
        Err(_) => return Ok(None),
    };
    Ok(Some(CachedVolume {
        icao: meta.icao,
        key: meta.key,
        start_time: Timestamp::from_epoch_millis(meta.start_time_epoch_millis),
        format_version: meta.format_version,
        file_path: data_path.to_path_buf(),
        size_bytes,
    }))
}

/// List the volumes currently cached for `icao` under `cache_root`, newest
/// first, by scanning `{cache_root}/{icao}/*.meta.json` sidecars (a
/// corrupt or unreadable sidecar is skipped, not an error, for the same
/// reason `read_existing` treats one that way).
///
/// This is plain synchronous directory/file I/O (not `async`): it is a
/// metadata-only scan of small JSON sidecars, not a decode of any radar
/// volume, so it is fine to call directly rather than needing to be
/// off-thread per GLOBAL_CONTRACT's "UI thread must not synchronously
/// decode complete radar volumes" (which this function never does).
pub fn scan_history(cache_root: &Path, icao: &str) -> Result<Vec<CachedVolume>, CacheError> {
    let dir = cache_root.join(icao);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(CacheError::Io(e)),
    };

    let mut volumes = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let Some(data_file_name) = file_name.strip_suffix(".meta.json") else {
            continue;
        };
        let data_path = dir.join(data_file_name);

        let Ok(meta_bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<SidecarMeta>(&meta_bytes) else {
            continue;
        };
        let Ok(fs_meta) = std::fs::metadata(&data_path) else {
            continue;
        };

        volumes.push(CachedVolume {
            icao: meta.icao,
            key: meta.key,
            start_time: Timestamp::from_epoch_millis(meta.start_time_epoch_millis),
            format_version: meta.format_version,
            file_path: data_path,
            size_bytes: fs_meta.len(),
        });
    }

    volumes.sort_by(|a, b| {
        b.start_time
            .cmp(&a.start_time)
            .then_with(|| b.key.cmp(&a.key))
    });
    Ok(volumes)
}

/// Decode a cached volume's data file on demand.
///
/// Deferred/on-demand by design (see the module docs): this crate's cache
/// holds verified-good raw bytes, not decoded [`Volume`]s resident in
/// memory. Callers with a UI thread must run this off of it (a background
/// task, `spawn_blocking`, or equivalent) -- this function itself performs
/// synchronous file I/O and a full volume decode, deliberately not wrapped
/// in `async`, so that obligation stays visible at the call site rather
/// than being silently hidden behind an `.await`.
pub fn load_cached_volume(cached: &CachedVolume) -> Result<Volume, CacheError> {
    let bytes = std::fs::read(&cached.file_path)?;
    nexrad_level2::decode_volume(&bytes).map_err(|source| CacheError::VerificationFailed {
        key: cached.key.clone(),
        attempts: 1,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::parse_object_key;

    fn unique_test_dir(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "radar-cache-test-{name}-{}-{}-{n}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn sample_discovered() -> DiscoveredVolume {
        parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V06").unwrap()
    }

    fn fixture_bytes(file_name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/nexrad-level2")
            .join(file_name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {e}", path.display()))
    }

    #[tokio::test]
    async fn write_cache_entry_persists_valid_bytes_atomically() {
        let cache_root = unique_test_dir("valid-write");
        let discovered = sample_discovered();
        let (data_path, meta_path) = paths_for(&cache_root, &discovered);
        tokio::fs::create_dir_all(data_path.parent().unwrap())
            .await
            .unwrap();

        let bytes = fixture_bytes("KTLX20240601_000353_V06");
        let cached = write_cache_entry(&data_path, &meta_path, &discovered, &bytes)
            .await
            .unwrap();

        assert_eq!(cached.icao, "KTLX");
        assert_eq!(cached.size_bytes, bytes.len() as u64);
        assert!(data_path.is_file());
        assert!(meta_path.is_file());
        assert_eq!(std::fs::read(&data_path).unwrap(), bytes);

        // No leftover temp files after a clean write.
        let leftovers: Vec<_> = std::fs::read_dir(data_path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "atomic_write should leave no .tmp- files behind"
        );

        let _ = std::fs::remove_dir_all(&cache_root);
    }

    /// Simulates a crash mid-download: a `.tmp-*` file is written directly
    /// (bypassing the rename step atomic_write would normally perform) and
    /// left in place. The final cache path must never see that partial
    /// content -- it must simply not exist, since the rename that would
    /// have exposed it never happened.
    #[tokio::test]
    async fn interrupted_write_never_exposes_partial_content_at_the_final_path() {
        let cache_root = unique_test_dir("interrupted-write");
        let discovered = sample_discovered();
        let (data_path, meta_path) = paths_for(&cache_root, &discovered);
        tokio::fs::create_dir_all(data_path.parent().unwrap())
            .await
            .unwrap();

        // Simulate a crash partway through: only the temp file gets
        // written, never renamed.
        let partial_temp = temp_sibling_path(&data_path);
        tokio::fs::write(&partial_temp, b"only half of the bytes")
            .await
            .unwrap();

        assert!(
            !data_path.exists(),
            "final path must not exist before any rename happens"
        );

        // Now perform a real (uninterrupted) atomic write of the correct
        // bytes, as a retry after the simulated crash would do.
        let good_bytes = fixture_bytes("KFTG20240601_000116_V06");
        let cached = write_cache_entry(&data_path, &meta_path, &discovered, &good_bytes)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&data_path).unwrap(), good_bytes);
        assert_eq!(cached.size_bytes, good_bytes.len() as u64);

        let _ = std::fs::remove_file(&partial_temp);
        let _ = std::fs::remove_dir_all(&cache_root);
    }

    #[tokio::test]
    async fn fetch_and_verify_style_rejection_never_writes_bad_bytes() {
        // Exercises the same "decode before write" ordering `fetch_and_verify`
        // relies on, without a network call: garbage bytes must fail
        // `decode_volume`, and this test asserts that failing verification
        // that way (as `fetch_and_verify` does before ever calling
        // `write_cache_entry`) means the cache directory stays empty.
        let cache_root = unique_test_dir("verify-reject");
        let discovered = sample_discovered();
        let (data_path, _meta_path) = paths_for(&cache_root, &discovered);

        let garbage = b"not a NEXRAD Archive II file".to_vec();
        let decode_result = nexrad_level2::decode_volume(&garbage);
        assert!(
            decode_result.is_err(),
            "garbage bytes must fail verification"
        );

        // `fetch_and_verify` would return early here without ever calling
        // `write_cache_entry`; confirm the path it would have used is
        // still untouched.
        assert!(!data_path.exists());
        assert!(!cache_root.exists() || std::fs::read_dir(&cache_root).unwrap().next().is_none());

        let _ = std::fs::remove_dir_all(&cache_root);
    }

    #[tokio::test]
    async fn scan_history_reflects_cached_volumes_newest_first() {
        let cache_root = unique_test_dir("scan-history");

        let earlier = parse_object_key("2026/09/12/KTLX/KTLX20260912_000110_V06").unwrap();
        let later = parse_object_key("2026/09/12/KTLX/KTLX20260912_000440_V06").unwrap();

        for (discovered, fixture) in [
            (&earlier, "KTLX20240601_000353_V06"),
            (&later, "KFTG20240601_000116_V06"),
        ] {
            let (data_path, meta_path) = paths_for(&cache_root, discovered);
            tokio::fs::create_dir_all(data_path.parent().unwrap())
                .await
                .unwrap();
            let bytes = fixture_bytes(fixture);
            write_cache_entry(&data_path, &meta_path, discovered, &bytes)
                .await
                .unwrap();
        }

        let history = scan_history(&cache_root, "KTLX").unwrap();
        assert_eq!(history.len(), 2);
        // Newest first.
        assert_eq!(history[0].key, later.key);
        assert_eq!(history[1].key, earlier.key);

        let _ = std::fs::remove_dir_all(&cache_root);
    }

    #[test]
    fn scan_history_on_missing_directory_is_empty_not_an_error() {
        let cache_root = unique_test_dir("scan-history-missing");
        let history = scan_history(&cache_root, "KTLX").unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn load_cached_volume_decodes_a_real_cached_file() {
        let cache_root = unique_test_dir("load-cached");
        let discovered = sample_discovered();
        let (data_path, _meta_path) = paths_for(&cache_root, &discovered);
        std::fs::create_dir_all(data_path.parent().unwrap()).unwrap();
        let bytes = fixture_bytes("KTLX20240601_000353_V06");
        std::fs::write(&data_path, &bytes).unwrap();

        let cached = CachedVolume {
            icao: "KTLX".to_string(),
            key: discovered.key.clone(),
            start_time: discovered.start_time,
            format_version: discovered.format_version,
            file_path: data_path,
            size_bytes: bytes.len() as u64,
        };

        let volume = load_cached_volume(&cached).unwrap();
        assert_eq!(volume.site.icao, "KTLX");

        let _ = std::fs::remove_dir_all(&cache_root);
    }

    #[test]
    fn build_object_url_joins_bucket_and_key() {
        let url = build_object_url(
            crate::discovery::NEXRAD_LEVEL2_BUCKET_URL,
            "2026/09/12/KTLX/KTLX20260912_000110_V06",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://unidata-nexrad-level2.s3.amazonaws.com/2026/09/12/KTLX/KTLX20260912_000110_V06"
        );
    }
}
