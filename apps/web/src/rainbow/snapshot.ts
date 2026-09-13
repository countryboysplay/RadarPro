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

/**
 * HTTP statuses this endpoint returns for a snapshot that is confirmed not
 * (yet) available -- the case `resolveRainbowSnapshot` steps back an older
 * boundary for, as opposed to a real transient failure it retries in place.
 *
 * `404` is the documented "missing" case. `400` was found live (via the
 * desktop CORS-bypass path, which is the first time this project ever saw
 * real, distinguishable HTTP responses from this endpoint instead of every
 * attempt failing identically to a browser CORS block): requesting the
 * newest 10-minute boundary before Rainbow has published it returns `400`
 * with body `{"message":"Invalid timestamp"}`, not `404` -- confirmed with
 * a real key against the real endpoint, not assumed. Before this fix, that
 * 400 was misclassified as `"transient-error"`, which
 * `probeBoundaryWithRetry` retries a few times *in place* and then gives up
 * entirely rather than stepping back -- so the whole resolution failed with
 * "unavailable" even though an older, genuinely published boundary existed
 * one step back. This was invisible in the browser path, where every
 * attempt already fails identically to CORS regardless of status code (see
 * `browserProbeTile`'s doc comment).
 *
 * The only variable part of this endpoint's request shape is the snapshot
 * timestamp (z/x/y come from a fixed, always-valid tile computation;
 * forecast_time is always `RAINBOW_FORECAST_TIME_CURRENT`), so a 400 here
 * means "invalid timestamp" in practice -- not response-body-sniffed since
 * `rainbow_probe_tile` (the desktop path) only returns a status code, not a
 * body, to keep that command minimal.
 */
export function isConfirmedNotYetAvailable(status: number): boolean {
  return status === 404 || status === 400;
}

/** Fixed probe zoom level for {@link probeTileForSite}. z=0's sole tile
 * covers the whole world, which isn't tied to any real, meaningful location
 * -- z=5 instead gives ~1,225km-square tiles at the equator: coarse enough
 * that the selected site is essentially guaranteed to fall inside whatever
 * regional coverage area contains it (this is only a does-a-snapshot-exist
 * probe, not a check that interesting weather is visible), while still
 * being a real coordinate derived from the site's actual lat/lon rather
 * than an arbitrary, location-independent corner tile. */
const PROBE_ZOOM = 5;

/** Standard slippy-map (Web Mercator) lon/lat -> tile x/y formula at a
 * fixed zoom -- see e.g. the OSM wiki's "Slippy map tilenames" page. Result
 * is clamped into `[0, 2^zoom - 1]` defensively (a lat/lon exactly at a
 * pole or the antimeridian could otherwise round to an out-of-range index),
 * though no real WSR-88D site is anywhere near either edge case. */
function lonLatToTile(lon: number, lat: number, zoom: number): { z: number; x: number; y: number } {
  const latRad = (lat * Math.PI) / 180;
  const n = 2 ** zoom;
  const x = Math.floor(((lon + 180) / 360) * n);
  const y = Math.floor(((1 - Math.log(Math.tan(latRad) + 1 / Math.cos(latRad)) / Math.PI) / 2) * n);
  const clamp = (v: number) => Math.min(Math.max(v, 0), n - 1);
  return { z: zoom, x: clamp(x), y: clamp(y) };
}

/** The probe tile coordinate for a given site: a real, meaningful location
 * (the currently selected radar site's own lat/lon) instead of the old
 * always-`(0,0,0)` whole-earth tile, which wasn't tied to any real,
 * populated location. `useRainbowOverlay` is the only caller -- see its
 * doc comment for how the selected site reaches this module. */
export function probeTileForSite(lat: number, lon: number): { z: number; x: number; y: number } {
  return lonLatToTile(lon, lat, PROBE_ZOOM);
}

/** Rainbow snapshots are epoch-UTC-seconds aligned to a 10-minute boundary
 * (doc.rainbow.ai, verified 2026-09-13). */
const SNAPSHOT_STEP_SECONDS = 600;

/** Bounded fallback depth for "latest snapshot not published yet": a
 * "handful" of 10-minute steps per the stage file, not an unbounded walk.
 * 6 steps = 1 hour back, comfortably inside the documented 2-hour access
 * window while keeping a single toggle-on action to a small, bounded number
 * of requests even in the worst case (every boundary reporting confirmed
 * not-yet-available -- see {@link isConfirmedNotYetAvailable} -- until this
 * limit is hit). */
const MAX_BOUNDARY_FALLBACK_STEPS = 6;

/** Retries applied to a *single* boundary when the probe fails with a
 * network/5xx error -- never on a confirmed not-yet-available status (see
 * {@link isConfirmedNotYetAvailable}). Mirrors the discipline
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

function precipTileUrl(
  snapshot: number,
  forecastTime: number,
  tile: { z: number; x: number; y: number },
  apiKey: string,
): string {
  return (
    `${RAINBOW_API_BASE}/tiles/v1/precip/${snapshot}/${forecastTime}/${tile.z}/${tile.x}/${tile.y}` +
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

export type ProbeResult = "published" | "not-published" | "transient-error";

/** Transport signature for a single boundary probe -- the browser path's
 * default is {@link browserProbeTile} (a plain `fetch`); the desktop path
 * (`useRainbowOverlay`, via `apps/web/src/rainbow/desktopTiles.ts`) injects
 * one that calls the native `rainbow_probe_tile` Tauri command instead, to
 * route around `api.rainbow.ai`'s total lack of CORS headers (see that
 * module's doc comment) -- everything else in this file (the retry/
 * fallback policy below) is unchanged either way, since only the transport
 * differs. */
export type ProbeFn = (
  snapshot: number,
  forecastTime: number,
  tile: { z: number; x: number; y: number },
  apiKey: string,
  signal?: AbortSignal,
) => Promise<ProbeResult>;

/** One HTTP probe of a single (snapshot, forecast_time) pair via a plain
 * browser `fetch`, never retried by this function itself -- retry/fallback
 * policy lives in `resolveRainbowSnapshot` so it can apply the "retry
 * error, fall back only on confirmed absence" rule across probes, not
 * within one. The default {@link ProbeFn} for the browser (non-desktop)
 * path. */
async function browserProbeTile(
  snapshot: number,
  forecastTime: number,
  tile: { z: number; x: number; y: number },
  apiKey: string,
  signal?: AbortSignal,
): Promise<ProbeResult> {
  try {
    const response = await fetch(precipTileUrl(snapshot, forecastTime, tile, apiKey), { method: "GET", signal });
    if (isConfirmedNotYetAvailable(response.status)) return "not-published"; // confirmed absence -- see that helper's doc comment (404 documented, 400 found live).
    if (response.ok) return "published";
    // Any other status (401/403/429/5xx/...) is a real failure, not
    // confirmed absence -- see the module doc comment.
    return "transient-error";
  } catch {
    // Network failure (offline, DNS, CORS, timeout, aborted) -- same
    // "not confirmed absence" treatment as a non-404 HTTP error. In the
    // plain browser this is, in practice, *always* how api.rainbow.ai's
    // missing CORS headers show up -- see the module doc comment on
    // `ProbeFn`; this catch is not a bug to chase further here.
    return "transient-error";
  }
}

/** Probe one boundary, retrying only `"transient-error"` outcomes (never a
 * confirmed `"not-published"`) up to `ERROR_RETRY_ATTEMPTS` times with a
 * linear backoff. */
async function probeBoundaryWithRetry(
  snapshot: number,
  forecastTime: number,
  tile: { z: number; x: number; y: number },
  apiKey: string,
  probeFn: ProbeFn,
  signal?: AbortSignal,
): Promise<ProbeResult> {
  let last: ProbeResult = "transient-error";
  for (let attempt = 0; attempt < ERROR_RETRY_ATTEMPTS; attempt++) {
    last = await probeFn(snapshot, forecastTime, tile, apiKey, signal);
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
 * published). On a *confirmed* not-yet-available response there (404, or
 * the 400 `{"message":"Invalid timestamp"}` this endpoint actually returns
 * for the newest boundary -- see {@link isConfirmedNotYetAvailable}), steps
 * back one boundary at a time (up to `MAX_BOUNDARY_FALLBACK_STEPS`) -- "not
 * published yet" is expected and normal for the newest boundary. On a
 * network/5xx error, retries the
 * *same* boundary in place (`probeBoundaryWithRetry`) and, if it never
 * resolves, gives up with an error rather than guessing an older boundary
 * might work -- a transient failure is never treated as confirmed absence
 * (see module doc comment).
 *
 * `tile` is the probe coordinate (see {@link probeTileForSite} -- normally
 * derived from the currently selected radar site's lat/lon, not an
 * arbitrary/location-independent corner). `probeFn` defaults to
 * {@link browserProbeTile}; the desktop path passes one that routes through
 * the native Tauri command instead (see {@link ProbeFn}'s doc comment).
 */
export async function resolveRainbowSnapshot(
  apiKey: string,
  tile: { z: number; x: number; y: number },
  forecastTime: number = RAINBOW_FORECAST_TIME_CURRENT,
  signal?: AbortSignal,
  probeFn: ProbeFn = browserProbeTile,
): Promise<RainbowSnapshotResolution> {
  let candidate = latestTenMinuteBoundaryEpoch();
  for (let step = 0; step <= MAX_BOUNDARY_FALLBACK_STEPS; step++) {
    const result = await probeBoundaryWithRetry(candidate, forecastTime, tile, apiKey, probeFn, signal);
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
