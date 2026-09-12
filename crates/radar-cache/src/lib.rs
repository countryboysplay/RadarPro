//! `radar-cache` -- S04 live NEXRAD Level II data pipeline.
//!
//! Implements the `discover -> download -> verify -> cache` half of the
//! S04 pipeline (`discover -> download -> verify -> cache -> decode -> GPU
//! upload -> render`): a WSR-88D site directory, S3 `ListObjectsV2`-based
//! object discovery, retrying/cancellable download, atomic on-disk
//! caching with mandatory decode verification, a background
//! poll-and-update loop, and scan-history browsing. It does **not** decode
//! into a resident `Volume` for every cached entry, upload anything to the
//! GPU, or render -- those stay downstream (`nexrad-level2` for decode,
//! `radar-render` for GPU upload/rendering, per S03), and MapLibre/web
//! integration is a separate follow-up task.
//!
//! # Threat model
//!
//! Per `GLOBAL_CONTRACT.md` ("remote data is unreliable and untrusted" /
//! "no uncontrolled panics on malformed input"), every module here treats
//! the network as adversarial: S3 XML responses are parsed with a
//! bounds-checked pull parser that returns structured errors instead of
//! panicking ([`xml`]), object keys are pattern-matched rather than
//! trusted ([`keys`]), and a downloaded object is never treated as a valid
//! cache entry until it verifiably decodes ([`cache`]). The one exception
//! is the embedded site directory ([`sites`]): that JSON is vendored,
//! compile-time-fixed data controlled by this crate, not network input, so
//! a parse failure there is treated as a build-time defect (a panic), not
//! a runtime condition to recover from.
//!
//! # Map of what's here
//!
//! - Site directory: [`all_sites`] / [`find_site`].
//! - Discovery: [`S3Client`] (`discover_day`/`discover_latest`),
//!   [`DiscoveredVolume`], [`parse_object_key`], [`CacheDate`].
//!   `ListObjectsV2` XML parsing and the shared retry/backoff/cancellation
//!   HTTP fetch helper are internal (not part of the public API).
//! - Download/cache/verify: [`download_and_cache`], [`DownloadOutcome`],
//!   [`CachedVolume`], [`scan_history`], and on-demand decode via
//!   [`load_cached_volume`].
//! - Background updates: [`run_update_loop`], [`UpdateLoopConfig`],
//!   [`UpdateEvent`], [`DEFAULT_POLL_INTERVAL`].
//!
//! # Cancellation
//!
//! Every network/retry operation in this crate accepts a
//! `tokio_util::sync::CancellationToken`. Cancelling one promptly abandons
//! the in-flight discovery/download/retry-wait it was passed to, which is
//! how a caller stops stale work for a site the user has since switched
//! away from (GLOBAL_CONTRACT: "cancellation/deprioritization is required
//! for stale network/decode work").
//!
//! See `docs/adr/0008-live-data-pipeline-architecture.md` for the full
//! design rationale (dependency choices, retry/backoff parameters, cache
//! layout, atomic-write approach).

mod cache;
mod discovery;
mod keys;
mod net;
mod sites;
mod update_loop;
mod xml;

pub use cache::{
    download_and_cache, load_cached_volume, scan_history, CacheError, CachedVolume, DownloadOutcome,
};
pub use discovery::{DiscoveryError, S3Client, NEXRAD_LEVEL2_BUCKET_URL};
pub use keys::{parse_object_key, CacheDate, DiscoveredVolume};
pub use sites::{all_sites, find_site, SiteInfo};
pub use update_loop::{run_update_loop, UpdateEvent, UpdateLoopConfig, DEFAULT_POLL_INTERVAL};
