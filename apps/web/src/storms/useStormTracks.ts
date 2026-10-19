import { useEffect, useState } from "react";
import { fetchStormProduct, parseHail, parseNst, parseTvs, type StormTracks } from "./nst";

/** NST updates once per volume scan (4-10 min); this catches each within a couple of minutes. */
const POLL_MS = 120_000;
/** Older than this, the NWS feed for the site has stalled; old tracks would mislead. */
const STALE_MS = 20 * 60_000;

/** Latest NWS storm tracks for `icao` while `enabled`, with TVS markers and
 * hail estimates merged in, re-polled every two minutes. */
export function useStormTracks(icao: string, enabled: boolean): { tracks: StormTracks | null; error: string | null } {
  const [tracks, setTracks] = useState<StormTracks | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setTracks(null);
    setError(null);
    if (!enabled) return;
    const controller = new AbortController();
    async function poll() {
      // TVS and hail are extras: if either fails or is stale, tracks still show without it.
      const extras = Promise.allSettled([
        fetchStormProduct("61tvs", icao, controller.signal).then(parseTvs),
        fetchStormProduct("p59hi", icao, controller.signal).then(parseHail),
      ]);
      try {
        const next = parseNst(await fetchStormProduct("58sti", icao, controller.signal));
        const [tvs, hail] = await extras;
        if (controller.signal.aborted) return;
        const fresh = (scanTimeMs: number) => Date.now() - scanTimeMs <= STALE_MS;
        if (!fresh(next.scanTimeMs)) {
          const when = new Date(next.scanTimeMs).toISOString().slice(5, 16).replace("-", "/").replace("T", " ");
          setTracks(null);
          setError(`NWS feed stalled, last ${when}Z`);
          return;
        }
        if (tvs.status === "fulfilled" && fresh(tvs.value.scanTimeMs)) next.tvs = tvs.value.tvs;
        if (hail.status === "fulfilled" && fresh(hail.value.scanTimeMs)) {
          for (const cell of next.cells) cell.hail = hail.value.hail.get(cell.id) ?? null;
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
