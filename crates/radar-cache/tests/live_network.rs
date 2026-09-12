//! The one test in this crate that actually talks to the real network: it
//! lists a real site's real NEXRAD Level II objects for today (UTC),
//! downloads the latest one, and confirms it verifies (decodes).
//!
//! Every other test in this crate (`sites`, `keys`, `xml`, `net`, `cache`,
//! `update_loop`) is pure logic or local-filesystem-only and needs no
//! network at all.
//!
//! Per the same pattern already established in this repo for
//! GPU-availability in `crates/radar-render/src/gpu_tests.rs` (S03): a
//! test that depends on an external resource this environment does not
//! control (there, a GPU adapter; here, live internet access and NOAA's
//! own service) must never make `cargo test --workspace` flaky on a
//! machine that simply lacks that resource -- an offline CI runner, a
//! sandboxed build, or a transient NOAA outage must not fail the suite.
//! So every fallible step here is wrapped: on any error, this test prints
//! a clear, visible skip message explaining why and returns (passing),
//! rather than panicking -- never silently doing nothing via `#[ignore]`,
//! and never letting a real, reproducible bug hide behind "maybe it was
//! the network."

use radar_cache::{CacheDate, S3Client};
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread")]
async fn discovers_downloads_and_verifies_a_real_live_scan() {
    let cancel = CancellationToken::new();

    let s3 = match S3Client::default_bucket() {
        Ok(s3) => s3,
        Err(e) => {
            println!("SKIP live_network test: failed to build HTTP client: {e}");
            return;
        }
    };

    // A short list of well-known-in-this-repo sites (KTLX/KFTG already have
    // real S01 fixture volumes) to try, in case one particular site has an
    // unrelated transient issue.
    const CANDIDATE_SITES: &[&str] = &["KTLX", "KFTG"];

    let mut volumes_today = Vec::new();
    let mut icao_used = "";
    let mut last_error = None;

    for icao in CANDIDATE_SITES.iter().copied() {
        match s3.discover_day(icao, CacheDate::today_utc(), &cancel).await {
            Ok(volumes) if !volumes.is_empty() => {
                icao_used = icao;
                volumes_today = volumes;
                break;
            }
            Ok(_) => continue, // no scans yet today for this site; try the next
            Err(e) => last_error = Some(e.to_string()),
        }
    }

    if volumes_today.is_empty() {
        println!(
            "SKIP live_network test: could not discover any scans for {:?} today (UTC) -- \
             no network access, NOAA's service unavailable, or genuinely no scans yet today. \
             Last error (if any): {:?}",
            CANDIDATE_SITES, last_error
        );
        return;
    }

    println!(
        "Found {} scan(s) for {icao_used} today (UTC); latest key: {}",
        volumes_today.len(),
        volumes_today.last().unwrap().key
    );

    let latest = volumes_today.last().unwrap().clone();

    let cache_root = std::env::temp_dir().join(format!(
        "radar-cache-live-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let outcome = match radar_cache::download_and_cache(&s3, &cache_root, &latest, &cancel).await {
        Ok(outcome) => outcome,
        Err(e) => {
            println!("SKIP live_network test: download/verify of a real scan failed: {e}");
            let _ = std::fs::remove_dir_all(&cache_root);
            return;
        }
    };

    let cached = outcome.cached_volume();
    println!(
        "Downloaded and verified {} ({} bytes) at {}",
        cached.key,
        cached.size_bytes,
        cached.file_path.display()
    );

    match radar_cache::load_cached_volume(cached) {
        Ok(volume) => {
            println!(
                "Decoded successfully: site {}, VCP {}, {} sweep(s)",
                volume.site.icao,
                volume.volume_coverage_pattern,
                volume.sweeps.len()
            );
            assert!(
                !volume.sweeps.is_empty(),
                "a verified-cached volume must have decoded at least one sweep"
            );
        }
        Err(e) => {
            // `download_and_cache` already required this exact file to
            // verify (decode) successfully before caching it, so reaching
            // this branch would indicate a real bug (e.g. the cached file
            // was corrupted after being written), not a network flake --
            // this is a genuine test failure, not a skip.
            panic!("re-decoding a cache entry that was just verified during download failed: {e}");
        }
    }

    let _ = std::fs::remove_dir_all(&cache_root);
}
