import { useEffect, useMemo, useState } from "react";
import { RADAR_SITES } from "../sites";
import { legLengthAndBearing } from "../ui/CrossSectionPanel";
import type { SiteTracks } from "./nearbyHazards";
import { fetchArchivedStormProduct, fetchStormProduct, parseHail, parseMeso, parseNst, parseTvs, type StormTracks } from "./nst";

/** NST updates once per volume scan (4-10 min); this catches each within a couple of minutes. */
const POLL_MS = 120_000;
/** Older than this, the NWS feed for the site has stalled; old tracks would mislead. */
const STALE_MS = 20 * 60_000;

/** The site's NWS feed is too old to trust: drop its tracks rather than show stale ones. */
export class StormFeedStalled extends Error {}

/** Latest NWS storm tracks for `icao`, with TVS markers, hail estimates and
 * mesocyclones merged in. Throws `StormFeedStalled` when the feed has stalled. */
export async function fetchStormTracks(icao: string, signal: AbortSignal): Promise<StormTracks> {
  // TVS, hail and mesocyclones are extras: if one fails or is stale, tracks still show without it.
  const extras = Promise.allSettled([
    fetchStormProduct("61tvs", icao, signal).then(parseTvs),
    fetchStormProduct("p59hi", icao, signal).then(parseHail),
    fetchStormProduct("141md", icao, signal).then(parseMeso),
  ]);
  const next = parseNst(await fetchStormProduct("58sti", icao, signal));
  const [tvs, hail, meso] = await extras;
  const fresh = (scanTimeMs: number) => Date.now() - scanTimeMs <= STALE_MS;
  if (!fresh(next.scanTimeMs)) {
    const when = new Date(next.scanTimeMs).toISOString().slice(5, 16).replace("-", "/").replace("T", " ");
    throw new StormFeedStalled(`NWS feed stalled, last ${when}Z`);
  }
  if (tvs.status === "fulfilled" && fresh(tvs.value.scanTimeMs)) next.tvs = tvs.value.tvs;
  if (meso.status === "fulfilled" && fresh(meso.value.scanTimeMs)) next.mesos = meso.value.mesos;
  if (hail.status === "fulfilled" && fresh(hail.value.scanTimeMs)) {
    for (const cell of next.cells) cell.hail = hail.value.hail.get(cell.id) ?? null;
  }
  return next;
}

/** Archived storm tracks and mesocyclones for the scan at `timeMs` (NTV and
 * NHI are not archived, so no TVS markers or hail). */
async function fetchArchivedStormTracks(icao: string, timeMs: number, signal: AbortSignal): Promise<StormTracks> {
  const [nst, nmd] = await Promise.all([
    fetchArchivedStormProduct("NST", icao, timeMs, signal),
    fetchArchivedStormProduct("NMD", icao, timeMs, signal).catch(() => null),
  ]);
  // Older than this, the archived tracks belong to some other scan.
  const near = (scanTimeMs: number) => Math.abs(timeMs - scanTimeMs) <= 15 * 60_000;
  const tracks = nst && parseNst(nst);
  if (!tracks || !near(tracks.scanTimeMs)) throw new StormFeedStalled("no NWS storm tracks archived for this scan");
  if (nmd) {
    const meso = parseMeso(nmd);
    if (near(meso.scanTimeMs)) tracks.mesos = meso.mesos;
  }
  return tracks;
}

/**
 * NWS storm tracks for `icao` while `enabled`: the latest, re-polled every two
 * minutes, or with `archiveTimeMs` set, the archived ones for that scan.
 */
export function useStormTracks(
  icao: string,
  enabled: boolean,
  archiveTimeMs: number | null = null,
): { tracks: StormTracks | null; error: string | null; archived: boolean } {
  const [tracks, setTracks] = useState<StormTracks | null>(null);
  const [error, setError] = useState<string | null>(null);
  const archived = archiveTimeMs !== null;

  // Cleared on a new site or mode; a new archive frame keeps the old tracks until its own arrive.
  useEffect(() => {
    setTracks(null);
    setError(null);
  }, [icao, enabled, archived]);

  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    async function poll() {
      try {
        const next = archiveTimeMs !== null ? await fetchArchivedStormTracks(icao, archiveTimeMs, controller.signal) : await fetchStormTracks(icao, controller.signal);
        if (controller.signal.aborted) return;
        setTracks(next);
        setError(null);
      } catch (err) {
        if (controller.signal.aborted) return;
        if (err instanceof StormFeedStalled) setTracks(null);
        setError(err instanceof Error ? err.message : String(err));
      }
    }
    poll();
    // Archived products never change; only live ones are re-polled.
    const timer = archiveTimeMs === null ? setInterval(poll, POLL_MS) : undefined;
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [icao, enabled, archiveTimeMs]);

  return { tracks, error, archived };
}

/** Radars whose storm products can see storms near the user. */
const NEARBY_SITES = 3;
const NEARBY_SITE_KM = 250;

/** The radars nearest `here` (nearest first), within storm-product range. */
export function nearestSites(here: { lat: number; lon: number }): string[] {
  return RADAR_SITES.map((site) => ({ icao: site.icao, km: legLengthAndBearing(here, site).km }))
    .filter(({ km }) => km <= NEARBY_SITE_KM)
    .sort((a, b) => a.km - b.km)
    .slice(0, NEARBY_SITES)
    .map(({ icao }) => icao);
}

/**
 * Storm tracks from the radars nearest `here`, whichever site is on screen,
 * so hazard alerts follow the user, not the view. Nearest radar first.
 */
export function useNearbyStormTracks(here: { lat: number; lon: number } | null, enabled: boolean): SiteTracks[] {
  // Re-pick sites only when the list changes, not on every GPS update.
  const siteKey = enabled && here ? nearestSites(here).join(",") : "";
  const icaos = useMemo(() => (siteKey ? siteKey.split(",") : []), [siteKey]);
  const [bySite, setBySite] = useState<Map<string, StormTracks>>(new Map());

  useEffect(() => {
    setBySite(new Map());
    if (icaos.length === 0) return;
    const controller = new AbortController();
    async function poll() {
      const results = await Promise.allSettled(icaos.map((icao) => fetchStormTracks(icao, controller.signal)));
      if (controller.signal.aborted) return;
      // A network failure keeps the site's last tracks; a stalled feed drops them.
      setBySite((prev) => {
        const next = new Map(prev);
        results.forEach((r, i) => {
          if (r.status === "fulfilled") next.set(icaos[i], r.value);
          else if (r.reason instanceof StormFeedStalled) next.delete(icaos[i]);
        });
        return next;
      });
    }
    poll();
    const timer = setInterval(poll, POLL_MS);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
  }, [icaos]);

  return useMemo(
    () => icaos.flatMap((icao) => (bySite.has(icao) ? [{ icao, tracks: bySite.get(icao)! }] : [])),
    [icaos, bySite],
  );
}
