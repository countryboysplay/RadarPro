// Browser-native (`fetch`) client for NWS's public `/alerts/active` CAP/
// GeoJSON feed. Mirrors `src/nexrad/bucket.ts`'s shape (a small
// retry/backoff policy for transient failures, `AbortSignal` passthrough
// for cancellation) even though this is a different provider (`api.weather.gov`,
// not the NEXRAD Level II S3 bucket) with a different auth requirement --
// see below.
//
// # Required `User-Agent` header
//
// Per NWS's API usage policy and `crates/weather-alerts/tests/fixtures/
// README.md` ("Fetch requirements"), every request to `api.weather.gov`
// must send a descriptive `User-Agent` identifying the calling application
// and a contact method, or NWS may reject or rate-limit the request. This
// module sets exactly the value that crate's fixtures were themselves
// fetched with, so this app's live traffic identifies itself the same way
// its own test fixtures document.
//
// Note: some browsers historically forbade scripts from overriding the
// `User-Agent` request header (silently dropping it, not erroring) --
// this was removed from the Fetch spec's forbidden-header-name list years
// ago, and was confirmed empirically during this task's own browser
// verification (see the task's final report / `apps/web/README.md`) to
// actually reach the network in the Chrome build used here. If a future
// browser silently strips it again, requests would still succeed (NWS
// does not currently hard-reject requests missing a `User-Agent`, per its
// public docs) but without this app's own identification -- worth
// re-verifying if this ever seems to regress.
export const NWS_USER_AGENT = "RadarPro (github.com/countryboysplay/RadarPro, open-source project)";

/**
 * The nationwide active-alerts endpoint. See `useAlertPoller.ts`'s module
 * docs for the nationwide-vs-area-filtered tradeoff this app deliberately
 * chose nationwide for.
 */
export const NWS_ALERTS_ACTIVE_URL = "https://api.weather.gov/alerts/active";

export class NwsFetchError extends Error {}

/**
 * GET `url` with a small retry/backoff policy for transient failures
 * (network error, HTTP 429, HTTP 5xx) -- same shape as
 * `src/nexrad/bucket.ts`'s `fetchWithRetry`. A permanent failure (404,
 * other 4xx) is never retried.
 */
async function fetchWithRetry(url: string, init: RequestInit, maxAttempts = 4): Promise<Response> {
  let attempt = 0;
  let delayMs = 500;
  for (;;) {
    attempt += 1;
    try {
      const response = await fetch(url, init);
      if (response.ok) return response;
      const retryable = response.status === 429 || (response.status >= 500 && response.status < 600);
      if (!retryable || attempt >= maxAttempts) {
        throw new NwsFetchError(`HTTP ${response.status} ${response.statusText} for ${url} after ${attempt} attempt(s)`);
      }
    } catch (err) {
      if (err instanceof DOMException && err.name === "AbortError") throw err;
      if (attempt >= maxAttempts) {
        throw err instanceof Error ? err : new Error(String(err));
      }
    }
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(resolve, delayMs);
      init.signal?.addEventListener(
        "abort",
        () => {
          clearTimeout(timer);
          reject(new DOMException("aborted while backing off", "AbortError"));
        },
        { once: true },
      );
    });
    delayMs = Math.min(delayMs * 2, 8000);
  }
}

/**
 * Fetch the full raw `/alerts/active` response body (nationwide -- see
 * `useAlertPoller.ts`), ready to hand directly to
 * `AlertStoreHandle.ingestPoll`. Never parses the body itself -- that is
 * `weather-alerts`'s job (Rust owns CAP/GeoJSON parsing per
 * `GLOBAL_CONTRACT.md`).
 */
export async function fetchActiveAlerts(signal?: AbortSignal): Promise<string> {
  const response = await fetchWithRetry(NWS_ALERTS_ACTIVE_URL, {
    signal,
    headers: {
      "User-Agent": NWS_USER_AGENT,
      Accept: "application/geo+json",
    },
  });
  return response.text();
}
