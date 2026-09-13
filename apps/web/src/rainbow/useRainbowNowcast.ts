// S09d Part B: Rainbow Nowcast API (KB §6) -- minute-by-minute precip
// forecast for the next 4 hours at a point.
//
// Phase-machine shape modeled on `forecast/useForecastProvider.ts`
// (`idle|loading|...|ready|error`, a generation-counter guard against a
// stale response, a per-call timeout) -- that hook is a *different*
// forecast concept (GEFS/HRRR NWP) and is otherwise untouched; only its
// shape is copied here, per this stage's explicit instruction.
//
// Same CORS situation as the Tiles API: `api.rainbow.ai` sends no CORS
// headers for any browser origin, so this only ever works on the desktop
// shell (`isDesktop()`), via the native `rainbow_nowcast_precip` Tauri
// command (`apps/desktop/src-tauri/src/rainbow_weather.rs`). In a plain
// browser dev build, `fetchNowcast` never attempts a doomed `fetch()` --
// it fails immediately with a clear "desktop app required" message.
import { useCallback, useRef, useState } from "react";
import { isDesktop } from "../platform/desktop";
import { useRainbowApiKey } from "./useRainbowApiKey";
import { isRainbowHttp404, rainbowErrorMessage, withTimeout } from "./rainbowAsync";
import { useRainbowPoint, type RainbowPoint } from "./useRainbowPoint";

export type RainbowNowcastPhase = "idle" | "loading" | "ready" | "error";

/** Wire shape of `PrecipForecastSummary` (KB §6.3). */
export interface PrecipForecastSummary {
  intensity: string;
}

/** Wire shape of one `PrecipForecastItem` (KB §6.3) -- field names match
 * `rainbow_weather.rs`'s `PrecipForecastItem` (`camelCase` on both sides of
 * the IPC boundary and the underlying API, so this mirrors the KB's field
 * table directly). */
export interface PrecipForecastItem {
  precipRate: number;
  precipType: string;
  timestampBegin: number;
  timestampEnd: number;
}

/** Wire shape of `PrecipForecastResponse` (KB §6.3). */
export interface PrecipForecastResponse {
  longitude: number;
  latitude: number;
  summary: PrecipForecastSummary;
  forecast: PrecipForecastItem[];
}

export interface RainbowNowcastState {
  /** The point this panel's inputs currently show -- owned here (rather
   * than inside `RainbowNowcastPanel`) so `App.tsx` can push a clicked map
   * point into it (S09d follow-up: "click the map to set the point")
   * without reaching past this hook into the panel's own local state. */
  point: RainbowPoint;
  setLon: (lon: number) => void;
  setLat: (lat: number) => void;
  /** Set both coordinates at once -- what a map click uses (see
   * `useRainbowPoint`'s `setPoint` doc comment). */
  setPoint: (lon: number, lat: number) => void;
  resetToDefault: () => void;
  /** Whether a Rainbow API key is configured at all -- same source of
   * truth as `RainbowToggle`'s (`useRainbowApiKey`). */
  configured: boolean;
  /** Whether this is running inside the desktop shell -- Nowcast is
   * unreachable from a plain browser tab (CORS), so the panel renders a
   * distinct "desktop app required" state when this is false. */
  desktopAvailable: boolean;
  phase: RainbowNowcastPhase;
  error: string | null;
  result: PrecipForecastResponse | null;
  /** Whether the last successful fetch actually came from the
   * `precip-global` fallback (KB §6.2) rather than `precip` -- the panel
   * surfaces this so a result is never silently mislabeled as
   * region-specific coverage when it wasn't. */
  usedGlobalFallback: boolean;
  /**
   * Fetch the nowcast for `point` (and optional `startTimestamp`, epoch
   * seconds). Only ever runs when called -- never as a reaction to `point`/
   * `startTimestamp` changing (this stage's "no fetch until the user
   * presses Get" requirement). Safe to call repeatedly; a superseded
   * in-flight call's result is discarded via the generation counter.
   */
  fetchNowcast: (point: RainbowPoint, startTimestamp: number | null) => void;
}

/** KB §6.1: `start_timestamp` must be aligned to a 1-minute boundary. */
const START_TIMESTAMP_ALIGN_SECONDS = 60;
/** KB §6.1: `start_timestamp` may be at most 30 minutes in the past. */
const START_TIMESTAMP_MAX_AGE_SECONDS = 1_800;

/** Client-side mirror of `rainbow_weather.rs`'s
 * `validate_nowcast_start_timestamp` (KB §6.1) -- catches an invalid value
 * before ever building a request, so a bad `start_timestamp` surfaces as an
 * immediate, clear error instead of a round trip that comes back 422. The
 * Rust side still validates independently (never trust a client-side check
 * alone) -- this is purely a faster, friendlier failure for the common case.
 * `now` is injectable for tests; real callers use the default. */
export function validateNowcastStartTimestamp(startTimestamp: number, now: number = Math.floor(Date.now() / 1000)): string | null {
  if (startTimestamp % START_TIMESTAMP_ALIGN_SECONDS !== 0) {
    return "start time must be aligned to a 1-minute boundary";
  }
  if (startTimestamp >= now) {
    return "start time must be before the current time";
  }
  if (now - startTimestamp > START_TIMESTAMP_MAX_AGE_SECONDS) {
    return "start time cannot be more than 30 minutes in the past";
  }
  return null;
}

async function invokeNowcast(
  point: RainbowPoint,
  useGlobal: boolean,
  startTimestamp: number | null,
  apiKey: string,
): Promise<PrecipForecastResponse> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<PrecipForecastResponse>("rainbow_nowcast_precip", {
    lon: point.lon,
    lat: point.lat,
    useGlobal,
    startTimestamp,
    apiKey,
  });
}

/**
 * Owns one Nowcast fetch at a time: `GET /nowcast/v1/precip/{lon}/{lat}`
 * (KB §6.1) first, falling back to `/nowcast/v1/precip-global/{lon}/{lat}`
 * (KB §6.2) only on a confirmed HTTP 404 -- the exact retry idiom
 * `provider-gefs`/`provider-hrrr`'s discovery loops already use
 * ([[radarpro-discovery-loop-transient-error-swallowing]]): a 404 means "no
 * coverage here, fall back to global"; a network/5xx error is not that and
 * is surfaced as a real error rather than silently retried against the
 * fallback layer. The fallback decision is plain, visible JS here (not
 * hidden inside the Rust command), per this stage's explicit instruction.
 *
 * `defaultPoint` (this app's currently selected radar site's lat/lon, same
 * value passed everywhere else in this feature) prefills the point and is
 * followed until the user edits it by hand or via a map click -- see
 * `useRainbowPoint`'s doc comment for that "prefilled but not fought with"
 * behavior.
 */
export function useRainbowNowcast(defaultPoint: RainbowPoint): RainbowNowcastState {
  const { effectiveKey, configured } = useRainbowApiKey();
  const desktopAvailable = isDesktop();
  const { point, setLon, setLat, setPoint, resetToDefault } = useRainbowPoint(defaultPoint);

  const [phase, setPhase] = useState<RainbowNowcastPhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<PrecipForecastResponse | null>(null);
  const [usedGlobalFallback, setUsedGlobalFallback] = useState(false);

  const generationRef = useRef(0);

  const fetchNowcast = useCallback(
    (point: RainbowPoint, startTimestamp: number | null) => {
      const generation = ++generationRef.current;

      if (!desktopAvailable) {
        setPhase("error");
        setError("Rainbow Nowcast requires the desktop app -- api.rainbow.ai blocks direct browser requests (CORS).");
        return;
      }
      if (!configured) {
        setPhase("error");
        setError("Rainbow API key not configured -- add one in Settings.");
        return;
      }
      if (startTimestamp !== null) {
        const validationError = validateNowcastStartTimestamp(startTimestamp);
        if (validationError) {
          setPhase("error");
          setError(validationError);
          return;
        }
      }

      setPhase("loading");
      setError(null);

      void (async () => {
        try {
          let usedGlobal = false;
          let data: PrecipForecastResponse;
          try {
            data = await withTimeout(invokeNowcast(point, false, startTimestamp, effectiveKey), "Rainbow nowcast");
          } catch (err) {
            if (!isRainbowHttp404(err)) throw err;
            usedGlobal = true;
            data = await withTimeout(invokeNowcast(point, true, startTimestamp, effectiveKey), "Rainbow nowcast (global)");
          }
          if (generationRef.current !== generation) return; // superseded by a newer fetchNowcast call.
          setResult(data);
          setUsedGlobalFallback(usedGlobal);
          setPhase("ready");
        } catch (err) {
          if (generationRef.current !== generation) return;
          setResult(null);
          setPhase("error");
          setError(rainbowErrorMessage(err));
        }
      })();
    },
    [configured, desktopAvailable, effectiveKey],
  );

  return { point, setLon, setLat, setPoint, resetToDefault, configured, desktopAvailable, phase, error, result, usedGlobalFallback, fetchNowcast };
}
