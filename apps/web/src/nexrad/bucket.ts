// Browser-native (`fetch`/`DOMParser`, no SDK) client for the current public
// NOAA Level II NEXRAD bucket: discovery (`ListObjectsV2`) and download
// (`GetObject`), kept as separate operations per S04's brief ("Separate
// object discovery from fetching/decoding so alternate sources remain
// possible") -- mirroring `crates/radar-cache/src/discovery.rs` and
// `net.rs`'s shape/reasoning, reimplemented here because that crate is
// native/`reqwest`-coupled and cannot run in a browser bundle.
//
// # CORS -- empirically verified, not assumed
//
// Before building this, a real Chrome instance (via `puppeteer-core`, not
// `curl`/Node) was driven to `fetch()` this bucket directly from a page
// served on `http://localhost`, for both operations this module performs:
//   - `GET /?list-type=2&prefix=...` (discovery): succeeded, HTTP 200, full
//     XML body readable from the page.
//   - `GET /<key>` with a `Range` header (download; the `Range` header
//     forces a CORS preflight `OPTIONS`, unlike a plain `GET`): succeeded,
//     HTTP 206, body bytes readable from the page.
// The bucket's CORS policy allows both cross-origin request shapes this
// module needs from an arbitrary origin -- no same-origin dev-server proxy
// or backend relay is required for this stage. If that policy ever changes,
// these `fetch()` calls will start rejecting with a `TypeError: Failed to
// fetch` (the browser deliberately gives no further detail on a CORS
// failure) and this module's retry logic will treat that as a network
// error like any other.

import { parseListObjectsV2 } from "./xml";
import { parseObjectKey, todayUtc, keyPrefix, compareDiscoveredVolumes, type DiscoveredVolume } from "./keys";

/**
 * The current public Unidata NEXRAD Level II bucket on AWS S3 (successor to
 * the retired `noaa-nexrad-level2` bucket as of September 2025). Publicly
 * readable, unauthenticated `ListObjectsV2`/`GetObject`. Same bucket
 * `crates/radar-cache`'s `discovery.rs` uses.
 */
export const NEXRAD_LEVEL2_BUCKET_URL = "https://unidata-nexrad-level2.s3.amazonaws.com";

/**
 * Hard ceiling on pagination loops -- purely a defense against a
 * pathological/misbehaving server that keeps returning
 * `IsTruncated=true` forever. A real day's objects for one site is at most
 * a few hundred keys (volumes every 4-10 minutes), i.e. one page even at
 * S3's default 1000-key page size; this is never reached by real data. Same
 * rationale/value as `radar-cache/src/discovery.rs`'s `MAX_PAGES`.
 */
const MAX_PAGES = 1000;

export class DiscoveryError extends Error {}

// Cancellation is surfaced as the standard `DOMException("AbortError")` a
// caller's own `AbortController` produces -- not re-wrapped into a
// RadarPro-specific error type, since `AbortSignal` is already the
// idiomatic, widely-understood browser cancellation primitive.

function buildListUrl(prefix: string, continuationToken: string | null): string {
  const url = new URL(NEXRAD_LEVEL2_BUCKET_URL + "/");
  url.searchParams.set("list-type", "2");
  url.searchParams.set("prefix", prefix);
  if (continuationToken) url.searchParams.set("continuation-token", continuationToken);
  return url.toString();
}

/**
 * GET `url` with a small retry/backoff policy for transient failures
 * (network error, HTTP 429, HTTP 5xx), matching the spirit (not the exact
 * constants) of `radar-cache/src/net.rs`: a handful of quick retries so a
 * flaky connection gets a real chance to recover, without ever blocking a
 * UI for anywhere close to a minute. A permanent failure (404, other 4xx)
 * is never retried.
 */
async function fetchWithRetry(url: string, init: RequestInit, maxAttempts = 4): Promise<Response> {
  let attempt = 0;
  let delayMs = 500;
  for (;;) {
    attempt += 1;
    try {
      const response = await fetch(url, init);
      if (response.ok || response.status === 206) return response;
      const retryable = response.status === 429 || (response.status >= 500 && response.status < 600);
      if (!retryable || attempt >= maxAttempts) {
        throw new Error(`HTTP ${response.status} ${response.statusText} for ${url} after ${attempt} attempt(s)`);
      }
    } catch (err) {
      // AbortError propagates immediately -- cancellation must never be
      // retried or masked as a transient network failure.
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
 * List a site's Level II volume-scan objects for one UTC calendar day,
 * sorted chronologically (oldest first). Handles `ListObjectsV2` pagination
 * (does not silently truncate at the default 1000-key page size).
 */
export async function discoverDay(
  icao: string,
  date: { year: number; month: number; day: number },
  signal?: AbortSignal,
): Promise<DiscoveredVolume[]> {
  const prefix = keyPrefix(date, icao);
  let continuationToken: string | null = null;
  const allKeys: string[] = [];

  for (let page = 0; page < MAX_PAGES; page++) {
    const response = await fetchWithRetry(buildListUrl(prefix, continuationToken), { signal });
    const body = await response.text();
    const parsed = parseListObjectsV2(body);
    allKeys.push(...parsed.keys);

    if (!parsed.isTruncated) {
      const volumes = allKeys
        .map(parseObjectKey)
        .filter((v): v is DiscoveredVolume => v !== null && v.icao.toUpperCase() === icao.toUpperCase());
      volumes.sort(compareDiscoveredVolumes);
      return volumes;
    }

    if (!parsed.nextContinuationToken) {
      throw new DiscoveryError("IsTruncated=true but no NextContinuationToken present in ListObjectsV2 response");
    }
    continuationToken = parsed.nextContinuationToken;
  }

  throw new DiscoveryError(`server kept paginating past ${MAX_PAGES} pages without finishing; aborting`);
}

/** The most recent Level II volume for a site, if any exist for today's UTC date so far. */
export async function discoverLatest(icao: string, signal?: AbortSignal): Promise<DiscoveredVolume | null> {
  const volumes = await discoverDay(icao, todayUtc(), signal);
  return volumes.length > 0 ? volumes[volumes.length - 1] : null;
}

/** Download one object's full bytes (a raw Archive II Level II volume). */
export async function downloadVolume(key: string, signal?: AbortSignal): Promise<Uint8Array> {
  const url = `${NEXRAD_LEVEL2_BUCKET_URL}/${key}`;
  const response = await fetchWithRetry(url, { signal });
  const buffer = await response.arrayBuffer();
  return new Uint8Array(buffer);
}
