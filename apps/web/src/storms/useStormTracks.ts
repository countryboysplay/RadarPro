import { useEffect, useState } from "react";
import { fetchNst, parseNst, type StormTracks } from "./nst";

/** NST updates once per volume scan (4-10 min); this catches each within a couple of minutes. */
const POLL_MS = 120_000;
/** Older than this, the NWS feed for the site has stalled; old tracks would mislead. */
const STALE_MS = 20 * 60_000;

/** Latest NWS storm tracks for `icao` while `enabled`, re-polled every two minutes. */
export function useStormTracks(icao: string, enabled: boolean): { tracks: StormTracks | null; error: string | null } {
  const [tracks, setTracks] = useState<StormTracks | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setTracks(null);
    setError(null);
    if (!enabled) return;
    const controller = new AbortController();
    async function poll() {
      try {
        const next = parseNst(await fetchNst(icao, controller.signal));
        if (controller.signal.aborted) return;
        if (Date.now() - next.scanTimeMs > STALE_MS) {
          const when = new Date(next.scanTimeMs).toISOString().slice(5, 16).replace("-", "/").replace("T", " ");
          setTracks(null);
          setError(`NWS feed stalled, last ${when}Z`);
          return;
        }
        setTracks(next);
        setError(null);
      } catch (err) {
        if (controller.signal.aborted) return;
        setError(err instanceof Error ? err.message : String(err));
      }
    }
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [icao, enabled]);

  return { tracks, error };
}
