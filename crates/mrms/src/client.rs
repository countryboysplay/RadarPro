//! Anonymous, unauthenticated HTTP access to the public `noaa-mrms-pds` S3
//! bucket: `ListObjectsV2` for snapshot discovery, plain `GET` for fetching
//! a whole (gzip-wrapped) object.
//!
//! No credentials anywhere -- same trust model `provider-gefs`/
//! `provider-hrrr` already established (GLOBAL_CONTRACT: "public NOAA/NWS
//! data must provide a useful no-subscription baseline" / "credentials
//! never ship in source").
//!
//! # Discovery: listing a day, not guessing a second
//!
//! `provider-hrrr::client::HrrrClient::find_recent_run` and
//! `provider-gefs::client::GefsClient::find_recent_run` both discover the
//! latest published run by *constructing a candidate key directly* (a
//! predictable run hour or forecast-hour file name) and checking whether it
//! exists, walking backward on a `Ok(false)`/never-retried-there,
//! bounded-retry-on-`Err` basis.
//!
//! MRMS cannot use that exact mechanism: confirmed empirically (2026-09-13,
//! live bucket), real MRMS snapshot timestamps drift by a couple of seconds
//! run to run (`...-000042`, `...-000242`, `...-000441`, ...) rather than
//! landing on a fixed, guessable `HHMMSS` grid -- see `crate::keys`'s module
//! doc. So this client instead **lists** a calendar day's worth of
//! snapshots at once (confirmed empirically: at most a few hundred keys for
//! a partial day, comfortably one un-paginated `ListObjectsV2` page) and
//! takes the lexicographically-greatest key (which, because every key
//! embeds a fixed-width `YYYYMMDD-HHMMSS`, sorts identically to
//! chronological order) as "latest" -- adapting, not reinventing, this
//! project's established discovery discipline: [`MrmsClient::list_objects`]
//! (S3 `ListObjectsV2`) returns `Ok` with an empty list for a day with no
//! published snapshots yet (e.g. just after UTC midnight) -- it never
//! 404s -- so, exactly like `discover_members`/`find_recent_run`'s existing
//! reasoning, an `Err` here is never legitimate absence, only a real
//! request failure; [`MrmsClient::list_objects_with_retries`] retries a
//! bounded number of times only on `Err`, never on a confirmed-empty `Ok`,
//! and [`MrmsClient::discover_latest_snapshot`]'s walk moves on to the
//! previous UTC day on either an empty listing or an exhausted retry budget
//! for the current day.

use crate::error::MrmsError;
use crate::keys::{parse_snapshot_key, MrmsProduct, SnapshotReference, MRMS_BUCKET_URL};
use crate::xml::parse_list_objects_v2;
use std::io::Read;

/// A hard ceiling on `ListObjectsV2` pagination -- defense against a
/// pathological/misbehaving server, not an expected real page count (a full
/// day of ~2-minute MRMS snapshots is a few hundred keys, far short of even
/// one page at S3's default 1000-key page size).
const MAX_PAGES: u32 = 1000;

/// A client for anonymous access to a public MRMS-layout S3 bucket.
#[derive(Clone)]
pub struct MrmsClient {
    http: reqwest::Client,
    bucket_url: String,
}

impl MrmsClient {
    /// Build a client against an arbitrary bucket base URL (no trailing
    /// slash) -- e.g. for tests against a local server.
    pub fn new(bucket_url: impl Into<String>) -> Result<Self, MrmsError> {
        #[cfg(not(target_arch = "wasm32"))]
        ensure_crypto_provider_installed();
        let builder = reqwest::Client::builder();
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.timeout(std::time::Duration::from_secs(30));
        let http = builder.build().map_err(|source| MrmsError::Request {
            url: "<client construction>".to_string(),
            source,
        })?;
        Ok(Self {
            http,
            bucket_url: bucket_url.into(),
        })
    }

    /// Build a client against the current public `noaa-mrms-pds` bucket.
    pub fn default_bucket() -> Result<Self, MrmsError> {
        Self::new(MRMS_BUCKET_URL)
    }

    pub fn bucket_url(&self) -> &str {
        &self.bucket_url
    }

    fn url_for_key(&self, key: &str) -> String {
        crate::keys::object_url(&self.bucket_url, key)
    }

    /// List every object key under `prefix`, handling `ListObjectsV2`
    /// pagination correctly.
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<String>, MrmsError> {
        let mut continuation_token: Option<String> = None;
        let mut all_keys = Vec::new();

        for _page in 0..MAX_PAGES {
            let url = build_list_url(&self.bucket_url, prefix, continuation_token.as_deref())?;
            let response =
                self.http
                    .get(url.clone())
                    .send()
                    .await
                    .map_err(|source| MrmsError::Request {
                        url: url.to_string(),
                        source,
                    })?;
            let status = response.status();
            if !status.is_success() {
                return Err(MrmsError::HttpStatus {
                    url: url.to_string(),
                    status: status.as_u16(),
                });
            }
            let body = response
                .bytes()
                .await
                .map_err(|source| MrmsError::Request {
                    url: url.to_string(),
                    source,
                })?;
            let page = parse_list_objects_v2(url.as_str(), &body)?;
            all_keys.extend(page.keys);

            if !page.is_truncated {
                return Ok(all_keys);
            }
            continuation_token = page.next_continuation_token;
            if continuation_token.is_none() {
                return Ok(all_keys);
            }
        }

        Ok(all_keys)
    }

    /// [`Self::list_objects`] with a bounded retry on transient failure --
    /// see this module's doc comment for why an `Err` here is never
    /// legitimate absence.
    async fn list_objects_with_retries(&self, prefix: &str) -> Result<Vec<String>, MrmsError> {
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

        let mut last_err = None;
        for attempt in 0..MAX_ATTEMPTS {
            match self.list_objects(prefix).await {
                Ok(keys) => return Ok(keys),
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 < MAX_ATTEMPTS {
                        forecast_core::sleep::sleep(RETRY_DELAY).await;
                    }
                }
            }
        }
        Err(last_err.expect("loop runs at least once, so last_err is always set on this path"))
    }

    /// Find the most recently published snapshot for `product`, walking
    /// backward one UTC calendar day at a time (up to `lookback_days`) --
    /// see this module's doc comment for why this lists a whole day rather
    /// than guessing an exact `HHMMSS`.
    pub async fn discover_latest_snapshot(
        &self,
        product: MrmsProduct,
        lookback_days: u32,
    ) -> Result<SnapshotReference, MrmsError> {
        let (mut year, mut month, mut day) = forecast_core::time::today_utc_date();

        for day_offset in 0..=lookback_days {
            if day_offset > 0 {
                let (y, m, d) = forecast_core::time::civil_date_minus_days(year, month, day, 1);
                year = y;
                month = m;
                day = d;
            }
            let prefix = SnapshotReference::day_prefix(product, year, month, day);
            if let Ok(keys) = self.list_objects_with_retries(&prefix).await {
                let latest = keys
                    .iter()
                    .filter_map(|k| parse_snapshot_key(product, k))
                    .max_by_key(|s| (s.year, s.month, s.day, s.hour, s.minute, s.second));
                if let Some(snapshot) = latest {
                    return Ok(snapshot);
                }
            }
        }

        Err(MrmsError::NoPublishedSnapshotFound {
            product: product.product_dir().to_string(),
            lookback_days,
        })
    }

    /// Fetch one whole object (gzip-wrapped GRIB2) and gunzip it, returning
    /// the decompressed GRIB2 bytes ready for [`crate::decode::decode_field`].
    /// Unlike GEFS/HRRR's sparse byte-range fetch (many fields packed into
    /// one large multi-message file), every real MRMS object observed is
    /// already a single-field, single-message file -- there is no smaller
    /// unit to sparse-fetch.
    pub async fn fetch_and_decompress(&self, key: &str) -> Result<Vec<u8>, MrmsError> {
        let url = self.url_for_key(key);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|source| MrmsError::Request {
                url: url.clone(),
                source,
            })?;
        classify_status(&url, response.status())?;
        let compressed = response
            .bytes()
            .await
            .map_err(|source| MrmsError::Request {
                url: url.clone(),
                source,
            })?;
        gunzip(&url, &compressed)
    }
}

/// Decompress gzip-wrapped bytes -- untrusted, network-sourced input, never
/// a panic on malformed/truncated/non-gzip data (GLOBAL_CONTRACT: "remote
/// data is unreliable and untrusted").
pub fn gunzip(url: &str, compressed: &[u8]) -> Result<Vec<u8>, MrmsError> {
    let mut decoder = flate2::read::GzDecoder::new(compressed);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| MrmsError::GzipDecompress {
            url: url.to_string(),
            message: e.to_string(),
        })?;
    Ok(decompressed)
}

fn classify_status(url: &str, status: reqwest::StatusCode) -> Result<(), MrmsError> {
    if status.is_success() {
        return Ok(());
    }
    if status.as_u16() == 404 {
        return Err(MrmsError::NotFound {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    Err(MrmsError::HttpStatus {
        url: url.to_string(),
        status: status.as_u16(),
    })
}

fn build_list_url(
    bucket_url: &str,
    prefix: &str,
    continuation_token: Option<&str>,
) -> Result<reqwest::Url, MrmsError> {
    let mut url = reqwest::Url::parse(bucket_url).map_err(|e| MrmsError::InvalidBucketUrl {
        url: bucket_url.to_string(),
        message: e.to_string(),
    })?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("list-type", "2");
        query.append_pair("prefix", prefix);
        if let Some(token) = continuation_token {
            query.append_pair("continuation-token", token);
        }
    }
    Ok(url)
}

/// Ensure a global `rustls` crypto provider is installed exactly once, per
/// process -- same idempotent pattern `provider-gefs`/`provider-hrrr` use
/// (this crate also builds `reqwest` with `rustls-no-provider`).
#[cfg(not(target_arch = "wasm32"))]
fn ensure_crypto_provider_installed() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_list_url_includes_prefix_and_list_type() {
        let url =
            build_list_url(MRMS_BUCKET_URL, "CONUS/PrecipRate_00.00/20260913/", None).unwrap();
        assert_eq!(url.host_str(), Some("noaa-mrms-pds.s3.amazonaws.com"));
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("list-type").map(String::as_str), Some("2"));
        assert_eq!(
            query.get("prefix").map(String::as_str),
            Some("CONUS/PrecipRate_00.00/20260913/")
        );
    }

    #[test]
    fn build_list_url_rejects_invalid_bucket_url() {
        let err = build_list_url("not a url", "prefix/", None).unwrap_err();
        assert!(matches!(err, MrmsError::InvalidBucketUrl { .. }));
    }

    #[test]
    fn gunzip_round_trips_real_compressed_data() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(b"GRIB fake payload for round-trip test 7777")
            .unwrap();
        let compressed = encoder.finish().unwrap();

        let decompressed = gunzip("u", &compressed).unwrap();
        assert_eq!(decompressed, b"GRIB fake payload for round-trip test 7777");
    }

    #[test]
    fn gunzip_rejects_garbage_cleanly_never_panics() {
        let err = gunzip("u", b"not gzip data at all").unwrap_err();
        assert!(matches!(err, MrmsError::GzipDecompress { .. }));
    }

    #[test]
    fn gunzip_rejects_empty_input_cleanly() {
        let err = gunzip("u", &[]).unwrap_err();
        assert!(matches!(err, MrmsError::GzipDecompress { .. }));
    }
}
