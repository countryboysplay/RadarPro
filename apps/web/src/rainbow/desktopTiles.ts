// S10 follow-up (precip-only) / S09d Part A (all four Tiles API layers):
// Rainbow Weather tile CORS bypass for the desktop shell.
//
// `api.rainbow.ai` sends no `Access-Control-Allow-Origin` header for any
// browser origin -- confirmed independent of auth method or Tauri-vs-plain-
// browser context (see the `radarpro-rainbow-cors-no-browser-support`
// memory). The webview's own `fetch()`/XHR always enforces CORS, so this
// cannot be fixed in JS running in the webview; the only way for the
// desktop app to load these tiles is to make the actual HTTP request from
// the privileged Rust side (`apps/desktop/src-tauri/src/rainbow.rs`, no
// CORS enforcement at all there) and hand the result back over `invoke()`.
//
// This module is the desktop-only half of the Rainbow pipeline -- the
// `ProbeFn`/`SnapshotStartFn` `useRainbowOverlay` injects into
// `resolveRainbowSnapshot` when `isDesktop()`, plus the
// `maplibregl.addProtocol` registration `MapView.tsx` calls once at module
// load. The plain-browser path (`snapshot.ts`'s `browserProbeTile` + a
// normal `https://` raster tile URL) is untouched -- CORS there is a real,
// unfixable limitation, not a bug (see GLOBAL_CONTRACT and this stage's own
// "browser stays honest" requirement).
//
// Same key-handling discipline as everywhere else in this feature: the API
// key is looked up fresh per tile request (via `getEffectiveRainbowApiKey`)
// and passed to `invoke()` as a plain call argument -- never put into the
// `rainbow-tile://` URL that flows through MapLibre (which could otherwise
// end up in a MapLibre-emitted error/log line) and never logged here.
// `@tauri-apps/api/core` is imported dynamically (inside the functions
// below), not at module top level -- same convention `platform/desktop.ts`
// establishes and explains: it keeps the plain browser build from ever
// touching `window.__TAURI_INTERNALS__` at module-load time, only at actual
// call time, which in practice never happens outside the desktop shell
// since every caller here is itself `isDesktop()`-gated.
//
// `maplibre-gl`, by contrast, is imported statically: `MapView.tsx` already
// gives it a hard, unconditional dependency for every build (browser and
// desktop alike -- there is no code-splitting of it today), so importing it
// here too costs nothing extra.
import { addProtocol } from "maplibre-gl";
import { getEffectiveRainbowApiKey } from "./useRainbowApiKey";
import {
  isConfirmedNotYetAvailable,
  normalizeRainbowTileParams,
  type ProbeFn,
  type ProbeResult,
  type RainbowTileParams,
  type SnapshotStartFn,
} from "./snapshot";
import type { RainbowTileLayer } from "./types";

/** Custom MapLibre protocol scheme for desktop-only Rainbow tiles --
 * registered once via {@link registerDesktopRainbowProtocol}. */
const RAINBOW_TILE_PROTOCOL = "rainbow-tile";

/** Encodes a normalized `color` (possibly `undefined` for a layer that
 * doesn't support it) as a single URL path segment for the `rainbow-tile://`
 * scheme below -- `"-"` is the "absent" sentinel (never a valid palette id:
 * every real one is a bare digit or `dbz_u8`, KB §5), avoiding an empty path
 * segment (`//`), which some URL parsers collapse or reject. */
function encodeColorSegment(color: string | undefined): string {
  return color ?? "-";
}

function decodeColorSegment(segment: string): string | undefined {
  return segment === "-" ? undefined : segment;
}

/** Build the tile URL *template* MapLibre's raster source consumes on the
 * desktop path: `{z}/{x}/{y}` left as literal MapLibre placeholders
 * (MapLibre substitutes these for any protocol, custom schemes included --
 * the same mechanism the `pmtiles://` protocol relies on), everything else
 * (`layer`, `snapshot`, `forecast_time`, `color`, `coverage`,
 * `use_precip_type`) already resolved to concrete, layer-appropriate values
 * via `normalizeRainbowTileParams`. Deliberately carries no API key -- the
 * protocol handler below looks that up itself at request time (see the
 * module doc comment on why). */
export function buildDesktopRainbowTileUrlTemplate(
  layer: RainbowTileLayer,
  snapshot: number,
  forecastTime: number,
  params: RainbowTileParams,
): string {
  const normalized = normalizeRainbowTileParams(layer, forecastTime, params);
  const coverage = normalized.coverage ? "1" : "0";
  const usePrecipType = normalized.usePrecipType ? "1" : "0";
  return (
    `${RAINBOW_TILE_PROTOCOL}://${layer}/${snapshot}/${normalized.forecastTime}/` +
    `${encodeColorSegment(normalized.color)}/${coverage}/${usePrecipType}/{z}/{x}/{y}`
  );
}

interface RawRainbowTile {
  data_base64: string;
  content_type: string | null;
}

function base64ToArrayBuffer(base64: string): ArrayBuffer {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

/** Fetch one Rainbow tile's raw image bytes via the native
 * `rainbow_fetch_tile` Tauri command instead of `fetch()`. Throws on any
 * failure (network error, an invalid parameter combination the Rust side
 * rejects, or a non-2xx status -- all surfaced as a plain error string) --
 * callers (the protocol handler below) let that rejection propagate to
 * MapLibre, which treats a failed tile load the same way the plain browser
 * path's 404/network-failure raster tiles already do (silently missing, not
 * a crash). */
async function fetchTileNative(
  layer: RainbowTileLayer,
  snapshot: number,
  forecastTime: number,
  color: string | undefined,
  coverage: boolean,
  usePrecipType: boolean,
  z: number,
  x: number,
  y: number,
): Promise<ArrayBuffer> {
  const apiKey = getEffectiveRainbowApiKey();
  if (!apiKey) throw new Error("Rainbow API key not configured");
  const { invoke } = await import("@tauri-apps/api/core");
  const raw = await invoke<RawRainbowTile>("rainbow_fetch_tile", {
    layer,
    snapshot,
    forecastTime,
    z,
    x,
    y,
    color: color ?? null,
    coverage,
    usePrecipType,
    apiKey,
  });
  return base64ToArrayBuffer(raw.data_base64);
}

/** {@link ProbeFn} implementation for the desktop path: same
 * (layer, snapshot, forecast_time, tile, params) ->
 * published/not-published/transient-error mapping `snapshot.ts`'s
 * `browserProbeTile` does, but via the native `rainbow_probe_tile` Tauri
 * command instead of `fetch`. Passed into `resolveRainbowSnapshot` by
 * `useRainbowOverlay` when `isDesktop()` -- the retry/fallback policy itself
 * lives entirely in `snapshot.ts` and is unchanged by this swap. */
export const desktopProbeTile: ProbeFn = async (layer, snapshot, forecastTime, tile, params, apiKey): Promise<ProbeResult> => {
  try {
    const normalized = normalizeRainbowTileParams(layer, forecastTime, params);
    const { invoke } = await import("@tauri-apps/api/core");
    const status = await invoke<number>("rainbow_probe_tile", {
      layer,
      snapshot,
      forecastTime: normalized.forecastTime,
      z: tile.z,
      x: tile.x,
      y: tile.y,
      color: normalized.color ?? null,
      coverage: normalized.coverage,
      usePrecipType: normalized.usePrecipType,
      apiKey,
    });
    if (isConfirmedNotYetAvailable(status)) return "not-published"; // confirmed absence -- see snapshot.ts's doc comment (404 documented, 400 found live).
    if (status >= 200 && status < 300) return "published";
    return "transient-error";
  } catch {
    // `invoke` rejects on the Rust side's `Err` (a real network failure, or
    // an invalid parameter combination -- see `rainbow_tile_url`'s doc
    // comment in `rainbow.rs`) -- same "not confirmed absence" treatment as
    // `browserProbeTile`'s catch block.
    return "transient-error";
  }
};

/** {@link SnapshotStartFn} implementation for the desktop path: the native
 * transport for `GET /tiles/v1/snapshot?layer=<layer>` (KB §4.1). Lets it
 * throw straight through on any failure -- `resolveStartingCandidate`
 * (`snapshot.ts`) already catches and falls back to the local
 * `latestTenMinuteBoundaryEpoch()` guess, so there is nothing useful to add
 * by catching here too. No browser equivalent exists: see this function's
 * exported type's doc comment in `snapshot.ts` for why. */
export const desktopGetSnapshot: SnapshotStartFn = async (layer, apiKey) => {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<number>("rainbow_get_snapshot", { layer, apiKey });
};

let protocolRegistered = false;

/**
 * Register the `rainbow-tile://` MapLibre protocol handler, once per app
 * lifetime. Idempotent (a second call is a no-op) so `MapView.tsx` can call
 * this unconditionally at module load without tracking whether an earlier
 * mount already did. No-op in a browser tab -- `MapView.tsx` gates the call
 * on `isDesktop()` itself, same convention as every other Tauri touchpoint
 * (`apps/web/src/platform/desktop.ts`).
 */
export function registerDesktopRainbowProtocol(): void {
  if (protocolRegistered) return;
  protocolRegistered = true;

  const urlPattern =
    /^rainbow-tile:\/\/([a-z-]+)\/(\d+)\/(\d+)\/([^/]+)\/([01])\/([01])\/(\d+)\/(\d+)\/(\d+)$/;

  addProtocol(RAINBOW_TILE_PROTOCOL, async (params) => {
    const match = urlPattern.exec(params.url);
    if (!match) throw new Error("malformed rainbow-tile:// URL");
    const [, layer, snapshot, forecastTime, colorSegment, coverage, usePrecipType, z, x, y] = match;
    const data = await fetchTileNative(
      layer as RainbowTileLayer,
      Number(snapshot),
      Number(forecastTime),
      decodeColorSegment(colorSegment),
      coverage === "1",
      usePrecipType === "1",
      Number(z),
      Number(x),
      Number(y),
    );
    return { data };
  });
}
