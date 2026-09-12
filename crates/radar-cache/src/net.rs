//! Low-level HTTP fetch-with-retry-and-cancellation, shared by discovery
//! (`ListObjectsV2`) and download (`GetObject`).
//!
//! # Retry/backoff policy
//!
//! Transient failures (connection errors, timeouts, HTTP 429, and HTTP
//! 5xx) are retried with exponential backoff: [`BackoffPolicy::DEFAULT`] is
//! 5 attempts total, starting at a 500ms delay and doubling each time,
//! capped at 8s per delay. Worst case (attempts 1-5 all fail) that is
//! `500ms + 1s + 2s + 4s = 7.5s` of sleeping across 4 retries (5 attempts),
//! comfortably under a minute so a UI waiting on this never hangs
//! indefinitely. These are engineering judgment calls, not sourced from a
//! spec: chosen to give a flaky connection a handful of real chances to
//! recover without a user-visible operation ever taking anywhere close to
//! a minute.
//!
//! A permanent failure -- HTTP 404 (no such object/prefix) or any other
//! non-retryable 4xx -- is never retried: retrying the exact same request
//! against the exact same non-existent resource cannot succeed.
//!
//! # Cancellation
//!
//! Every attempt and every backoff sleep races against the caller's
//! [`CancellationToken`]; cancelling promptly abandons the in-flight
//! request or sleep rather than letting a retry loop run to completion
//! wastefully (GLOBAL_CONTRACT: "cancellation/deprioritization is required
//! for stale network/decode work").

use std::time::Duration;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// A fetch failure, before it is mapped into a higher-level
/// (`DiscoveryError`/`CacheError`) type by the caller.
#[derive(Debug, Error)]
pub(crate) enum FetchError {
    #[error("request error after {attempts} attempt(s): {source}")]
    Request {
        attempts: u32,
        #[source]
        source: reqwest::Error,
    },
    #[error("HTTP {status} for {url} after {attempts} attempt(s)")]
    Status {
        status: u16,
        url: String,
        attempts: u32,
    },
    #[error("operation was cancelled")]
    Cancelled,
}

enum Classification {
    Retry,
    Permanent,
}

fn classify_status(status: u16) -> Classification {
    if status == 429 || (500..600).contains(&status) {
        Classification::Retry
    } else {
        Classification::Permanent
    }
}

fn classify_request_error(err: &reqwest::Error) -> Classification {
    if err.is_timeout() || err.is_connect() || err.is_body() {
        Classification::Retry
    } else {
        Classification::Permanent
    }
}

/// Exponential backoff parameters. See the module docs for the chosen
/// defaults and rationale.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BackoffPolicy {
    pub base_delay: Duration,
    pub multiplier: f64,
    pub max_delay: Duration,
    pub max_attempts: u32,
}

impl BackoffPolicy {
    pub const DEFAULT: Self = Self {
        base_delay: Duration::from_millis(500),
        multiplier: 2.0,
        max_delay: Duration::from_secs(8),
        max_attempts: 5,
    };

    /// The delay to sleep *before* attempt number `attempt` (1-based; there
    /// is no delay before attempt 1).
    fn delay_before_attempt(&self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return Duration::ZERO;
        }
        let exponent = (attempt - 2) as i32;
        let scaled = self.base_delay.as_secs_f64() * self.multiplier.powi(exponent);
        Duration::from_secs_f64(scaled).min(self.max_delay)
    }
}

/// Ensure a global `rustls` crypto provider is installed exactly once, per
/// process. Required before any TLS connection because this crate builds
/// `reqwest` with `rustls-no-provider` (deliberately, to select `ring` over
/// the `aws-lc-rs` default -- see
/// `docs/adr/0008-live-data-pipeline-architecture.md`) rather than relying
/// on a feature-selected default provider.
///
/// Idempotent and safe to call from multiple client-construction call
/// sites: `install_default` errors if a provider is already installed
/// (e.g. by an earlier call, or by a caller of this crate that installed
/// its own), which this function treats as success, not a failure.
pub(crate) fn ensure_crypto_provider_installed() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// GET `url`, retrying transient failures with backoff, and return the full
/// response body. Cancellable via `cancel`.
pub(crate) async fn fetch_bytes(
    client: &reqwest::Client,
    url: reqwest::Url,
    cancel: &CancellationToken,
    policy: &BackoffPolicy,
) -> Result<Vec<u8>, FetchError> {
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;

        let delay = policy.delay_before_attempt(attempt);
        if delay.is_zero() {
            if cancel.is_cancelled() {
                return Err(FetchError::Cancelled);
            }
        } else {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(FetchError::Cancelled),
                () = tokio::time::sleep(delay) => {}
            }
        }

        let send_result = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(FetchError::Cancelled),
            result = client.get(url.clone()).send() => result,
        };

        let (retryable, err) = match send_result {
            Ok(resp) if resp.status().is_success() => {
                let body = tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(FetchError::Cancelled),
                    result = resp.bytes() => result,
                };
                match body {
                    Ok(bytes) => return Ok(bytes.to_vec()),
                    Err(source) => {
                        let retryable =
                            matches!(classify_request_error(&source), Classification::Retry);
                        (
                            retryable,
                            FetchError::Request {
                                attempts: attempt,
                                source,
                            },
                        )
                    }
                }
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                let retryable = matches!(classify_status(status), Classification::Retry);
                (
                    retryable,
                    FetchError::Status {
                        status,
                        url: url.to_string(),
                        attempts: attempt,
                    },
                )
            }
            Err(source) => {
                let retryable = matches!(classify_request_error(&source), Classification::Retry);
                (
                    retryable,
                    FetchError::Request {
                        attempts: attempt,
                        source,
                    },
                )
            }
        };

        if !retryable || attempt >= policy.max_attempts {
            return Err(err);
        }
        // Otherwise, loop back around: the top of the next iteration sleeps
        // for `delay_before_attempt(attempt + 1)` before retrying.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delays_double_up_to_the_cap() {
        let policy = BackoffPolicy::DEFAULT;
        assert_eq!(policy.delay_before_attempt(1), Duration::ZERO);
        assert_eq!(policy.delay_before_attempt(2), Duration::from_millis(500));
        assert_eq!(policy.delay_before_attempt(3), Duration::from_millis(1000));
        assert_eq!(policy.delay_before_attempt(4), Duration::from_millis(2000));
        assert_eq!(policy.delay_before_attempt(5), Duration::from_millis(4000));
        // A hypothetical 6th attempt would compute to 8s uncapped and stay
        // at the 8s cap.
        assert_eq!(policy.delay_before_attempt(6), Duration::from_secs(8));
    }

    #[test]
    fn total_worst_case_backoff_is_well_under_a_minute() {
        let policy = BackoffPolicy::DEFAULT;
        let total: Duration = (1..=policy.max_attempts)
            .map(|a| policy.delay_before_attempt(a))
            .sum();
        assert!(
            total < Duration::from_secs(60),
            "total backoff {total:?} should stay well under a minute"
        );
    }

    #[test]
    fn classify_status_marks_5xx_and_429_as_retryable() {
        assert!(matches!(classify_status(500), Classification::Retry));
        assert!(matches!(classify_status(503), Classification::Retry));
        assert!(matches!(classify_status(429), Classification::Retry));
    }

    #[test]
    fn classify_status_marks_404_and_other_4xx_as_permanent() {
        assert!(matches!(classify_status(404), Classification::Permanent));
        assert!(matches!(classify_status(403), Classification::Permanent));
        assert!(matches!(classify_status(400), Classification::Permanent));
    }
}
