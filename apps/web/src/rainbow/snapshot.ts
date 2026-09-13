// S09b (precip-only) / S09d Part A (all four Tiles API layers): resolving
// Rainbow's `{snapshot}`/`{forecast_time}` tile-URL placeholders to concrete
// values.
//
// MapLibre raster sources only ever substitute `{z}/{x}/{y}` in a tile URL
// template -- `{snapshot}` and `{forecast_time}` are this API's own scheme
// (doc.rainbow.ai), not standard XYZ, so they must be resolved to concrete
// numbers *before* the URL template is handed to MapLibre. This module does
// that resolution; `useRainbowOverlay` is the only caller.
import { RAINBOW_API_BASE } from "./config";
import { rainbowTileLayerInfo, type RainbowTileLayer } from "./types";

/** `forecast_time=0` is "current conditions" -- the default this stage's UI
 * opens on (a labeled dropdown, `RAINBOW_FORECAST_TIME_STEPS` in
 * `types.ts`, lets the user pick any documented lead time up to 4h for
 * `precip`/`precip-global`). */
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
 * a real key against the real endpoint, not assumed (originally observed
 * against the `precip` layer; treated as applying to all four layers, which
 * share the same underlying tile-serving behavior per KB §4.2-§4.5, until
 * proven otherwise). Before this fix, that 400 was misclassified as
 * `"transient-error"`, which `probeBoundaryWithRetry` retries a few times
 * *in place* and then gives up entirely rather than stepping back -- so the
 * whole resolution failed with "unavailable" even though an older, genuinely
 * published boundary existed one step back. This was invisible in the
 * browser path, where every attempt already fails identically to CORS
 * regardless of status code (see `browserProbeTile`'s doc comment).
 *
 * The only variable part of this endpoint's request shape is the snapshot
 * timestamp (z/x/y come from a fixed, always-valid tile computation;
 * forecast_time/color/coverage/use_precip_type are all caller-controlled but
 * fixed for the duration of one resolution), so a 400 here means "invalid
 * timestamp" in practice -- not response-body-sniffed since
 * `rainbow_probe_tile` (the desktop path) only returns a status code, not a
 * body, to keep that command minimal.
 */
export function isConfirmedNotYetAvailable(status: number): boolean {
  return status === 404 || status === 400;
}

/** Fixed probe zoom level for {@link probeTileForSite}, clamped to the
 * current layer's own zoom ceiling (`clouds`/`radars` stop at 7, KB
 * §4.4/§4.5) -- 5 already sits under both documented ceilings (7 and 12),
 * so this clamp is a no-op today, kept only so a future change to either
 * number can never silently produce an out-of-range probe tile. z=0's sole
 * tile covers the whole world, which isn't tied to any real, meaningful
 * location -- z=5 instead gives ~1,225km-square tiles at the equator:
 * coarse enough that the selected site is essentially guaranteed to fall
 * inside whatever regional coverage area contains it (this is only a
 * does-a-snapshot-exist probe, not a check that interesting weather is
 * visible), while still being a real coordinate derived from the site's
 * actual lat/lon rather than an arbitrary, location-independent corner
 * tile. */
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
 * doc comment for how the selected site reaches this module. `layer`
 * clamps the probe zoom to that layer's own ceiling (see `PROBE_ZOOM`'s doc
 * comment -- a no-op today, since 5 already fits under both 7 and 12). */
export function probeTileForSite(lat: number, lon: number, layer: RainbowTileLayer): { z: number; x: number; y: number } {
  const zoom = Math.min(PROBE_ZOOM, rainbowTileLayerInfo(layer).maxZoom);
  return lonLatToTile(lon, lat, zoom);
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

/** The layer-independent tile query params a caller picks (KB §4.2-§4.5) --
 * `color`/`coverage`/`use_precip_type` are all only *sometimes* applicable
 * (see `RainbowTileLayerInfo`); this type carries whatever the UI currently
 * holds, and {@link normalizeRainbowTileParams} is what actually decides
 * which of them apply to a given layer. */
export interface RainbowTileParams {
  color: string;
  coverage: boolean;
  usePrecipType: boolean;
}

interface NormalizedRainbowTileParams {
  forecastTime: number;
  color: string | undefined;
  coverage: boolean;
  usePrecipType: boolean;
}

/** Reduces a possibly-inapplicable `(forecastTime, params)` pair down to
 * what actually applies to `layer`, per KB §4.2-§4.5 -- e.g. `clouds` never
 * gets a `color`/`coverage`/`forecast_time` no matter what the dropdowns
 * currently hold. `RainbowToggle` already hides those controls per-layer,
 * but a state update and a resolve/build-URL call can still race (the user
 * flips the layer dropdown while a resolution from the *previous* layer
 * selection is still in flight), so this is enforced here too, at the one
 * place both the probe path and the final tile-URL builder both go through
 * -- not just visually in the UI. Mirrors `rainbow.rs`'s `RainbowLayer`
 * `supports_*` methods one-for-one. */
export function normalizeRainbowTileParams(
  layer: RainbowTileLayer,
  forecastTime: number,
  params: RainbowTileParams,
): NormalizedRainbowTileParams {
  const info = rainbowTileLayerInfo(layer);
  return {
    forecastTime: info.supportsForecastTime ? forecastTime : 0,
    color: info.supportsColorCoverage ? params.color : undefined,
    coverage: info.supportsColorCoverage ? params.coverage : false,
    usePrecipType: info.supportsUsePrecipType ? params.usePrecipType : false,
  };
}

/** Builds the `/tiles/v1/<layer>/<snapshot>/[<forecast_time>/]<tileSegment>`
 * path plus its layer-appropriate query string. `tileSegment` is either a
 * concrete `{z}/{x}/{y}` (probing) or the literal string `"{z}/{x}/{y}"`
 * left for MapLibre to substitute (the final tile URL template) -- both
 * callers below need the exact same path/query construction, just a
 * different tail. */
function rainbowTileUrlWithTileSegment(
  layer: RainbowTileLayer,
  snapshot: number,
  tileSegment: string,
  normalized: NormalizedRainbowTileParams,
  apiKey: string,
): string {
  const info = rainbowTileLayerInfo(layer);
  let path = `${RAINBOW_API_BASE}/tiles/v1/${layer}/${snapshot}/`;
  if (info.supportsForecastTime) path += `${normalized.forecastTime}/`;
  path += tileSegment;

  const query = new URLSearchParams();
  if (normalized.color !== undefined) query.set("color", normalized.color);
  if (normalized.coverage) query.set("coverage", "1");
  if (normalized.usePrecipType) query.set("use_precip_type", "1");
  query.set("token", apiKey);
  return `${path}?${query.toString()}`;
}

function rainbowTileUrl(
  layer: RainbowTileLayer,
  snapshot: number,
  tile: { z: number; x: number; y: number },
  normalized: NormalizedRainbowTileParams,
  apiKey: string,
): string {
  return rainbowTileUrlWithTileSegment(layer, snapshot, `${tile.z}/${tile.x}/${tile.y}`, normalized, apiKey);
}

/** Build the full tile URL *template* MapLibre's raster source consumes --
 * `{z}/{x}/{y}` left as literal MapLibre placeholders, everything else
 * (`snapshot`, `forecast_time`, `color`, `coverage`, `use_precip_type`)
 * already resolved to concrete, layer-appropriate values via
 * {@link normalizeRainbowTileParams}. Generalizes S09b's original
 * precip-only `buildRainbowPrecipTileUrlTemplate` to all four documented
 * layers (KB §4.2-§4.5). */
export function buildRainbowTileUrlTemplate(
  layer: RainbowTileLayer,
  snapshot: number,
  forecastTime: number,
  params: RainbowTileParams,
  apiKey: string,
): string {
  const normalized = normalizeRainbowTileParams(layer, forecastTime, params);
  return rainbowTileUrlWithTileSegment(layer, snapshot, "{z}/{x}/{y}", normalized, apiKey);
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
  layer: RainbowTileLayer,
  snapshot: number,
  forecastTime: number,
  tile: { z: number; x: number; y: number },
  params: RainbowTileParams,
  apiKey: string,
  signal?: AbortSignal,
) => Promise<ProbeResult>;

/** One HTTP probe of a single (layer, snapshot, forecast_time, ...params)
 * combination via a plain browser `fetch`, never retried by this function
 * itself -- retry/fallback policy lives in `resolveRainbowSnapshot` so it
 * can apply the "retry error, fall back only on confirmed absence" rule
 * across probes, not within one. The default {@link ProbeFn} for the
 * browser (non-desktop) path. */
async function browserProbeTile(
  layer: RainbowTileLayer,
  snapshot: number,
  forecastTime: number,
  tile: { z: number; x: number; y: number },
  params: RainbowTileParams,
  apiKey: string,
  signal?: AbortSignal,
): Promise<ProbeResult> {
  try {
    const normalized = normalizeRainbowTileParams(layer, forecastTime, params);
    const url = rainbowTileUrl(layer, snapshot, tile, normalized, apiKey);
    const response = await fetch(url, { method: "GET", signal });
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
  layer: RainbowTileLayer,
  snapshot: number,
  forecastTime: number,
  tile: { z: number; x: number; y: number },
  params: RainbowTileParams,
  apiKey: string,
  probeFn: ProbeFn,
  signal?: AbortSignal,
): Promise<ProbeResult> {
  let last: ProbeResult = "transient-error";
  for (let attempt = 0; attempt < ERROR_RETRY_ATTEMPTS; attempt++) {
    last = await probeFn(layer, snapshot, forecastTime, tile, params, apiKey, signal);
    if (last !== "transient-error") return last;
    if (attempt < ERROR_RETRY_ATTEMPTS - 1) {
      await delay(ERROR_RETRY_BASE_DELAY_MS * (attempt + 1));
    }
  }
  return last;
}

/** Optional transport for `GET /tiles/v1/snapshot?layer=<layer>` (KB §4.1),
 * used only to seed {@link resolveRainbowSnapshot}'s *starting* candidate
 * boundary in place of the local `latestTenMinuteBoundaryEpoch()` guess --
 * the existing probe-and-step-back loop still runs from that starting
 * point, since this endpoint's answer can still be momentarily ahead of
 * what a tile request will actually serve. Returns `null` (or throws) to
 * fall back to the local guess. No browser implementation exists: this
 * endpoint sits behind the same CORS-less gateway as every other Tiles API
 * route (see `radarpro-rainbow-cors-no-browser-support`), so a browser
 * `fetch` here would only ever add a doomed round trip before falling back
 * anyway -- only the desktop path (`desktopTiles.ts`'s `desktopGetSnapshot`)
 * supplies one. */
export type SnapshotStartFn = (layer: RainbowTileLayer, apiKey: string, signal?: AbortSignal) => Promise<number | null>;

async function resolveStartingCandidate(
  layer: RainbowTileLayer,
  apiKey: string,
  signal: AbortSignal | undefined,
  snapshotStartFn: SnapshotStartFn | undefined,
): Promise<number> {
  if (snapshotStartFn) {
    try {
      const snapshot = await snapshotStartFn(layer, apiKey, signal);
      if (snapshot !== null) return snapshot;
    } catch {
      // Not confirmed absence, just an unavailable shortcut -- fall through
      // to the local guess below, which the ordinary probe-and-step-back
      // loop will then confirm or correct.
    }
  }
  return latestTenMinuteBoundaryEpoch();
}

export type RainbowSnapshotResolution = { ok: true; snapshot: number } | { ok: false; reason: string };

/**
 * Resolve the newest usable `snapshot` value for a given layer's tile
 * endpoint.
 *
 * Starting candidate: the documented `GET /tiles/v1/snapshot?layer=<layer>`
 * value when `snapshotStartFn` is supplied and succeeds (KB §4.1 -- the
 * desktop path only, see that type's doc comment), else the latest local
 * 10-minute-boundary guess. Either way, the same probe-and-step-back loop
 * then runs: on a *confirmed* not-yet-available response (404, or the 400
 * `{"message":"Invalid timestamp"}` this endpoint actually returns for the
 * newest boundary -- see {@link isConfirmedNotYetAvailable}), steps back one
 * boundary at a time (up to `MAX_BOUNDARY_FALLBACK_STEPS`) -- "not published
 * yet" is expected and normal for the newest boundary, including
 * immediately after a fresh `/tiles/v1/snapshot` answer that raced ahead of
 * actual tile availability. On a network/5xx error, retries the *same*
 * boundary in place (`probeBoundaryWithRetry`) and, if it never resolves,
 * gives up with an error rather than guessing an older boundary might work
 * -- a transient failure is never treated as confirmed absence (see module
 * doc comment).
 *
 * `tile` is the probe coordinate (see {@link probeTileForSite} -- normally
 * derived from the currently selected radar site's lat/lon, not an
 * arbitrary/location-independent corner). `probeFn` defaults to
 * {@link browserProbeTile}; the desktop path passes one that routes through
 * the native Tauri command instead (see {@link ProbeFn}'s doc comment).
 */
export async function resolveRainbowSnapshot(
  apiKey: string,
  layer: RainbowTileLayer,
  tile: { z: number; x: number; y: number },
  forecastTime: number = RAINBOW_FORECAST_TIME_CURRENT,
  params: RainbowTileParams = { color: "0", coverage: false, usePrecipType: false },
  signal?: AbortSignal,
  probeFn: ProbeFn = browserProbeTile,
  snapshotStartFn?: SnapshotStartFn,
): Promise<RainbowSnapshotResolution> {
  let candidate = await resolveStartingCandidate(layer, apiKey, signal, snapshotStartFn);
  for (let step = 0; step <= MAX_BOUNDARY_FALLBACK_STEPS; step++) {
    const result = await probeBoundaryWithRetry(layer, candidate, forecastTime, tile, params, apiKey, probeFn, signal);
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
