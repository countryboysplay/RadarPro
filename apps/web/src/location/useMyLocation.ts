// The user's own position (browser geolocation; works in the Tauri webview
// too) and the "a warning covers you" check against active alert polygons.

import { useEffect, useRef, useState } from "react";
import type { HeldAlert } from "../alerts/types";
import { pointInAlertGeometry } from "../alerts/geometryHitTest";
import { legLengthAndBearing } from "../ui/CrossSectionPanel";

export interface MyLocation {
  lat: number;
  lon: number;
  /** Metres (95%-ish radius, per the Geolocation spec). */
  accuracyM: number;
  /** Degrees clockwise from north, or null while stationary/unknown. */
  headingDeg: number | null;
}

/** Below this, a moved-from-last-fix bearing is GPS jitter, not travel. */
const HEADING_MIN_MOVE_KM = 0.025;

export function useMyLocation(enabled: boolean): { location: MyLocation | null; error: string | null } {
  const [location, setLocation] = useState<MyLocation | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Where travel heading is measured from: the last fix that moved far enough.
  const anchorRef = useRef<{ lat: number; lon: number; headingDeg: number | null } | null>(null);

  useEffect(() => {
    setLocation(null);
    setError(null);
    anchorRef.current = null;
    if (!enabled) return;
    if (!("geolocation" in navigator)) {
      setError("location not available in this browser");
      return;
    }
    const id = navigator.geolocation.watchPosition(
      ({ coords }) => {
        const here = { lat: coords.latitude, lon: coords.longitude };
        // Windows location rarely reports a heading, so fall back to the bearing of travel.
        const gpsHeading = coords.heading !== null && Number.isFinite(coords.heading) && coords.speed ? coords.heading : null;
        const anchor = anchorRef.current;
        const moved = anchor && legLengthAndBearing(anchor, here);
        const headingDeg = gpsHeading ?? (moved && moved.km >= HEADING_MIN_MOVE_KM ? moved.deg : (anchor?.headingDeg ?? null));
        if (!moved || moved.km >= HEADING_MIN_MOVE_KM) anchorRef.current = { ...here, headingDeg };
        setLocation({ ...here, accuracyM: coords.accuracy, headingDeg });
        setError(null);
      },
      (e) => setError(e.code === e.PERMISSION_DENIED ? "location permission denied" : `location unavailable (${e.message})`),
      { enableHighAccuracy: true, maximumAge: 5000 },
    );
    return () => navigator.geolocation.clearWatch(id);
  }, [enabled]);

  return { location, error };
}

/** Active alerts whose polygon contains `point`, and which of those weren't
 * covering it last time (`prevKeys`), i.e. the ones to notify about. */
export function coveringAlerts(
  point: { lat: number; lon: number },
  active: readonly HeldAlert[],
  prevKeys: ReadonlySet<string>,
): { covering: HeldAlert[]; fresh: HeldAlert[] } {
  const covering = active.filter(({ alert }) => alert.geometry && pointInAlertGeometry([point.lon, point.lat], alert.geometry));
  return { covering, fresh: covering.filter(({ key }) => !prevKeys.has(key)) };
}

/** Two short tones. No audio asset to ship or load. */
export function playAlertTone() {
  const ctx = new AudioContext();
  [880, 660].forEach((hz, i) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.frequency.value = hz;
    gain.gain.value = 0.2;
    osc.connect(gain).connect(ctx.destination);
    osc.start(ctx.currentTime + i * 0.25);
    osc.stop(ctx.currentTime + i * 0.25 + 0.2);
  });
  setTimeout(() => void ctx.close(), 1000);
}
