import { useCallback, useEffect, useRef, useState } from "react";
import { isDesktop } from "../platform/desktop";
import { useRainbowApiKey } from "./useRainbowApiKey";
import {
  buildRainbowTileUrlTemplate,
  probeTileForSite,
  RAINBOW_FORECAST_TIME_CURRENT,
  resolveRainbowSnapshot,
  type RainbowTileParams,
} from "./snapshot";
import { buildDesktopRainbowTileUrlTemplate, desktopGetSnapshot, desktopProbeTile } from "./desktopTiles";
import {
  DEFAULT_RAINBOW_PALETTE,
  DEFAULT_RAINBOW_TILE_LAYER,
  rainbowTileLayerInfo,
  type RainbowTileLayer,
  type RainbowTileLayerInfo,
} from "./types";

export type RainbowStatus =
  | "unconfigured" // no VITE_RAINBOW_API_KEY at all -- toggle stays disabled.
  | "off" // configured, but the user hasn't turned it on.
  | "resolving" // toggled on, probing for a usable snapshot.
  | "ready" // resolved -- `tileUrlTemplate` is safe to hand to MapLibre.
  | "error"; // resolution failed (see `error`); overlay stays off.

export interface RainbowOverlay {
  /** Whether a key is configured at all -- drives the toggle's
   * enabled/disabled affordance (`RainbowToggle`). */
  configured: boolean;
  /** The user's toggle intent. True as soon as they flip it on, independent
   * of whether snapshot resolution has finished -- `MapView` uses this
   * (not `status`) to decide whether to hide the live radar canvas, since
   * that mutual-exclusivity decision should follow user intent immediately,
   * not lag behind a network round trip. */
  enabled: boolean;
  toggle: () => void;
  status: RainbowStatus;
  error: string | null;
  /** Concrete MapLibre raster tile URL template (`{z}/{x}/{y}` still
   * literal), or `null` whenever it isn't safe to add the layer (off,
   * unconfigured, resolving, or errored). */
  tileUrlTemplate: string | null;

  /** S09d Part A: the four documented Tiles API layers (KB §4/§8). Changing
   * this is a plain state update -- it never itself fires a network call;
   * only the resolution effect below (gated on `enabled`) does, and only
   * when the overlay is already on. */
  layer: RainbowTileLayer;
  setLayer: (layer: RainbowTileLayer) => void;
  /** Convenience lookup of `layer`'s own rule set (`types.ts`) -- lets
   * `RainbowToggle` decide which controls to show without re-deriving the
   * per-layer rules itself, and lets `MapView` read the current zoom
   * ceiling for the MapLibre source config. */
  layerInfo: RainbowTileLayerInfo;

  /** Palette id (KB §5) -- ignored server-side for `clouds` (undocumented
   * there); see `layerInfo.supportsColorCoverage`. */
  color: string;
  setColor: (color: string) => void;

  /** Coverage-mask query param -- `precip`/`precip-global`/`radars` only. */
  coverage: boolean;
  setCoverage: (coverage: boolean) => void;

  /** `use_precip_type` query param -- `radars` only (KB §4.5). */
  usePrecipType: boolean;
  setUsePrecipType: (usePrecipType: boolean) => void;

  /** Forecast offset in seconds, `[0, 14400]` step `600` -- `precip`/
   * `precip-global` only (KB §4.2/§4.3). */
  forecastTime: number;
  setForecastTime: (forecastTime: number) => void;
}

/**
 * Owns the Rainbow overlay's whole lifecycle: config-gating, the toggle
 * itself, the layer/palette/coverage/use_precip_type/forecast_time
 * selection state (S09d Part A), and resolving `{snapshot}` (via
 * `resolveRainbowSnapshot`) into a ready-to-use MapLibre tile URL template.
 * `App`/`MapView` only ever read the fields above -- neither needs to know
 * about Rainbow's snapshot scheme or retry/fallback policy, matching this
 * codebase's existing provider-hook convention (`useForecastProvider`,
 * `useAlertPoller`).
 *
 * Per this stage's explicit UI decision, changing any selection dropdown
 * never itself fires a network call -- it's a plain `useState` update.
 * Only two things start a resolution: flipping `enabled` on, or changing a
 * selection *while already enabled* (a legitimate re-fetch of the newly
 * chosen combination) -- both fall out naturally from the single effect
 * below listing every selection field in its dependency array alongside
 * `enabled`: when `enabled` is false the effect's body returns immediately
 * after clearing state, before ever reaching a network call.
 *
 * `site` supplies the lat/lon the availability probe is tied to (see
 * `snapshot.ts`'s `probeTileForSite` -- a real, meaningful location instead
 * of the old always-`(0,0,0)` whole-earth tile). `App.tsx` passes its
 * currently selected radar site; only its `lat`/`lon` are read.
 *
 * On the desktop shell (`isDesktop()`), snapshot probing/lookup and the
 * resulting tile URL template route through the native Tauri commands in
 * `apps/desktop/src-tauri/src/rainbow.rs` (via `./desktopTiles`) instead of
 * a plain `fetch`/`https://` URL, since `api.rainbow.ai` sends no CORS
 * headers for any browser origin and the webview's own `fetch()` can never
 * load these tiles (see `desktopTiles.ts`'s doc comment). The plain browser
 * path is completely unchanged -- CORS there is a real, unfixable
 * limitation, not a bug.
 */
export function useRainbowOverlay(site: { lat: number; lon: number }): RainbowOverlay {
  // S09c: `effectiveKey`/`configured` are reactive to a key saved in the
  // Settings section (localStorage), not just the build-time env var -- see
  // `useRainbowApiKey`'s doc comment for the priority rule.
  const { effectiveKey, configured } = useRainbowApiKey();
  const [enabled, setEnabled] = useState(false);
  const [status, setStatus] = useState<RainbowStatus>(configured ? "off" : "unconfigured");
  const [error, setError] = useState<string | null>(null);
  const [tileUrlTemplate, setTileUrlTemplate] = useState<string | null>(null);

  const [layer, setLayer] = useState<RainbowTileLayer>(DEFAULT_RAINBOW_TILE_LAYER);
  const [color, setColor] = useState<string>(DEFAULT_RAINBOW_PALETTE);
  const [coverage, setCoverage] = useState(false);
  const [usePrecipType, setUsePrecipType] = useState(false);
  const [forecastTime, setForecastTime] = useState(RAINBOW_FORECAST_TIME_CURRENT);

  // Guards a resolution in flight against a toggle-off (or rapid re-toggle,
  // or a fast option change) that happens before it settles -- same
  // "generation counter" pattern `useForecastProvider` uses for its own
  // async provider switches, so a stale response can never clobber a newer
  // state.
  const generationRef = useRef(0);

  // `site` changes over the hook's lifetime (the user can pick a different
  // radar site while Rainbow is enabled) but shouldn't itself force a
  // re-resolution -- it's only read at the moment a resolution starts, same
  // "ref for a value the effect reads but doesn't need to re-run for"
  // pattern `rangeRingsRef`/`alertsRef` use in `MapView.tsx`.
  const siteRef = useRef(site);
  siteRef.current = site;

  const toggle = useCallback(() => {
    if (!configured) return; // never toggleable with no key -- see RainbowToggle.
    setEnabled((prev) => !prev);
  }, [configured]);

  useEffect(() => {
    if (!configured) return; // status is permanently "unconfigured"; nothing to resolve.

    if (!enabled) {
      generationRef.current += 1; // invalidate any resolution still in flight.
      setStatus("off");
      setTileUrlTemplate(null);
      setError(null);
      return;
    }

    const generation = ++generationRef.current;
    const controller = new AbortController();
    setStatus("resolving");
    setError(null);

    const tile = probeTileForSite(siteRef.current.lat, siteRef.current.lon, layer);
    const desktop = isDesktop();
    const params: RainbowTileParams = { color, coverage, usePrecipType };

    resolveRainbowSnapshot(
      effectiveKey,
      layer,
      tile,
      forecastTime,
      params,
      controller.signal,
      desktop ? desktopProbeTile : undefined,
      desktop ? desktopGetSnapshot : undefined,
    ).then(
      (result) => {
        if (generationRef.current !== generation) return; // superseded -- ignore.
        if (result.ok) {
          setTileUrlTemplate(
            desktop
              ? buildDesktopRainbowTileUrlTemplate(layer, result.snapshot, forecastTime, params)
              : buildRainbowTileUrlTemplate(layer, result.snapshot, forecastTime, params, effectiveKey),
          );
          setStatus("ready");
        } else {
          setTileUrlTemplate(null);
          setStatus("error");
          setError(result.reason);
        }
      },
      // `resolveRainbowSnapshot` already turns every failure mode into an
      // `{ ok: false }` value, but guard here too rather than ever letting
      // an unhandled rejection reach the user (Global Contract: no
      // uncontrolled crashes on a failed fetch).
      (err: unknown) => {
        if (generationRef.current !== generation) return;
        setTileUrlTemplate(null);
        setStatus("error");
        setError(err instanceof Error ? err.message : "unexpected error resolving Rainbow snapshot");
      },
    );

    return () => {
      controller.abort();
    };
    // Re-resolves if the effective key itself changes (e.g. a Settings edit)
    // while the overlay is enabled, or if any selection field changes while
    // enabled -- not just on toggle/configured changes. When `enabled` is
    // false this still runs (any of these deps can change while off), but
    // the early-return above means it never reaches a network call -- see
    // this hook's own doc comment.
  }, [configured, enabled, effectiveKey, layer, color, coverage, usePrecipType, forecastTime]);

  return {
    configured,
    enabled,
    toggle,
    status,
    error,
    tileUrlTemplate,
    layer,
    setLayer,
    layerInfo: rainbowTileLayerInfo(layer),
    color,
    setColor,
    coverage,
    setCoverage,
    usePrecipType,
    setUsePrecipType,
    forecastTime,
    setForecastTime,
  };
}
