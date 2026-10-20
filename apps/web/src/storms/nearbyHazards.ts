// Which storm hazards threaten the user's own position: a TVS, a mesocyclone
// or severe hail on a storm that is near now, or whose NWS forecast track
// brings it near within the next half hour. One alert per storm, merged
// across the radars around the user.

import { legLengthAndBearing } from "../ui/CrossSectionPanel";
import type { LonLat, Mesocyclone, StormCell, StormTracks, TvsDetection } from "./nst";

/** About 15 miles: close enough to act on, far enough to still have time. */
export const HAZARD_ALERT_KM = 25;
/** How far ahead along the forecast track to look. NST steps are 15 minutes. */
const LOOKAHEAD_MIN = 30;
const STEP_MIN = 15;
/** A meso this close to a cell belongs to it; two radars' cells this close are one storm. */
const SAME_STORM_KM = 10;

export interface SiteTracks {
  icao: string;
  tracks: StormTracks;
}

export interface StormAlert {
  /** Stable across scans (radar + NWS cell ID), so each storm alerts once. */
  key: string;
  /** Higher is worse: 1 hail, 2 mesocyclone, 3 low-level mesocyclone, 4 TVS. A storm re-alerts when this rises. */
  rank: number;
  title: string;
  body: string;
  /** Tornado-related: the notification stays up until dismissed. */
  urgent: boolean;
}

const COMPASS = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

interface Approach {
  /** Closest distance within the lookahead, and when (0 = now). */
  km: number;
  min: number;
  at: LonLat;
}

/** Closest point to `here` along `track` (15-minute steps from now) within the lookahead. */
export function closestApproach(here: { lat: number; lon: number }, track: LonLat[]): Approach {
  // Local flat km frame around `here`; fine over a ~50 km track.
  const kx = 111.32 * Math.cos((here.lat * Math.PI) / 180);
  const xy = ([lon, lat]: LonLat) => [(lon - here.lon) * kx, (lat - here.lat) * 110.57] as const;
  let best: Approach = { km: Math.hypot(...xy(track[0])), min: 0, at: track[0] };
  const steps = Math.min(track.length - 1, LOOKAHEAD_MIN / STEP_MIN);
  for (let i = 0; i < steps; i++) {
    const [ax, ay] = xy(track[i]);
    const [bx, by] = xy(track[i + 1]);
    const len2 = (bx - ax) ** 2 + (by - ay) ** 2;
    const t = len2 > 0 ? Math.min(1, Math.max(0, -(ax * (bx - ax) + ay * (by - ay)) / len2)) : 0;
    const km = Math.hypot(ax + t * (bx - ax), ay + t * (by - ay));
    if (km < best.km) {
      const at: LonLat = [track[i][0] + t * (track[i + 1][0] - track[i][0]), track[i][1] + t * (track[i + 1][1] - track[i][1])];
      best = { km, min: (i + t) * STEP_MIN, at };
    }
  }
  return best;
}

interface StormHazards {
  key: string;
  cell: StormCell | null;
  /** Where the worst hazard is now; its track is the cell's, shifted here. */
  at: LonLat;
  tvs: TvsDetection | null;
  mesos: Mesocyclone[];
  hailIn: number;
}

function hazardsBySite({ icao, tracks }: SiteTracks): StormHazards[] {
  const km = (a: LonLat, b: LonLat) => legLengthAndBearing({ lon: a[0], lat: a[1] }, { lon: b[0], lat: b[1] }).km;
  const byCell = new Map<string, StormHazards>();
  const forCell = (cell: StormCell) => {
    let h = byCell.get(cell.id);
    if (!h) byCell.set(cell.id, (h = { key: `${icao}:${cell.id}`, cell, at: cell.at, tvs: null, mesos: [], hailIn: 0 }));
    return h;
  };
  const loose: StormHazards[] = [];
  for (const cell of tracks.cells) if (cell.hail && cell.hail.sizeIn >= 1) forCell(cell).hailIn = cell.hail.sizeIn;
  for (const meso of tracks.mesos) {
    let owner: StormCell | null = null;
    for (const cell of tracks.cells) if (km(cell.at, meso.at) <= SAME_STORM_KM && (!owner || km(cell.at, meso.at) < km(owner.at, meso.at))) owner = cell;
    if (owner) forCell(owner).mesos.push(meso);
    else loose.push({ key: `${icao}:meso:${meso.id}`, cell: null, at: meso.at, tvs: null, mesos: [meso], hailIn: 0 });
  }
  for (const tvs of tracks.tvs) {
    const cell = tracks.cells.find((c) => c.id === tvs.stormId);
    if (cell) forCell(cell).tvs = tvs;
    else loose.push({ key: `${icao}:tvs:${tvs.stormId}`, cell: null, at: tvs.at, tvs, mesos: [], hailIn: 0 });
  }
  const all = [...byCell.values(), ...loose];
  for (const h of all) {
    const lowMeso = h.mesos.find((m) => m.lowLevel);
    h.at = h.tvs?.at ?? (lowMeso ?? h.mesos[0])?.at ?? h.at;
  }
  return all;
}

function rankOf(h: StormHazards): number {
  if (h.tvs) return 4;
  if (h.mesos.some((m) => m.lowLevel)) return 3;
  if (h.mesos.length > 0) return 2;
  return 1;
}

/**
 * Storms threatening `here`, worst first. `sites` should be ordered nearest
 * radar first: a storm seen by two radars is reported once, from the nearer.
 */
export function stormAlerts(here: { lat: number; lon: number }, sites: SiteTracks[]): StormAlert[] {
  const kept: { at: LonLat; alert: StormAlert }[] = [];
  const where = (at: LonLat) => {
    const { km, deg } = legLengthAndBearing(here, { lon: at[0], lat: at[1] });
    return km < 3 ? "over you" : `${Math.round(km / 1.609)} mi ${COMPASS[Math.round(deg / 45) % 8]} of you`;
  };
  for (const site of sites) {
    for (const h of hazardsBySite(site)) {
      // The cell's forecast track, carried along with the hazard's own offset from the cell centre.
      const [dx, dy] = h.cell ? [h.at[0] - h.cell.at[0], h.at[1] - h.cell.at[1]] : [0, 0];
      const track: LonLat[] = h.cell && h.cell.forecast.length >= 2 ? h.cell.forecast.map(([lon, lat]) => [lon + dx, lat + dy]) : [h.at];
      const approach = closestApproach(here, track);
      if (approach.km > HAZARD_ALERT_KM) continue;
      const sameStorm = kept.some(({ at }) => legLengthAndBearing({ lon: at[0], lat: at[1] }, { lon: h.at[0], lat: h.at[1] }).km <= SAME_STORM_KM);
      if (sameStorm) continue;

      const rank = rankOf(h);
      const lowMeso = h.mesos.some((m) => m.lowLevel);
      const hail = h.hailIn >= 1 ? `${h.hailIn.toFixed(2)}″ hail` : null;
      const what = [
        h.tvs ? (h.tvs.elevated ? "elevated TVS" : "TVS") : null,
        h.mesos.length > 0 ? (lowMeso ? "low-level mesocyclone" : "mesocyclone") : null,
        hail,
      ].filter(Boolean);
      const title =
        rank === 4
          ? h.tvs!.elevated
            ? "Elevated TVS"
            : "Tornado vortex signature"
          : rank === 3
            ? "Low-level mesocyclone"
            : rank === 2
              ? "Mesocyclone"
              : `${hail} storm`;
      const nearNow = legLengthAndBearing(here, { lon: h.at[0], lat: h.at[1] }).km <= HAZARD_ALERT_KM;
      const timing = nearNow
        ? `now ${where(h.at)}`
        : `arrives in ~${Math.max(5, Math.round(approach.min / 5) * 5)} min, passing ${where(approach.at)}`;
      const storm = h.cell ? `Storm ${h.cell.id}` : `Circulation ${h.mesos[0]?.id ?? h.tvs?.stormId}`;
      kept.push({
        at: h.at,
        alert: {
          key: h.key,
          rank,
          title: `${title} ${nearNow ? "near you" : "heading your way"}`,
          body: `${storm} (${site.icao}): ${what.join(", ")}; ${timing}`,
          urgent: rank >= 3,
        },
      });
    }
  }
  return kept.map(({ alert }) => alert).sort((a, b) => b.rank - a.rank);
}
