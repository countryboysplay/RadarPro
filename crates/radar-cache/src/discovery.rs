//! NEXRAD Level II object discovery: list a site's objects for a given UTC
//! day via S3 `ListObjectsV2`, and filter/sort them into
//! [`DiscoveredVolume`]s.
//!
//! Deliberately separate from downloading/decoding (S04 stage brief:
//! "Separate object discovery from fetching/decoding so alternate sources
//! remain possible"): this module only ever performs `GET .../?list-type=2`
//! requests and returns metadata about what exists, never a `GetObject`.

use crate::keys::{parse_object_key, CacheDate, DiscoveredVolume};
use crate::net::{self, BackoffPolicy, FetchError};
use crate::xml::parse_list_objects_v2;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// The current public Unidata NEXRAD Level II bucket on AWS S3 (successor
/// to the retired `noaa-nexrad-level2` bucket as of September 2025).
/// Publicly readable, unauthenticated `ListObjectsV2`/`GetObject`.
pub const NEXRAD_LEVEL2_BUCKET_URL: &str = "https://unidata-nexrad-level2.s3.amazonaws.com";

/// A hard ceiling on pagination loops, purely as a defense against a
/// pathological/malicious server that keeps returning
/// `IsTruncated=true` forever: a real day's worth of objects for one site
/// is at most a few hundred keys (volumes every 4-10 minutes), i.e. at most
/// one page even at S3's default 1000-key page size, so this bound is
/// never reached by real data -- it exists only so a discovery call is
/// guaranteed to terminate rather than loop indefinitely against
/// misbehaving remote input.
const MAX_PAGES: u32 = 1000;

/// Errors from listing a site's objects.
#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("invalid bucket URL {url:?}: {message}")]
    InvalidBucketUrl { url: String, message: String },
    #[error("network request failed after {attempts} attempt(s): {source}")]
    Request {
        attempts: u32,
        #[source]
        source: reqwest::Error,
    },
    #[error("S3 returned HTTP {status} for {url} after {attempts} attempt(s)")]
    HttpStatus {
        status: u16,
        url: String,
        attempts: u32,
    },
    #[error("malformed ListObjectsV2 XML response: {0}")]
    MalformedXml(String),
    #[error("server kept paginating past {MAX_PAGES} pages without finishing; aborting")]
    TooManyPages,
    #[error("discovery was cancelled before completing")]
    Cancelled,
}

impl From<FetchError> for DiscoveryError {
    fn from(e: FetchError) -> Self {
        match e {
            FetchError::Cancelled => DiscoveryError::Cancelled,
            FetchError::Request { attempts, source } => {
                DiscoveryError::Request { attempts, source }
            }
            FetchError::Status {
                status,
                url,
                attempts,
            } => DiscoveryError::HttpStatus {
                status,
                url,
                attempts,
            },
        }
    }
}

/// A client for anonymous, unauthenticated access to a public NEXRAD Level
/// II S3 bucket (`ListObjectsV2` for discovery, `GetObject` for download --
/// see [`crate::cache::download_and_cache`]).
#[derive(Clone)]
pub struct S3Client {
    http: reqwest::Client,
    bucket_url: String,
}

impl S3Client {
    /// Build a client against an arbitrary bucket base URL (no trailing
    /// slash), e.g. for tests against a local server.
    pub fn new(bucket_url: impl Into<String>) -> reqwest::Result<Self> {
        net::ensure_crypto_provider_installed();
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()?;
        Ok(Self {
            http,
            bucket_url: bucket_url.into(),
        })
    }

    /// Build a client against the current public Unidata NEXRAD Level II
    /// bucket ([`NEXRAD_LEVEL2_BUCKET_URL`]).
    pub fn default_bucket() -> reqwest::Result<Self> {
        Self::new(NEXRAD_LEVEL2_BUCKET_URL)
    }

    pub(crate) fn http_client(&self) -> &reqwest::Client {
        &self.http
    }

    pub(crate) fn bucket_url(&self) -> &str {
        &self.bucket_url
    }

    /// List a site's Level II volume-scan objects for one UTC calendar day,
    /// sorted chronologically (oldest first). Handles `ListObjectsV2`
    /// pagination correctly rather than silently truncating at the
    /// default 1000-key page size.
    pub async fn discover_day(
        &self,
        icao: &str,
        date: CacheDate,
        cancel: &CancellationToken,
    ) -> Result<Vec<DiscoveredVolume>, DiscoveryError> {
        let prefix = date.key_prefix(icao);
        let mut continuation_token: Option<String> = None;
        let mut all_keys: Vec<String> = Vec::new();

        for _page in 0..MAX_PAGES {
            if cancel.is_cancelled() {
                return Err(DiscoveryError::Cancelled);
            }

            let url = build_list_url(&self.bucket_url, &prefix, continuation_token.as_deref())?;
            let body = net::fetch_bytes(&self.http, url, cancel, &BackoffPolicy::DEFAULT).await?;
            let page = parse_list_objects_v2(&body)?;
            all_keys.extend(page.keys);

            if !page.is_truncated {
                let mut volumes: Vec<DiscoveredVolume> = all_keys
                    .iter()
                    .filter_map(|k| parse_object_key(k))
                    .filter(|v| v.icao.eq_ignore_ascii_case(icao))
                    .collect();
                volumes.sort();
                return Ok(volumes);
            }

            continuation_token = match page.next_continuation_token {
                Some(token) => Some(token),
                None => {
                    return Err(DiscoveryError::MalformedXml(
                        "IsTruncated=true but no NextContinuationToken present".to_string(),
                    ))
                }
            };
        }

        Err(DiscoveryError::TooManyPages)
    }

    /// The most recent Level II volume for a site, if any exist for
    /// today's UTC date so far.
    pub async fn discover_latest(
        &self,
        icao: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<DiscoveredVolume>, DiscoveryError> {
        let mut volumes = self
            .discover_day(icao, CacheDate::today_utc(), cancel)
            .await?;
        Ok(volumes.pop())
    }
}

fn build_list_url(
    bucket_url: &str,
    prefix: &str,
    continuation_token: Option<&str>,
) -> Result<reqwest::Url, DiscoveryError> {
    let mut url =
        reqwest::Url::parse(bucket_url).map_err(|e| DiscoveryError::InvalidBucketUrl {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_list_url_includes_prefix_and_list_type() {
        let url = build_list_url(NEXRAD_LEVEL2_BUCKET_URL, "2026/09/12/KTLX/", None).unwrap();
        assert_eq!(
            url.host_str(),
            Some("unidata-nexrad-level2.s3.amazonaws.com")
        );
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("list-type").map(String::as_str), Some("2"));
        assert_eq!(
            query.get("prefix").map(String::as_str),
            Some("2026/09/12/KTLX/")
        );
        assert!(!query.contains_key("continuation-token"));
    }

    #[test]
    fn build_list_url_includes_continuation_token_when_present() {
        let url =
            build_list_url(NEXRAD_LEVEL2_BUCKET_URL, "2026/09/12/KTLX/", Some("tok==")).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("continuation-token").map(String::as_str),
            Some("tok==")
        );
    }

    #[test]
    fn build_list_url_rejects_invalid_bucket_url() {
        let err = build_list_url("not a url", "prefix/", None).unwrap_err();
        assert!(matches!(err, DiscoveryError::InvalidBucketUrl { .. }));
    }
}
