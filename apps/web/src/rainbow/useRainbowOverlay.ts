import { useCallback, useEffect, useRef, useState } from "react";
import { isDesktop } from "../platform/desktop";
import { useRainbowApiKey } from "./useRainbowApiKey";
import {
  buildRainbowPrecipTileUrlTemplate,
  probeTileForSite,
  RAINBOW_FORECAST_TIME_CURRENT,
  resolveRainbowSnapshot,
} from "./snapshot";
import { buildDesktopRainbowPrecipTileUrlTemplate, desktopProbeTile } from "./desktopTiles";

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
}

/**
 * Owns the Rainbow overlay's whole lifecycle: config-gating, the toggle
 * itself, and resolving `{snapshot}` (via `resolveRainbowSnapshot`) into a
 * ready-to-use MapLibre tile URL template. `App`/`MapView` only ever read
 * the fields above -- neither needs to know about Rainbow's snapshot
 * scheme or retry/fallback policy, matching this codebase's existing
 * provider-hook convention (`useForecastProvider`, `useAlertPoller`).
 *
 * `site` supplies the lat/lon the availability probe is tied to (see
 * `snapshot.ts`'s `probeTileForSite` -- a real, meaningful location instead
 * of the old always-`(0,0,0)` whole-earth tile). `App.tsx` passes its
 * currently selected radar site; only its `lat`/`lon` are read.
 *
 * On the desktop shell (`isDesktop()`), snapshot probing and the resulting
 * tile URL template route through the native Tauri commands in
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

  // Guards a resolution in flight against a toggle-off (or rapid re-toggle)
  // that happens before it settles -- same "generation counter" pattern
  // `useForecastProvider` uses for its own async provider switches, so a
  // stale response can never clobber a newer toggle state.
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

    const tile = probeTileForSite(siteRef.current.lat, siteRef.current.lon);
    const desktop = isDesktop();

    resolveRainbowSnapshot(
      effectiveKey,
      tile,
      RAINBOW_FORECAST_TIME_CURRENT,
      controller.signal,
      desktop ? desktopProbeTile : undefined,
    ).then(
      (result) => {
        if (generationRef.current !== generation) return; // superseded -- ignore.
        if (result.ok) {
          setTileUrlTemplate(
            desktop
              ? buildDesktopRainbowPrecipTileUrlTemplate(result.snapshot, RAINBOW_FORECAST_TIME_CURRENT)
              : buildRainbowPrecipTileUrlTemplate(result.snapshot, RAINBOW_FORECAST_TIME_CURRENT, effectiveKey),
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
    // while the overlay is enabled -- not just on toggle/configured changes.
  }, [configured, enabled, effectiveKey]);

  return { configured, enabled, toggle, status, error, tileUrlTemplate };
}
