// WSR-88D radar site directory. Backed by a vendored copy of the real,
// NOAA-sourced 159-site fixture -- see `src/data/README.md` and
// `fixtures/nexrad-sites/README.md` for provenance. Never hand-transcribed.

import rawSites from "./data/wsr88d-sites.json";

export interface RadarSite {
  /** Four-letter ICAO identifier, e.g. "KTLX". */
  icao: string;
  /** Human-readable site/city name, e.g. "Oklahoma City". */
  name: string;
  lat: number;
  lon: number;
  /** Antenna elevation above sea level, in meters. */
  heightM: number;
}

function loadSites(): RadarSite[] {
  const parsed = rawSites as Array<{
    icao: string;
    name: string;
    lat: number;
    lon: number;
    height_m: number;
  }>;
  return parsed
    .map((s) => ({ icao: s.icao, name: s.name, lat: s.lat, lon: s.lon, heightM: s.height_m }))
    .sort((a, b) => a.icao.localeCompare(b.icao));
}

/** All 159 real WSR-88D sites, sorted by ICAO identifier. */
export const RADAR_SITES: RadarSite[] = loadSites();

/**
 * Default selected site on first load. KTLX (Oklahoma City / Norman) is a
 * reasonable default: it is a well-known, frequently-active WSR-88D site
 * with a long history of real convective weather, making it a good "does
 * this actually show something" first impression.
 */
export const DEFAULT_SITE_ICAO = "KTLX";

export function findSite(icao: string): RadarSite | undefined {
  return RADAR_SITES.find((s) => s.icao === icao);
}
