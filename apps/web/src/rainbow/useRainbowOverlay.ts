import { useCallback, useEffect, useRef, useState } from "react";
import { isRainbowConfigured, RAINBOW_API_KEY } from "./config";
import {
  buildRainbowPrecipTileUrlTemplate,
  RAINBOW_FORECAST_TIME_CURRENT,
  resolveRainbowSnapshot,
} from "./snapshot";

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
 */
export function useRainbowOverlay(): RainbowOverlay {
  const configured = isRainbowConfigured();
  const [enabled, setEnabled] = useState(false);
  const [status, setStatus] = useState<RainbowStatus>(configured ? "off" : "unconfigured");
  const [error, setError] = useState<string | null>(null);
  const [tileUrlTemplate, setTileUrlTemplate] = useState<string | null>(null);

  // Guards a resolution in flight against a toggle-off (or rapid re-toggle)
  // that happens before it settles -- same "generation counter" pattern
  // `useForecastProvider` uses for its own async provider switches, so a
  // stale response can never clobber a newer toggle state.
  const generationRef = useRef(0);

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

    resolveRainbowSnapshot(RAINBOW_API_KEY, RAINBOW_FORECAST_TIME_CURRENT, controller.signal).then(
      (result) => {
        if (generationRef.current !== generation) return; // superseded -- ignore.
        if (result.ok) {
          setTileUrlTemplate(
            buildRainbowPrecipTileUrlTemplate(result.snapshot, RAINBOW_FORECAST_TIME_CURRENT, RAINBOW_API_KEY),
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
  }, [configured, enabled]);

  return { configured, enabled, toggle, status, error, tileUrlTemplate };
}
