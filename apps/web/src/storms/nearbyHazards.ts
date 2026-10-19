// Which storm hazards are close enough to the user's own position to alert
// on: a TVS, a mesocyclone, or severe hail. Distance only.
// shortcut: ignores where storms are heading; add the cell's forecast track if
// chasers want warning before a hazard is already within range.

import { legLengthAndBearing } from "../ui/CrossSectionPanel";
import type { LonLat, StormTracks } from "./nst";

/** About 15 miles: close enough to act on, far enough to still have time. */
export const HAZARD_ALERT_KM = 25;

export interface NearbyHazard {
  /** Stable across scans (NWS IDs), so each hazard alerts once while it stays near. */
  key: string;
  title: string;
  body: string;
  /** Tornado-related: the notification stays up until dismissed. */
  urgent: boolean;
}

const COMPASS = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];

export function nearbyHazards(here: { lat: number; lon: number }, tracks: StormTracks): NearbyHazard[] {
  const away = ([lon, lat]: LonLat) => legLengthAndBearing(here, { lat, lon });
  const near = (at: LonLat) => away(at).km <= HAZARD_ALERT_KM;
  const where = (at: LonLat) => {
    const { km, deg } = away(at);
    return `${Math.round(km / 1.609)} mi ${COMPASS[Math.round(deg / 45) % 8]} of you`;
  };
  const out: NearbyHazard[] = [];
  for (const tvs of tracks.tvs) {
    if (!near(tvs.at)) continue;
    const title = tvs.elevated ? "Elevated TVS near you" : "Tornado vortex signature near you";
    out.push({ key: `tvs:${tvs.stormId}`, title, body: `Storm ${tvs.stormId}, ${where(tvs.at)}`, urgent: true });
  }
  for (const meso of tracks.mesos) {
    if (!near(meso.at)) continue;
    const title = meso.lowLevel ? "Low-level mesocyclone near you" : "Mesocyclone near you";
    out.push({ key: `meso:${meso.id}`, title, body: `Circulation ${meso.id}, ${where(meso.at)}`, urgent: meso.lowLevel });
  }
  for (const cell of tracks.cells) {
    if (!cell.hail || cell.hail.sizeIn < 1 || !near(cell.at)) continue;
    out.push({
      key: `hail:${cell.id}`,
      title: `${cell.hail.sizeIn.toFixed(2)}″ hail near you`,
      body: `Storm ${cell.id}, ${where(cell.at)} (${cell.hail.severeChance}% chance of severe hail)`,
      urgent: false,
    });
  }
  return out;
}
