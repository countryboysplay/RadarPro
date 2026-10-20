// Phase 5.2: HRRR point sounding. Full 39-level column from Open-Meteo (the
// only CORS-open per-point HRRR source; AWS needs ~128 MB of whole-CONUS
// messages per sounding). When Open-Meteo is slow or down, fall back to a
// NOMADS crop of the HRRR surface file: 5 pressure levels, so labelled coarse.
import { fetchUrlBytes } from "../placefiles/usePlacefiles";
import { ensureForecastWasmModuleLoaded, sampleHrrrColumn } from "./wasmModule";
import { legLengthAndBearing } from "../ui/CrossSectionPanel";

export interface SoundingLevel {
  /** hPa */
  p: number;
  /** metres above sea level */
  z: number;
  /** °C */
  t: number;
  /** °C */
  td: number;
  /** knots, toward east / north */
  u: number;
  v: number;
}

export interface Sounding {
  lat: number;
  lon: number;
  validMs: number;
  /** Surface first, pressure decreasing. */
  levels: SoundingLevel[];
  coarse: boolean;
  source: string;
  /** HRRR's own surface CAPE (J/kg), for the coarse column whose 500 mb top would cut the parcel's CAPE short. */
  modelCape: number | null;
  /** Observed soundings: the launch site and where it is from the click, e.g. "KOUN Norman OK · 47 mi W". */
  site?: string;
}

export const OPEN_METEO_TIMEOUT_MS = 8_000;
const PRESSURE_LEVELS = [1000, 975, 950, 925, 900, 875, 850, 825, 800, 775, 750, 725, 700, 675, 650, 625, 600, 575, 550, 525, 500, 475, 450, 425, 400, 375, 350, 325, 300, 275, 250, 225, 200, 175, 150, 125, 100, 75, 50];
const MS_TO_KT = 1.943844;

function windToUv(speedKt: number, fromDeg: number): { u: number; v: number } {
  const r = (fromDeg * Math.PI) / 180;
  return { u: -speedKt * Math.sin(r), v: -speedKt * Math.cos(r) };
}

/** Open-Meteo's `start_hour` format: UTC, minutes zeroed. */
function openMeteoHour(ms: number): string {
  return new Date(ms).toISOString().slice(0, 14) + "00";
}

export async function fetchOpenMeteoSounding(lat: number, lon: number, validMs: number, signal: AbortSignal): Promise<Sounding> {
  const fields = ["temperature", "dew_point", "wind_speed", "wind_direction", "geopotential_height"];
  const hourly = [
    "surface_pressure", "temperature_2m", "dew_point_2m", "wind_speed_10m", "wind_direction_10m",
    ...PRESSURE_LEVELS.flatMap((p) => fields.map((f) => `${f}_${p}hPa`)),
  ];
  const hour = openMeteoHour(validMs);
  const url =
    `https://api.open-meteo.com/v1/gfs?latitude=${lat.toFixed(3)}&longitude=${lon.toFixed(3)}` +
    `&models=ncep_hrrr_conus&wind_speed_unit=kn&start_hour=${hour}&end_hour=${hour}&hourly=${hourly.join(",")}`;
  const response = await fetch(url, { signal });
  const body = await response.json();
  if (!response.ok || body.error) throw new Error(body.reason ?? `HTTP ${response.status}`);
  const at = (name: string): number | null => body.hourly?.[name]?.[0] ?? null;
  const psfc = at("surface_pressure");
  const [t2, td2, ws10, wd10] = ["temperature_2m", "dew_point_2m", "wind_speed_10m", "wind_direction_10m"].map(at);
  if (psfc === null || t2 === null || td2 === null || ws10 === null || wd10 === null) throw new Error("HRRR has no data for that hour");
  const levels: SoundingLevel[] = [{ p: psfc, z: NaN, t: t2, td: td2, ...windToUv(ws10, wd10) }];
  for (const p of PRESSURE_LEVELS) {
    const [t, td, ws, wd, z] = fields.map((f) => at(`${f}_${p}hPa`));
    // Levels under the ground are extrapolated; the top level has no dew point.
    if (p >= psfc || t === null || ws === null || wd === null || z === null) continue;
    levels.push({ p, z, t, td: td ?? -100, ...windToUv(ws, wd) });
  }
  if (levels.length < 2) throw new Error("HRRR has no levels above the ground there");
  // Model ground, not Open-Meteo's `elevation` (a fine DEM that can be hundreds of metres off in hills):
  // hypsometric thickness down from the first level above the ground.
  const [sfc, above] = levels;
  sfc.z = above.z - ((RD * ((sfc.t + above.t) / 2 + 273.15)) / G) * Math.log(sfc.p / above.p);
  return { lat, lon, validMs, levels, coarse: false, source: "HRRR via Open-Meteo", modelCape: null };
}

/** The coarse fallback: NOMADS crops the HRRR surface file around the point. */
export async function fetchNomadsSounding(lat: number, lon: number, runInitMs: number, validMs: number, signal: AbortSignal): Promise<Sounding> {
  const run = new Date(runInitMs).toISOString();
  const lead = String(Math.round((validMs - runInitMs) / 3_600_000)).padStart(2, "0");
  const levels = ["surface", "2_m_above_ground", "10_m_above_ground", "1000_mb", "925_mb", "850_mb", "700_mb", "500_mb"];
  const url =
    `https://nomads.ncep.noaa.gov/cgi-bin/filter_hrrr_2d.pl?dir=%2Fhrrr.${run.slice(0, 10).replaceAll("-", "")}%2Fconus` +
    `&file=hrrr.t${run.slice(11, 13)}z.wrfsfcf${lead}.grib2` +
    ["TMP", "DPT", "UGRD", "VGRD", "HGT", "PRES", "CAPE"].map((v) => `&var_${v}=on`).join("") +
    levels.map((l) => `&lev_${l}=on`).join("") +
    `&subregion=&toplat=${lat + 0.05}&bottomlat=${lat - 0.05}&leftlon=${lon - 0.05}&rightlon=${lon + 0.05}`;
  const [bytes] = await Promise.all([fetchUrlBytes(url, signal), ensureForecastWasmModuleLoaded()]);
  const flat = sampleHrrrColumn(bytes, lat, lon);
  // key: "category.number/surfaceType/level"
  const values = new Map<string, number>();
  for (let i = 0; i < flat.length; i += 5) values.set(`${flat[i]}.${flat[i + 1]}/${flat[i + 2]}/${flat[i + 3]}`, flat[i + 4]);
  const get = (param: string, surface: number, level: number) => values.get(`${param}/${surface}/${level}`) ?? null;
  const level = (param: string, pa: number) => get(param, 100, pa);
  const psfcPa = get("3.0", 1, 0);
  const zsfc = get("3.5", 1, 0);
  const t2 = get("0.0", 103, 2);
  const td2 = get("0.6", 103, 2);
  const u10 = get("2.2", 103, 10);
  const v10 = get("2.3", 103, 10);
  if (psfcPa === null || zsfc === null || t2 === null || td2 === null || u10 === null || v10 === null) throw new Error("NOAA crop is missing surface fields");
  const out: SoundingLevel[] = [{ p: psfcPa / 100, z: zsfc, t: t2 - 273.15, td: td2 - 273.15, u: u10 * MS_TO_KT, v: v10 * MS_TO_KT }];
  for (const p of [1000, 925, 850, 700, 500]) {
    const [t, td, u, v] = ["0.0", "0.6", "2.2", "2.3"].map((k) => level(k, p * 100));
    if (p * 100 >= psfcPa || t === null || td === null || u === null || v === null) continue;
    out.push({ p, z: level("3.5", p * 100) ?? NaN, t: t - 273.15, td: td - 273.15, u: u * MS_TO_KT, v: v * MS_TO_KT });
  }
  fillMissingHeights(out);
  return { lat, lon, validMs, levels: out, coarse: true, source: "HRRR via NOAA NOMADS (5 levels)", modelCape: get("7.6", 1, 0) };
}

/** The surface file has no 925 mb height: interpolate in ln p between neighbours. */
function fillMissingHeights(levels: SoundingLevel[]): void {
  levels.forEach((l, i) => {
    if (!Number.isNaN(l.z)) return;
    const below = levels[i - 1];
    const above = levels.slice(i + 1).find((x) => !Number.isNaN(x.z));
    if (!above) return;
    const f = Math.log(below.p / l.p) / Math.log(below.p / above.p);
    l.z = below.z + f * (above.z - below.z);
  });
}

/** Open-Meteo first; on failure or timeout, the coarse NOAA column if the HRRR run is known. */
export async function fetchSounding(lat: number, lon: number, validMs: number, hrrrRunInitMs: number | null, signal: AbortSignal): Promise<Sounding> {
  const timeout = AbortSignal.timeout(OPEN_METEO_TIMEOUT_MS);
  try {
    return await fetchOpenMeteoSounding(lat, lon, validMs, AbortSignal.any([signal, timeout]));
  } catch (err) {
    if (signal.aborted || hrrrRunInitMs === null) throw err;
    return fetchNomadsSounding(lat, lon, hrrrRunInitMs, validMs, signal);
  }
}

// ---- Observed balloon soundings (Phase 5.4, IEM; CORS-open) ----

const IEM = "https://mesonet.agron.iastate.edu";
const SYNOPTIC_MS = 12 * 3_600_000;

interface RaobSite {
  name: string;
  lat: number;
  lon: number;
}

interface IemLevel {
  pres: number | null;
  hght: number | null;
  tmpc: number | null;
  dwpc: number | null;
  drct: number | null;
  sknt: number | null;
}

let raobSites: Promise<Map<string, RaobSite>> | null = null;
const raobLaunches = new Map<number, Promise<{ station: string; profile: IemLevel[] }[]>>();

function loadRaobSites(): Promise<Map<string, RaobSite>> {
  raobSites ??= fetch(`${IEM}/geojson/network/RAOB.geojson`)
    .then((r) => r.json())
    .then((body) => {
      const sites = new Map<string, RaobSite>();
      for (const f of body.features) {
        const [lon, lat] = f.geometry.coordinates;
        sites.set(f.properties.sid, { name: String(f.properties.sname).split(" -- ")[0].replace(/\s+/g, " ").replace(/\/US$/, ""), lat, lon });
      }
      return sites;
    });
  raobSites.catch(() => (raobSites = null));
  return raobSites;
}

/** Every US launch at one synoptic time in one request (~650 KB), cached: one fetch serves any click. */
function loadRaobLaunches(ms: number): Promise<{ station: string; profile: IemLevel[] }[]> {
  let launches = raobLaunches.get(ms);
  if (!launches) {
    const ts = new Date(ms).toISOString().slice(0, 13).replace(/\D/g, "") + "00";
    launches = fetch(`${IEM}/json/raob.py?ts=${ts}`).then((r) => r.json()).then((body) => body.profiles ?? []);
    raobLaunches.set(ms, launches);
    launches.catch(() => raobLaunches.delete(ms));
    // Launches keep arriving for hours after the synoptic time, so a recent time isn't kept.
    if (Date.now() - ms < 6 * 3_600_000) launches.finally(() => raobLaunches.delete(ms)).catch(() => {});
  }
  return launches;
}

/** IEM levels to ours: surface first; winds missing on significant levels are interpolated in height. */
export function raobLevels(profile: IemLevel[]): SoundingLevel[] {
  const rows = profile.filter((l) => l.pres !== null && l.hght !== null && l.tmpc !== null);
  const first = rows.findIndex((l) => l.dwpc !== null && l.drct !== null && l.sknt !== null);
  if (first < 0) return [];
  const used = rows.slice(first);
  const winds = used.filter((l) => l.drct !== null && l.sknt !== null).map((l) => ({ z: l.hght!, ...windToUv(l.sknt!, l.drct!) }));
  return used.map((l) => {
    let i = winds.findIndex((w) => w.z >= l.hght!);
    if (i < 0) i = winds.length - 1;
    const a = winds[Math.max(0, i - 1)];
    const b = winds[i];
    const f = b.z === a.z ? 1 : Math.min(1, Math.max(0, (l.hght! - a.z) / (b.z - a.z)));
    // Td stops in dry upper air; NaN leaves a gap in the Td line.
    return { p: l.pres!, z: l.hght!, t: l.tmpc!, td: l.dwpc ?? NaN, u: a.u + f * (b.u - a.u), v: a.v + f * (b.v - a.v) };
  });
}

/** The nearest site's latest launch at or before `atMs` (looks back up to 2 days, since a launch can be late or missing). */
export async function fetchRaobSounding(lat: number, lon: number, atMs: number): Promise<Sounding> {
  const sites = await loadRaobSites();
  let ms = Math.floor(atMs / SYNOPTIC_MS) * SYNOPTIC_MS;
  // ponytail: takes the newest time with any launch in, so right after 00Z/12Z a far site that reported early beats a near one still on its way; prefer a near older launch if that bites.
  for (let tries = 0; tries < 4; tries++, ms -= SYNOPTIC_MS) {
    let best: { km: number; sounding: Sounding } | null = null;
    for (const { station, profile } of await loadRaobLaunches(ms)) {
      const site = sites.get(station);
      if (!site) continue;
      const { km, deg } = legLengthAndBearing({ lat, lon }, site);
      if (best && km >= best.km) continue;
      const levels = raobLevels(profile);
      if (levels.length < 2) continue;
      const where = `${Math.round(km / 1.609)} mi ${["N", "NE", "E", "SE", "S", "SW", "W", "NW"][Math.round(deg / 45) % 8]}`;
      best = { km, sounding: { lat: site.lat, lon: site.lon, validMs: ms, levels, coarse: false, source: "Observed balloon sounding via Iowa Environmental Mesonet", modelCape: null, site: `${station} ${site.name} · ${where}` } };
    }
    if (best) return best.sounding;
  }
  throw new Error("no balloon launches found in the last 2 days");
}

// ---- Parcel math (surface-based) ----

const RD = 287.04;
const CP = 1005.7;
const LV = 2.501e6;
const EPS = 0.622;
const G = 9.80665;

/** Saturation vapour pressure, hPa (Bolton 1980). */
function satVapor(tC: number): number {
  return 6.112 * Math.exp((17.67 * tC) / (tC + 243.5));
}

/** LCL temperature (K) from surface T and Td in K (Bolton 1980, eq. 15). */
function lclTempK(tK: number, tdK: number): number {
  return 1 / (1 / (tdK - 56) + Math.log(tK / tdK) / 800) + 56;
}

export interface ParcelResult {
  /** Parcel temperature (°C) at each sounding level. */
  path: { p: number; t: number }[];
  lclP: number;
  /** metres above ground */
  lclAgl: number;
  /** J/kg */
  cape: number;
}

/** Lift the surface parcel: dry adiabat to the LCL, then a pseudo-adiabat in 5 hPa steps. */
export function surfaceParcel(levels: SoundingLevel[]): ParcelResult {
  const sfc = levels[0];
  const tK = sfc.t + 273.15;
  const tLcl = lclTempK(tK, sfc.td + 273.15);
  const lclP = sfc.p * (tLcl / tK) ** (CP / RD);
  const path: { p: number; t: number }[] = [];
  let p = lclP;
  let tMoist = tLcl;
  for (const level of levels) {
    if (level.p >= lclP) {
      path.push({ p: level.p, t: tK * (level.p / sfc.p) ** (RD / CP) - 273.15 });
      continue;
    }
    while (p > level.p) {
      const dp = Math.min(5, p - level.p);
      const es = satVapor(tMoist - 273.15);
      const rs = (EPS * es) / (p - es);
      tMoist -= (dp / p) * ((RD * tMoist + LV * rs) / (CP + (LV * LV * rs * EPS) / (RD * tMoist * tMoist)));
      p -= dp;
    }
    path.push({ p: level.p, t: tMoist - 273.15 });
  }
  // shortcut: CAPE from plain (not virtual) temperatures, a little low in moist air; fine for a glance, upgrade if numbers must match SPC.
  let cape = 0;
  for (let i = 1; i < levels.length; i++) {
    const buoy = (j: number) => (G * (path[j].t - levels[j].t)) / (levels[j].t + 273.15);
    const mean = (buoy(i - 1) + buoy(i)) / 2;
    if (mean > 0) cape += mean * (levels[i].z - levels[i - 1].z);
  }
  const zAt = (target: number) => {
    const i = levels.findIndex((l) => l.p < target);
    if (i <= 0) return levels[0].z;
    const f = Math.log(levels[i - 1].p / target) / Math.log(levels[i - 1].p / levels[i].p);
    return levels[i - 1].z + f * (levels[i].z - levels[i - 1].z);
  };
  return { path, lclP, lclAgl: zAt(lclP) - sfc.z, cape };
}
