// S09b: resolving Rainbow's `{snapshot}`/`{forecast_time}` tile-URL
// placeholders to concrete values.
//
// MapLibre raster sources only ever substitute `{z}/{x}/{y}` in a tile URL
// template -- `{snapshot}` and `{forecast_time}` are this API's own scheme
// (doc.rainbow.ai), not standard XYZ, so they must be resolved to concrete
// numbers *before* the URL template is handed to MapLibre. This module does
// that resolution; `useRainbowOverlay` is the only caller.
import { RAINBOW_API_BASE } from "./config";

/** `forecast_time=0` is "current conditions" -- the only value this stage
 * uses (a full lead-time timeline scrubber is S09/later scope per the
 * stage file). */
export const RAINBOW_FORECAST_TIME_CURRENT = 0;

/** A fixed, always-in-range tile coordinate used only to *probe* whether a
 * candidate snapshot is published -- z=0 is valid for every Rainbow tile
 * endpoint (precip's zoom range is `[0,12]`) and is the sole tile at that
 * zoom, so it always exists as a coordinate regardless of the region a key
 * has access to. What we care about is the HTTP status the API returns for
 * this (snapshot, forecast_time) pair, not the pixels themselves. */
const PROBE_Z = 0;
const PROBE_X = 0;
const PROBE_Y = 0;

/** Rainbow snapshots are epoch-UTC-seconds aligned to a 10-minute boundary
 * (doc.rainbow.ai, verified 2026-09-13). */
const SNAPSHOT_STEP_SECONDS = 600;

/** Bounded fallback depth for "latest snapshot not published yet": a
 * "handful" of 10-minute steps per the stage file, not an unbounded walk.
 * 6 steps = 1 hour back, comfortably inside the documented 2-hour access
 * window while keeping a single toggle-on action to a small, bounded number
 * of requests even in the worst case (every boundary reporting confirmed
 * 404 until this limit is hit). */
const MAX_BOUNDARY_FALLBACK_STEPS = 6;

/** Retries applied to a *single* boundary when the probe fails with a
 * network/5xx error -- never on a confirmed 404. Mirrors the discipline
 * learned from provider-gefs/provider-hrrr's run-discovery loops
 * ([[radarpro-discovery-loop-transient-error-swallowing]]): a transient
 * failure is not confirmed absence, and swallowing it as "try an older
 * boundary" would spuriously walk past a snapshot that actually exists. */
const ERROR_RETRY_ATTEMPTS = 3;
const ERROR_RETRY_BASE_DELAY_MS = 300;

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Most recent 10-minute-aligned epoch-seconds boundary at or before `now`.
 * `now` is injectable only for tests -- real callers always use the
 * default (`Date.now()`). */
export function latestTenMinuteBoundaryEpoch(now: number = Date.now()): number {
  const nowSeconds = Math.floor(now / 1000);
  return nowSeconds - (nowSeconds % SNAPSHOT_STEP_SECONDS);
}

function precipTileUrl(snapshot: number, forecastTime: number, apiKey: string): string {
  return (
    `${RAINBOW_API_BASE}/tiles/v1/precip/${snapshot}/${forecastTime}/${PROBE_Z}/${PROBE_X}/${PROBE_Y}` +
    `?token=${encodeURIComponent(apiKey)}`
  );
}

/** Build the full tile URL *template* MapLibre's raster source consumes --
 * `{z}/{x}/{y}` left as literal MapLibre placeholders, `snapshot`/
 * `forecast_time` already resolved to concrete values. Regional precip
 * endpoint per the stage file (not `precip-global`). */
export function buildRainbowPrecipTileUrlTemplate(
  snapshot: number,
  forecastTime: number,
  apiKey: string,
): string {
  return (
    `${RAINBOW_API_BASE}/tiles/v1/precip/${snapshot}/${forecastTime}/{z}/{x}/{y}` +
    `?token=${encodeURIComponent(apiKey)}`
  );
}

type ProbeResult = "published" | "not-published" | "transient-error";

/** One HTTP probe of a single (snapshot, forecast_time) pair, never
 * retried by this function itself -- retry/fallback policy lives in
 * `resolveRainbowSnapshot` so it can apply the "retry error, fall back only
 * on confirmed absence" rule across probes, not within one. */
async function probeOnce(snapshot: number, forecastTime: number, apiKey: string, signal?: AbortSignal): Promise<ProbeResult> {
  try {
    const response = await fetch(precipTileUrl(snapshot, forecastTime, apiKey), { method: "GET", signal });
    if (response.status === 404) return "not-published"; // confirmed absence -- the only case we step back for.
    if (response.ok) return "published";
    // Any other status (401/403/429/5xx/...) is a real failure, not
    // confirmed absence -- see the module doc comment.
    return "transient-error";
  } catch {
    // Network failure (offline, DNS, CORS, timeout, aborted) -- same
    // "not confirmed absence" treatment as a non-404 HTTP error.
    return "transient-error";
  }
}

/** Probe one boundary, retrying only `"transient-error"` outcomes (never a
 * confirmed `"not-published"`) up to `ERROR_RETRY_ATTEMPTS` times with a
 * linear backoff. */
async function probeBoundaryWithRetry(
  snapshot: number,
  forecastTime: number,
  apiKey: string,
  signal?: AbortSignal,
): Promise<ProbeResult> {
  let last: ProbeResult = "transient-error";
  for (let attempt = 0; attempt < ERROR_RETRY_ATTEMPTS; attempt++) {
    last = await probeOnce(snapshot, forecastTime, apiKey, signal);
    if (last !== "transient-error") return last;
    if (attempt < ERROR_RETRY_ATTEMPTS - 1) {
      await delay(ERROR_RETRY_BASE_DELAY_MS * (attempt + 1));
    }
  }
  return last;
}

export type RainbowSnapshotResolution =
  | { ok: true; snapshot: number }
  | { ok: false; reason: string };

/**
 * Resolve the newest usable `snapshot` value for the precip tile endpoint.
 *
 * Tries the latest 10-minute boundary first (the common case: already
 * published). On a *confirmed* 404 there, steps back one boundary at a time
 * (up to `MAX_BOUNDARY_FALLBACK_STEPS`) -- "not published yet" is expected
 * and normal for the newest boundary. On a network/5xx error, retries the
 * *same* boundary in place (`probeBoundaryWithRetry`) and, if it never
 * resolves, gives up with an error rather than guessing an older boundary
 * might work -- a transient failure is never treated as confirmed absence
 * (see module doc comment).
 */
export async function resolveRainbowSnapshot(
  apiKey: string,
  forecastTime: number = RAINBOW_FORECAST_TIME_CURRENT,
  signal?: AbortSignal,
): Promise<RainbowSnapshotResolution> {
  let candidate = latestTenMinuteBoundaryEpoch();
  for (let step = 0; step <= MAX_BOUNDARY_FALLBACK_STEPS; step++) {
    const result = await probeBoundaryWithRetry(candidate, forecastTime, apiKey, signal);
    if (result === "published") return { ok: true, snapshot: candidate };
    if (result === "not-published") {
      candidate -= SNAPSHOT_STEP_SECONDS;
      continue;
    }
    // "transient-error" surviving every in-place retry: an unresolved
    // network/API problem, not evidence the snapshot is missing.
    return { ok: false, reason: "could not reach Rainbow Weather (network or API error) -- not a confirmed missing snapshot" };
  }
  return {
    ok: false,
    reason: `no published Rainbow snapshot found in the last ${MAX_BOUNDARY_FALLBACK_STEPS * (SNAPSHOT_STEP_SECONDS / 60)} minutes`,
  };
}
