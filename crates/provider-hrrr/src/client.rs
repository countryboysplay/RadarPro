//! Anonymous, unauthenticated HTTP access to the public `noaa-hrrr-bdp-pds`
//! S3 bucket: plain `GET`/`HEAD` (with an HTTP Range request for the
//! sparse-fetch path), and `ListObjectsV2` for run discovery.
//!
//! No credentials anywhere -- same trust model `provider-gefs` already
//! established for `noaa-gefs-pds` (GLOBAL_CONTRACT: "public NOAA/NWS data
//! must provide a useful no-subscription baseline" / "credentials never
//! ship in source").

use crate::error::HrrrError;
use crate::keys::{ForecastHour, Product, RunReference, HRRR_BUCKET_URL};
use crate::xml::parse_list_objects_v2;

const MAX_PAGES: u32 = 1000;

/// A client for anonymous access to a public HRRR-layout S3 bucket.
#[derive(Clone)]
pub struct HrrrClient {
    http: reqwest::Client,
    bucket_url: String,
}

impl HrrrClient {
    /// Build a client against an arbitrary bucket base URL (no trailing
    /// slash) -- e.g. for tests against a local server.
    pub fn new(bucket_url: impl Into<String>) -> Result<Self, HrrrError> {
        #[cfg(not(target_arch = "wasm32"))]
        ensure_crypto_provider_installed();
        let builder = reqwest::Client::builder();
        // `ClientBuilder::timeout` does not exist on reqwest's wasm32
        // (browser `fetch`) backend -- see `provider-gefs::client::GefsClient::new`
        // for the same gating and rationale.
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.timeout(std::time::Duration::from_secs(30));
        let http = builder.build().map_err(|source| HrrrError::Request {
            url: "<client construction>".to_string(),
            source,
        })?;
        Ok(Self {
            http,
            bucket_url: bucket_url.into(),
        })
    }

    /// Build a client against the current public `noaa-hrrr-bdp-pds`
    /// bucket.
    pub fn default_bucket() -> Result<Self, HrrrError> {
        Self::new(HRRR_BUCKET_URL)
    }

    pub fn bucket_url(&self) -> &str {
        &self.bucket_url
    }

    fn url_for_key(&self, key: &str) -> String {
        crate::keys::object_url(&self.bucket_url, key)
    }

    /// List every object key under `prefix`, handling `ListObjectsV2`
    /// pagination correctly.
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<String>, HrrrError> {
        let mut continuation_token: Option<String> = None;
        let mut all_keys = Vec::new();

        for _page in 0..MAX_PAGES {
            let url = build_list_url(&self.bucket_url, prefix, continuation_token.as_deref())?;
            let response =
                self.http
                    .get(url.clone())
                    .send()
                    .await
                    .map_err(|source| HrrrError::Request {
                        url: url.to_string(),
                        source,
                    })?;
            let status = response.status();
            if !status.is_success() {
                return Err(HrrrError::HttpStatus {
                    url: url.to_string(),
                    status: status.as_u16(),
                });
            }
            let body = response
                .bytes()
                .await
                .map_err(|source| HrrrError::Request {
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

    /// Whether a specific (run, product, forecast hour) tuple's GRIB2
    /// object actually exists, via a real listing under that exact key's
    /// prefix (cheaper and just as authoritative as a HEAD request, and
    /// consistent with `provider-gefs`'s own "list, don't guess" discovery
    /// approach).
    ///
    /// Note this method's `Ok(false)` (a successful, empty listing) is the
    /// *only* representation of "genuinely absent" -- `ListObjectsV2`
    /// itself returns HTTP 200 with zero matches for a prefix nothing
    /// matches, never a 404. An `Err` here is therefore never "this run
    /// doesn't exist"; it is always a real request failure (timeout,
    /// transient HTTP error, malformed response), which is exactly why
    /// [`Self::object_exists_with_retries`] retries only on `Err`, never on
    /// `Ok(false)`.
    async fn object_exists(
        &self,
        run: RunReference,
        product: Product,
        forecast_hour: ForecastHour,
    ) -> Result<bool, HrrrError> {
        let key = crate::keys::object_key(run, product, forecast_hour);
        let keys = self.list_objects(&key).await?;
        Ok(keys.iter().any(|k| k == &key))
    }

    /// [`Self::object_exists`] with a bounded retry on transient failure.
    /// Confirmed empirically during this stage's own verification: a bare,
    /// unretried call can spuriously report "not found" for an object that
    /// demonstrably exists (reproduced twice, live, immediately followed by
    /// a successful request for the exact same key moments later) --
    /// `find_recent_run`'s tight, back-to-back-hour walk (up to
    /// `lookback_hours` sequential requests, no delay between them) is
    /// exactly the shape of traffic that can trip a transient failure or
    /// brief S3 throttling response. Retrying a handful of times with a
    /// short delay costs nothing when the object genuinely doesn't exist
    /// (`Ok(false)` returns immediately, no retry) and meaningfully
    /// improves real-world reliability when it does.
    async fn object_exists_with_retries(
        &self,
        run: RunReference,
        product: Product,
        forecast_hour: ForecastHour,
    ) -> Result<bool, HrrrError> {
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

        let mut last_err = None;
        for attempt in 0..MAX_ATTEMPTS {
            match self.object_exists(run, product, forecast_hour).await {
                Ok(exists) => return Ok(exists),
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 < MAX_ATTEMPTS {
                        forecast_core::sleep::sleep(RETRY_DELAY).await;
                    }
                }
            }
        }
        // Every attempt failed with a real error (never "confirmed absent",
        // per this method's own doc comment) -- surface it to the caller
        // rather than silently treating it as absence; `find_recent_run`
        // still just moves on to the next hour, but this keeps the
        // distinction available to anything that wants it (e.g. logging).
        Err(last_err.expect("loop runs at least once, so last_err is always set on this path"))
    }

    /// Find the most recent published HRRR run (walking backward hour by
    /// hour from the current UTC hour) that actually has a `conus` surface
    /// object published for `forecast_hour` -- a run takes up to roughly an
    /// hour after its nominal run hour to fully publish every forecast
    /// hour, so "the current UTC hour" may not exist yet at call time.
    pub async fn find_recent_run(
        &self,
        product: Product,
        forecast_hour: ForecastHour,
        lookback_hours: u32,
    ) -> Result<RunReference, HrrrError> {
        let (mut year, mut month, mut day) = forecast_core::time::today_utc_date();
        let now_hour = current_utc_hour();

        let mut hour = i64::from(now_hour);
        for _ in 0..=lookback_hours {
            if hour < 0 {
                let (y, m, d) = forecast_core::time::civil_date_minus_days(year, month, day, 1);
                year = y;
                month = m;
                day = d;
                hour += 24;
            }
            let run = RunReference::new(year, month, day, hour as u8);
            if let Ok(true) = self
                .object_exists_with_retries(run, product, forecast_hour)
                .await
            {
                return Ok(run);
            }
            hour -= 1;
        }

        Err(HrrrError::NoPublishedRunFound {
            product: product.as_str().to_string(),
            lookback_hours,
        })
    }

    /// Fetch a `.idx` sidecar as UTF-8 text.
    pub async fn fetch_idx_text(&self, key: &str) -> Result<String, HrrrError> {
        let url = self.url_for_key(key);
        let response = self.get(&url).await?;
        response
            .text()
            .await
            .map_err(|source| HrrrError::Request { url, source })
    }

    /// `HEAD` an object and return its `Content-Length`, needed only for
    /// computing the last message's byte range in a `.idx`.
    pub async fn content_length(&self, key: &str) -> Result<u64, HrrrError> {
        let url = self.url_for_key(key);
        let response = self
            .http
            .head(&url)
            .send()
            .await
            .map_err(|source| HrrrError::Request {
                url: url.clone(),
                source,
            })?;
        classify_status(&url, response.status())?;
        response
            .content_length()
            .ok_or_else(|| HrrrError::MissingContentLength { url: url.clone() })
    }

    /// Fetch exactly the half-open byte range `[start, end)` of an object
    /// via an HTTP Range request.
    pub async fn fetch_byte_range(
        &self,
        key: &str,
        start: u64,
        end: u64,
    ) -> Result<Vec<u8>, HrrrError> {
        let url = self.url_for_key(key);
        let response = self
            .http
            .get(&url)
            .header("Range", format!("bytes={start}-{}", end.saturating_sub(1)))
            .send()
            .await
            .map_err(|source| HrrrError::Request {
                url: url.clone(),
                source,
            })?;
        classify_status(&url, response.status())?;
        let bytes = response
            .bytes()
            .await
            .map_err(|source| HrrrError::Request {
                url: url.clone(),
                source,
            })?;
        Ok(bytes.to_vec())
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response, HrrrError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|source| HrrrError::Request {
                url: url.to_string(),
                source,
            })?;
        classify_status(url, response.status())?;
        Ok(response)
    }
}

fn classify_status(url: &str, status: reqwest::StatusCode) -> Result<(), HrrrError> {
    if status.is_success() {
        return Ok(());
    }
    if status.as_u16() == 404 {
        return Err(HrrrError::NotFound {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    Err(HrrrError::HttpStatus {
        url: url.to_string(),
        status: status.as_u16(),
    })
}

fn build_list_url(
    bucket_url: &str,
    prefix: &str,
    continuation_token: Option<&str>,
) -> Result<reqwest::Url, HrrrError> {
    let mut url = reqwest::Url::parse(bucket_url).map_err(|e| HrrrError::InvalidBucketUrl {
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

/// Current UTC hour (`0..=23`), from the system clock -- used only by
/// [`HrrrClient::find_recent_run`]'s discovery walk, never to compute or
/// validate a decoded field's own metadata (which always comes from the
/// GRIB2 message itself).
///
/// Goes through `forecast_core::time::unix_seconds_now()` rather than
/// calling `std::time::SystemTime::now()` directly (as this used to) --
/// that call panics unconditionally on `wasm32-unknown-unknown`, which
/// `forecast-web` builds this crate for; see that function's own doc
/// comment for why, and `today_utc_date`'s identical fix in the same
/// commit (this crate's own `find_recent_run` calls that too, a few lines
/// below).
fn current_utc_hour() -> u8 {
    ((forecast_core::time::unix_seconds_now() / 3600) % 24) as u8
}

/// Ensure a global `rustls` crypto provider is installed exactly once, per
/// process -- required because this crate builds `reqwest` with
/// `rustls-no-provider`. Idempotent: `install_default` erroring because a
/// provider is already installed (by an earlier call here, or by another
/// crate in the same process, e.g. `provider-gefs`/`radar-cache`) is
/// treated as success.
///
/// Native-only -- see `provider-gefs::client::ensure_crypto_provider_installed`
/// for why this has no wasm32 equivalent (this crate has no `rustls`
/// dependency at all on that target).
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
        let url = build_list_url(HRRR_BUCKET_URL, "hrrr.20260912/conus/", None).unwrap();
        assert_eq!(url.host_str(), Some("noaa-hrrr-bdp-pds.s3.amazonaws.com"));
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("list-type").map(String::as_str), Some("2"));
        assert_eq!(
            query.get("prefix").map(String::as_str),
            Some("hrrr.20260912/conus/")
        );
    }

    #[test]
    fn build_list_url_rejects_invalid_bucket_url() {
        let err = build_list_url("not a url", "prefix/", None).unwrap_err();
        assert!(matches!(err, HrrrError::InvalidBucketUrl { .. }));
    }

    #[test]
    fn current_utc_hour_is_a_plausible_value() {
        assert!(current_utc_hour() < 24);
    }
}
