//! Anonymous, unauthenticated HTTP access to the public `noaa-gefs-pds` S3
//! bucket: `ListObjectsV2` for member discovery, plain `GET`/`HEAD` (with
//! an HTTP Range request for the sparse-fetch path) for everything else.
//!
//! No credentials anywhere -- same trust model `radar-cache` already
//! established for `unidata-nexrad-level2` (GLOBAL_CONTRACT: "public
//! NOAA/NWS data must provide a useful no-subscription baseline" /
//! "credentials never ship in source").

use crate::error::GefsError;
use crate::keys::{ForecastHour, MemberKey, ProductGroup, RunHour, RunReference, GEFS_BUCKET_URL};
use crate::xml::parse_list_objects_v2;

/// A hard ceiling on `ListObjectsV2` pagination, purely as a defense
/// against a pathological/misbehaving server that keeps returning
/// `IsTruncated=true` forever -- a real GEFS run's `pgrb2sp25` product
/// group for one forecast hour has at most a few dozen keys, far short of
/// even one page at S3's default 1000-key page size.
const MAX_PAGES: u32 = 1000;

/// A client for anonymous access to a public GEFS-layout S3 bucket.
#[derive(Clone)]
pub struct GefsClient {
    http: reqwest::Client,
    bucket_url: String,
}

impl GefsClient {
    /// Build a client against an arbitrary bucket base URL (no trailing
    /// slash) -- e.g. for tests against a local server.
    pub fn new(bucket_url: impl Into<String>) -> Result<Self, GefsError> {
        #[cfg(not(target_arch = "wasm32"))]
        ensure_crypto_provider_installed();
        let builder = reqwest::Client::builder();
        // `ClientBuilder::timeout` does not exist on reqwest's wasm32
        // (browser `fetch`) backend -- there is no client-wide request
        // timeout to set there; a hung fetch is left to the browser's own
        // defaults on that target. Native keeps the same 30s timeout this
        // crate has always used.
        #[cfg(not(target_arch = "wasm32"))]
        let builder = builder.timeout(std::time::Duration::from_secs(30));
        let http = builder.build().map_err(|source| GefsError::Request {
            url: "<client construction>".to_string(),
            source,
        })?;
        Ok(Self {
            http,
            bucket_url: bucket_url.into(),
        })
    }

    /// Build a client against the current public `noaa-gefs-pds` bucket.
    pub fn default_bucket() -> Result<Self, GefsError> {
        Self::new(GEFS_BUCKET_URL)
    }

    pub fn bucket_url(&self) -> &str {
        &self.bucket_url
    }

    fn url_for_key(&self, key: &str) -> String {
        crate::keys::object_url(&self.bucket_url, key)
    }

    /// List every object key under `prefix`, handling `ListObjectsV2`
    /// pagination correctly rather than silently truncating at the default
    /// 1000-key page size.
    pub async fn list_objects(&self, prefix: &str) -> Result<Vec<String>, GefsError> {
        let mut continuation_token: Option<String> = None;
        let mut all_keys = Vec::new();

        for _page in 0..MAX_PAGES {
            let url = build_list_url(&self.bucket_url, prefix, continuation_token.as_deref())?;
            let response =
                self.http
                    .get(url.clone())
                    .send()
                    .await
                    .map_err(|source| GefsError::Request {
                        url: url.to_string(),
                        source,
                    })?;
            let status = response.status();
            if !status.is_success() {
                return Err(GefsError::HttpStatus {
                    url: url.to_string(),
                    status: status.as_u16(),
                });
            }
            let body = response
                .bytes()
                .await
                .map_err(|source| GefsError::Request {
                    url: url.to_string(),
                    source,
                })?;
            let page = parse_list_objects_v2(url.as_str(), &body)?;
            all_keys.extend(page.keys);

            if !page.is_truncated {
                return Ok(all_keys);
            }
            // If a server ever claims `IsTruncated=true` with no
            // continuation token, stop rather than looping forever --
            // treated the same as "done" since there is no way to
            // continue. Unreachable against a spec-compliant S3 response.
            continuation_token = page.next_continuation_token;
            if continuation_token.is_none() {
                return Ok(all_keys);
            }
        }

        Ok(all_keys)
    }

    /// Discover which ensemble member files actually exist for a given
    /// (run, product group, forecast hour) tuple, by listing the group's
    /// prefix and filtering/parsing file names -- the "list" half of this
    /// crate's discovery approach (see also [`Self::content_length`], the
    /// "directly construct + HEAD-check" half, used by the sparse-fetch
    /// path once a specific member is already chosen).
    pub async fn discover_members(
        &self,
        run: RunReference,
        group: ProductGroup,
        forecast_hour: ForecastHour,
    ) -> Result<Vec<MemberKey>, GefsError> {
        let prefix = format!("{}atmos/{}/", run.run_prefix(), group.as_str());
        let keys = self.list_objects(&prefix).await?;
        let suffix = format!(
            ".t{}z.pgrb2s.0p25.{}",
            run.run_hour,
            forecast_hour.file_suffix()
        );

        let mut members: Vec<MemberKey> = keys
            .iter()
            .filter(|k| !k.ends_with(".idx"))
            .filter_map(|k| k.rsplit('/').next())
            .filter_map(|file_name| file_name.strip_suffix(suffix.as_str()))
            .filter_map(parse_member_stem)
            .collect();

        members.sort_by_key(member_sort_key);
        members.dedup();
        Ok(members)
    }

    /// Find the most recent published GEFS run (walking backward from
    /// today's UTC date, most recent run hour first) that actually has
    /// `pgrb2sp25`/`forecast_hour` objects published -- a run takes a few
    /// hours after its nominal run hour to fully publish, so "today's most
    /// recent run hour" may not exist yet at call time.
    pub async fn find_recent_run(
        &self,
        group: ProductGroup,
        forecast_hour: ForecastHour,
        lookback_days: u32,
    ) -> Result<RunReference, GefsError> {
        let (mut year, mut month, mut day) = forecast_core::time::today_utc_date();

        for day_offset in 0..=lookback_days {
            if day_offset > 0 {
                let (y, m, d) = forecast_core::time::civil_date_minus_days(year, month, day, 1);
                year = y;
                month = m;
                day = d;
            }
            for run_hour in [RunHour::H18, RunHour::H12, RunHour::H06, RunHour::H00] {
                let run = RunReference::new(year, month, day, run_hour);
                if let Ok(members) = self.discover_members(run, group, forecast_hour).await {
                    if !members.is_empty() {
                        return Ok(run);
                    }
                }
            }
        }

        Err(GefsError::NoPublishedRunFound {
            group: group.as_str().to_string(),
            lookback_days,
        })
    }

    /// Fetch a `.idx` sidecar as UTF-8 text.
    pub async fn fetch_idx_text(&self, key: &str) -> Result<String, GefsError> {
        let url = self.url_for_key(key);
        let response = self.get(&url).await?;
        response
            .text()
            .await
            .map_err(|source| GefsError::Request { url, source })
    }

    /// `HEAD` an object and return its `Content-Length`, needed only for
    /// computing the last message's byte range in a `.idx` (see
    /// [`crate::idx::byte_range`]).
    pub async fn content_length(&self, key: &str) -> Result<u64, GefsError> {
        let url = self.url_for_key(key);
        let response = self
            .http
            .head(&url)
            .send()
            .await
            .map_err(|source| GefsError::Request {
                url: url.clone(),
                source,
            })?;
        classify_status(&url, response.status())?;
        response
            .content_length()
            .ok_or_else(|| GefsError::MissingContentLength { url: url.clone() })
    }

    /// Fetch exactly the half-open byte range `[start, end)` of an object
    /// via an HTTP Range request -- the sparse-fetch technique this
    /// crate's whole design is built around: one GRIB2 message out of a
    /// tens-of-megabytes file with dozens of other fields, not the whole
    /// file.
    pub async fn fetch_byte_range(
        &self,
        key: &str,
        start: u64,
        end: u64,
    ) -> Result<Vec<u8>, GefsError> {
        let url = self.url_for_key(key);
        let response = self
            .http
            .get(&url)
            .header("Range", format!("bytes={start}-{}", end.saturating_sub(1)))
            .send()
            .await
            .map_err(|source| GefsError::Request {
                url: url.clone(),
                source,
            })?;
        classify_status(&url, response.status())?;
        let bytes = response
            .bytes()
            .await
            .map_err(|source| GefsError::Request {
                url: url.clone(),
                source,
            })?;
        Ok(bytes.to_vec())
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response, GefsError> {
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|source| GefsError::Request {
                url: url.to_string(),
                source,
            })?;
        classify_status(url, response.status())?;
        Ok(response)
    }
}

fn classify_status(url: &str, status: reqwest::StatusCode) -> Result<(), GefsError> {
    if status.is_success() {
        return Ok(());
    }
    if status.as_u16() == 404 {
        return Err(GefsError::NotFound {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    Err(GefsError::HttpStatus {
        url: url.to_string(),
        status: status.as_u16(),
    })
}

fn build_list_url(
    bucket_url: &str,
    prefix: &str,
    continuation_token: Option<&str>,
) -> Result<reqwest::Url, GefsError> {
    let mut url = reqwest::Url::parse(bucket_url).map_err(|e| GefsError::InvalidBucketUrl {
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

/// Parse a bare member file stem (`"gec00"`, `"gep01"`, `"geavg"`) into a
/// [`MemberKey`]. Returns `None` (never panics) for anything else, so an
/// unrelated file under the same prefix is silently skipped rather than
/// rejecting the whole listing.
fn parse_member_stem(stem: &str) -> Option<MemberKey> {
    if stem == "gec00" {
        return Some(MemberKey::Control);
    }
    if stem == "geavg" {
        return Some(MemberKey::Mean);
    }
    let number_part = stem.strip_prefix("gep")?;
    let number: u8 = number_part.parse().ok()?;
    if number == 0 {
        return None;
    }
    Some(MemberKey::Perturbed(number))
}

fn member_sort_key(member: &MemberKey) -> (u8, u8) {
    match member {
        MemberKey::Control => (0, 0),
        MemberKey::Perturbed(n) => (1, *n),
        MemberKey::Mean => (2, 0),
    }
}

/// Ensure a global `rustls` crypto provider is installed exactly once, per
/// process -- required because this crate builds `reqwest` with
/// `rustls-no-provider` (see `Cargo.toml`) rather than relying on a
/// feature-selected default. Idempotent: `install_default` erroring because
/// a provider is already installed (by an earlier call here, or by another
/// crate in the same process, e.g. `radar-cache`) is treated as success.
///
/// Native-only: on `wasm32`, `reqwest` never builds a `rustls`-based TLS
/// backend at all (the browser's own `fetch` does TLS), so this crate has
/// no `rustls` dependency there and this function is never called (see the
/// `cfg` on its call site in [`GefsClient::new`]).
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
        let url = build_list_url(GEFS_BUCKET_URL, "gefs.20260912/12/", None).unwrap();
        assert_eq!(url.host_str(), Some("noaa-gefs-pds.s3.amazonaws.com"));
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("list-type").map(String::as_str), Some("2"));
        assert_eq!(
            query.get("prefix").map(String::as_str),
            Some("gefs.20260912/12/")
        );
    }

    #[test]
    fn build_list_url_rejects_invalid_bucket_url() {
        let err = build_list_url("not a url", "prefix/", None).unwrap_err();
        assert!(matches!(err, GefsError::InvalidBucketUrl { .. }));
    }

    #[test]
    fn parse_member_stem_recognizes_all_three_shapes() {
        assert_eq!(parse_member_stem("gec00"), Some(MemberKey::Control));
        assert_eq!(parse_member_stem("geavg"), Some(MemberKey::Mean));
        assert_eq!(parse_member_stem("gep01"), Some(MemberKey::Perturbed(1)));
        assert_eq!(parse_member_stem("gep30"), Some(MemberKey::Perturbed(30)));
    }

    #[test]
    fn parse_member_stem_rejects_garbage() {
        assert_eq!(parse_member_stem("gep00"), None);
        assert_eq!(parse_member_stem("gepxx"), None);
        assert_eq!(parse_member_stem("something_else"), None);
        assert_eq!(parse_member_stem(""), None);
    }

    #[test]
    fn member_sort_key_orders_control_then_perturbed_then_mean() {
        let mut members = vec![
            MemberKey::Mean,
            MemberKey::Perturbed(2),
            MemberKey::Control,
            MemberKey::Perturbed(1),
        ];
        members.sort_by_key(member_sort_key);
        assert_eq!(
            members,
            vec![
                MemberKey::Control,
                MemberKey::Perturbed(1),
                MemberKey::Perturbed(2),
                MemberKey::Mean,
            ]
        );
    }
}
