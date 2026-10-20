// The user's own position (browser geolocation; works in the Tauri webview
// too) and the "a warning covers you" check against active alert polygons.

import { useEffect, useRef, useState } from "react";
import type { HeldAlert } from "../alerts/types";
import { pointInAlertGeometry } from "../alerts/geometryHitTest";
import { fetchAlertIdsAtPoint } from "../alerts/nwsApi";
import { ALERT_POLL_INTERVAL_MS } from "../alerts/useAlertPoller";
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

/**
 * Ids of the active alerts NWS says cover `point` (see `fetchAlertIdsAtPoint`),
 * re-polled with the nationwide feed and whenever the point moves ~1 km.
 * Empty while `point` is null. Keeps the last answer through a failed poll.
 */
export function useAlertIdsAtPoint(point: { lat: number; lon: number } | null): ReadonlySet<string> {
  const [ids, setIds] = useState<ReadonlySet<string>>(new Set());
  const at = point ? `${point.lat.toFixed(2)},${point.lon.toFixed(2)}` : null;
  useEffect(() => {
    setIds(new Set());
    if (!at) return;
    const [lat, lon] = at.split(",").map(Number);
    const controller = new AbortController();
    const poll = () =>
      fetchAlertIdsAtPoint(lat, lon, controller.signal).then(
        (next) => !controller.signal.aborted && setIds(next),
        (err: unknown) => !controller.signal.aborted && console.warn("[alerts] point query failed:", err),
      );
    poll();
    const timer = setInterval(poll, ALERT_POLL_INTERVAL_MS);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [at]);
  return ids;
}

/** Active alerts covering `point` -- its polygon contains it, or NWS lists it
 * for that point (`idsAtPoint`: zone-only alerts such as watches) -- and which
 * of those weren't covering it last time (`prevKeys`), i.e. the ones to notify about. */
export function coveringAlerts(
  point: { lat: number; lon: number },
  active: readonly HeldAlert[],
  prevKeys: ReadonlySet<string>,
  idsAtPoint: ReadonlySet<string> = new Set(),
): { covering: HeldAlert[]; fresh: HeldAlert[] } {
  const covering = active.filter(
    ({ alert }) => idsAtPoint.has(alert.id) || (alert.geometry && pointInAlertGeometry([point.lon, point.lat], alert.geometry)),
  );
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
