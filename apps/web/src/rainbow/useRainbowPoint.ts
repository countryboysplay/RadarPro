// S09d Parts B/C: shared lon/lat point state for the Nowcast and Weather
// panels. Both panels need "a point, defaulting to current map center,
// user-editable" (this stage's brief) -- this hook is the one place that
// logic lives instead of two near-identical copies in
// `RainbowNowcastPanel.tsx`/`RainbowWeatherPanel.tsx`.
//
// `defaultPoint` is a plain prop/argument (this app's currently selected
// radar site's lat/lon, threaded down from `App.tsx`) -- this hook never
// reaches into MapLibre/`MapView` itself, per this stage's explicit
// instruction.
import { useCallback, useEffect, useRef, useState } from "react";

export interface RainbowPoint {
  lon: number;
  lat: number;
}

export interface UseRainbowPointResult {
  point: RainbowPoint;
  setLon: (lon: number) => void;
  setLat: (lat: number) => void;
  /** Set both coordinates at once in a single state update -- e.g. a map
   * click (S09d follow-up: "click the map to set the point") naturally
   * produces both lon and lat together, so this avoids the extra render a
   * `setLon`+`setLat` pair would otherwise cause. */
  setPoint: (lon: number, lat: number) => void;
  /** Reset back to the current `defaultPoint` (e.g. a "use map center"
   * button), clearing the "user edited this" flag so a later default change
   * (the user picks a different radar site) is followed again. */
  resetToDefault: () => void;
}

/**
 * Owns one editable `{ lon, lat }` point, initialized from `defaultPoint`
 * and kept following it (e.g. the user selects a different radar site
 * elsewhere in the app) for as long as the user hasn't typed their own
 * value -- the same "prefilled but not fought with" pattern a search box's
 * placeholder-vs-typed-value distinction uses. Once the user edits either
 * field, this stops following `defaultPoint` until {@link resetToDefault}
 * is called.
 */
export function useRainbowPoint(defaultPoint: RainbowPoint): UseRainbowPointResult {
  const [point, setPointState] = useState<RainbowPoint>(defaultPoint);
  const userEditedRef = useRef(false);

  useEffect(() => {
    if (!userEditedRef.current) {
      setPointState(defaultPoint);
    }
    // Only re-run when the default's actual value changes, not on every
    // parent re-render (a fresh `{ lon, lat }` object identity each render
    // would otherwise re-trigger this every time).
  }, [defaultPoint.lon, defaultPoint.lat]);

  const setLon = useCallback((lon: number) => {
    userEditedRef.current = true;
    setPointState((p) => ({ ...p, lon }));
  }, []);

  const setLat = useCallback((lat: number) => {
    userEditedRef.current = true;
    setPointState((p) => ({ ...p, lat }));
  }, []);

  const setPoint = useCallback((lon: number, lat: number) => {
    userEditedRef.current = true;
    setPointState({ lon, lat });
  }, []);

  const resetToDefault = useCallback(() => {
    userEditedRef.current = false;
    setPointState(defaultPoint);
  }, [defaultPoint.lon, defaultPoint.lat]);

  return { point, setLon, setLat, setPoint, resetToDefault };
}
