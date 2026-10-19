// NWS Level 3 storm products. Storm Tracking Information (58, "NST"): each
// storm cell's current position, past track and 15-60 min forecast track,
// plus the average storm motion. Tornado Vortex Signature (61, "NTV"), Hail
// Index (59, "NHI") and Mesocyclone (141, "NMD") add TVS markers, hail
// estimates and mesocyclone circles. Latest issue
// only, from tgftp (see `fetchStormProduct`).

import { isDesktop } from "../platform/desktop";
import { fetchWithRetry } from "../nexrad/bucket";
import { legLengthAndBearing } from "../ui/CrossSectionPanel";

export type LonLat = [number, number];

/** `fromDeg` is where the storm moves *from*, as NWS reports it. */
export interface StormMotionReport {
  speedKts: number;
  fromDeg: number;
}

export interface StormCell {
  /** Two-character NWS cell ID, e.g. "Q8". */
  id: string;
  at: LonLat;
  /** Oldest-last, starting at `at`. Empty for a new cell. */
  past: LonLat[];
  /** 15-minute steps, starting at `at`. Empty when there is no forecast. */
  forecast: LonLat[];
  /** From the forecast track; null when there is none (a new cell). */
  motion: StormMotionReport | null;
  /** From the hail index product, matched by cell ID; null when not reported. */
  hail: HailEstimate | null;
}

export interface HailEstimate {
  /** Probability of severe hail (1"+), percent. */
  severeChance: number;
  /** Probability of any hail, percent. */
  chance: number;
  /** Maximum expected hail size, inches; 0 when under 0.50". */
  sizeIn: number;
}

export interface TvsDetection {
  /** Storm cell ID the TVS belongs to. */
  stormId: string;
  at: LonLat;
  /** An elevated TVS (ETVS): rotation aloft, not yet near the ground. */
  elevated: boolean;
}

export interface StormTracks {
  scanTimeMs: number;
  cells: StormCell[];
  avgMotion: StormMotionReport | null;
  /** From the TVS product; empty when none, or when that product is unavailable. */
  tvs: TvsDetection[];
  /** From the mesocyclone product, likewise. */
  mesos: Mesocyclone[];
}

/** A strength rank 5+ circulation from the Mesocyclone Detection Algorithm. */
export interface Mesocyclone {
  /** NWS circulation ID, e.g. "284". */
  id: string;
  at: LonLat;
  radiusKm: number;
  /** Base within 1 km of the ground (or on the lowest tilt): the dangerous kind. */
  lowLevel: boolean;
}

const EARTH_RADIUS_KM = 6371;

/** The point `eastKm`/`northKm` from `[lon, lat]`, along a great circle. */
function offset([lon, lat]: LonLat, eastKm: number, northKm: number): LonLat {
  const rad = Math.PI / 180;
  const d = Math.hypot(eastKm, northKm) / EARTH_RADIUS_KM;
  const b = Math.atan2(eastKm, northKm);
  const φ1 = lat * rad;
  const φ2 = Math.asin(Math.sin(φ1) * Math.cos(d) + Math.cos(φ1) * Math.sin(d) * Math.cos(b));
  const λ2 = lon * rad + Math.atan2(Math.sin(b) * Math.sin(d) * Math.cos(φ1), Math.cos(d) - Math.sin(φ1) * Math.sin(φ2));
  return [λ2 / rad, φ2 / rad];
}

/** Motion along a forecast track of 15-minute steps, first to last point
 * (the longest span, since points are only 1/4 km precise). */
function trackMotion(forecast: LonLat[]): StormMotionReport | null {
  if (forecast.length < 2) return null;
  const [a, b] = [forecast[0], forecast[forecast.length - 1]];
  const { km, deg } = legLengthAndBearing({ lon: a[0], lat: a[1] }, { lon: b[0], lat: b[1] });
  const hours = (forecast.length - 1) / 4;
  return { speedKts: Math.round(km / 1.852 / hours), fromDeg: Math.round(deg + 180) % 360 };
}

/** The header fields and packet reader shared by every Level 3 storm product. */
function readProduct(bytes: Uint8Array, code: number, name: string) {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  // The message header starts with the product code; its Product
  // Description Block starts 18 bytes later with a -1 divider.
  let m = 0;
  while (m < 64 && !(v.getUint16(m) === code && v.getInt16(m + 18) === -1)) m++;
  if (m === 64) throw new Error(`not a ${name} product`);

  const radar: LonLat = [v.getInt32(m + 24) / 1000, v.getInt32(m + 20) / 1000];
  // Days since 1969-12-31 (day 1 is 1970-01-01), then seconds of day.
  const scanTimeMs = (v.getUint16(m + 40) - 1) * 86_400_000 + v.getUint32(m + 42) * 1000;
  // Packet coordinates are 1/4 km, +I east, +J north of the radar.
  const point = (o: number) => offset(radar, v.getInt16(o) / 4, v.getInt16(o + 2) / 4);

  /** Calls `visit(code, start)` for every top-level symbology packet; `start` is just past its 4-byte code/length. */
  function packets(visit: (code: number, start: number, length: number) => void) {
    const symbology = m + v.getUint32(m + 108) * 2;
    let p = symbology + 10;
    for (let layer = v.getUint16(symbology + 8); layer > 0; layer--) {
      const layerEnd = p + 6 + v.getUint32(p + 2);
      for (p += 6; p < layerEnd; p += 4 + v.getUint16(p + 2)) visit(v.getUint16(p), p + 4, v.getUint16(p + 2));
    }
  }

  return { v, scanTimeMs, point, packets, text: () => new TextDecoder("latin1").decode(bytes) };
}

/** Parse a raw NST product (WMO/AWIPS text header allowed in front). Throws on anything else. */
export function parseNst(bytes: Uint8Array): StormTracks {
  const { v, scanTimeMs, point, packets, text } = readProduct(bytes, 58, "storm tracking (NST)");

  // Within a past (23) or forecast (24) packet, the linked vector (6) holds the track.
  function track(start: number, end: number): LonLat[] {
    for (let p = start; p < end; p += 4 + v.getUint16(p + 2)) {
      if (v.getUint16(p) !== 6) continue;
      const out: LonLat[] = [];
      for (let o = p + 4; o < p + 4 + v.getUint16(p + 2); o += 4) out.push(point(o));
      return out;
    }
    return [];
  }

  const cells: StormCell[] = [];
  // Each cell's ID packet (15) precedes its own past/forecast packets.
  packets((code, start, length) => {
    const cell = cells[cells.length - 1];
    if (code === 15) {
      cells.push({ id: String.fromCharCode(bytes[start + 4], bytes[start + 5]), at: point(start), past: [], forecast: [], motion: null, hail: null });
    } else if (code === 23 && cell) {
      cell.past = track(start, start + length);
    } else if (code === 24 && cell) {
      cell.forecast = track(start, start + length);
      cell.motion = trackMotion(cell.forecast);
    }
  });

  const avg = /AVG SPEED\s+(\d+)\s+KTS\s+AVG DIRECTION\s+(\d+)\s+DEG/.exec(text());
  return {
    scanTimeMs,
    cells,
    avgMotion: avg ? { speedKts: Number(avg[1]), fromDeg: Number(avg[2]) } : null,
    tvs: [],
    mesos: [],
  };
}

/** Parse a raw TVS product (61, NTV): each TVS (packet 12) and elevated TVS (26) at its own position. */
export function parseTvs(bytes: Uint8Array): { scanTimeMs: number; tvs: TvsDetection[] } {
  const { scanTimeMs, point, packets } = readProduct(bytes, 61, "tornado vortex signature (NTV)");
  const tvs: TvsDetection[] = [];
  // Each TVS is followed by its storm ID packet (15).
  packets((code, start) => {
    if (code === 12 || code === 26) tvs.push({ stormId: "", at: point(start), elevated: code === 26 });
    else if (code === 15 && tvs.length > 0) tvs[tvs.length - 1].stormId = String.fromCharCode(bytes[start + 4], bytes[start + 5]);
  });
  return { scanTimeMs, tvs };
}

/** Parse a raw mesocyclone product (141, NMD): each rank 5+ circulation, as a
 * point feature (20) of type 9 (low base) or 10 (higher base) followed by
 * its ID in a text packet (8). Weaker circulations (type 11) are left out. */
export function parseMeso(bytes: Uint8Array): { scanTimeMs: number; mesos: Mesocyclone[] } {
  const { v, scanTimeMs, point, packets } = readProduct(bytes, 141, "mesocyclone (NMD)");
  const mesos: Mesocyclone[] = [];
  let last: Mesocyclone | null = null;
  packets((code, start, length) => {
    if (code === 20) {
      const type = v.getUint16(start + 4);
      last = type === 9 || type === 10 ? { id: "", at: point(start), radiusKm: v.getUint16(start + 6) / 4, lowLevel: type === 9 } : null;
      if (last) mesos.push(last);
    } else if (code === 8 && last) {
      last.id = String.fromCharCode(...bytes.subarray(start + 6, start + length)).trim();
      last = null;
    }
  });
  return { scanTimeMs, mesos };
}

/** Parse a raw hail index product (59, NHI) into estimates by storm cell ID. Its
 * binary packets store hail size in whole inches, so this reads the text table (0.01"). */
export function parseHail(bytes: Uint8Array): { scanTimeMs: number; hail: Map<string, HailEstimate> } {
  const { scanTimeMs, text } = readProduct(bytes, 59, "hail index (NHI)");
  const hail = new Map<string, HailEstimate>();
  // Rows have no newlines: each starts with its 2-byte length, 80 (0x0050, reads as "P"), e.g.
  // "P        L8             100                  100                 2.75". "<0.50" reads as 0;
  // cells NWS couldn't assess say UNKNOWN and are skipped.
  for (const [, id, posh, poh, below, size] of text().matchAll(/P\s+([A-Z]\d)\s+(\d+)\s+(\d+)\s+(<?)([\d.]+)/g)) {
    hail.set(id, { severeChance: Number(posh), chance: Number(poh), sizeIn: below ? 0 : Number(size) });
  }
  return { scanTimeMs, hail };
}

/** NWS radar products this app reads, by their tgftp directory name. */
export type StormProduct = "58sti" | "61tvs" | "p59hi" | "141md";

/** Latest issue of `product` for `icao`. tgftp has no CORS: desktop relays it, the browser build goes through Vite's `/tgftp` proxy. */
export async function fetchStormProduct(product: StormProduct, icao: string, signal?: AbortSignal): Promise<Uint8Array> {
  const site = icao.toLowerCase();
  if (isDesktop()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return new Uint8Array(await invoke<ArrayBuffer>("nws_radar_product", { product, site }));
  }
  const response = await fetchWithRetry(`/tgftp/SL.us008001/DF.of/DC.radar/DS.${product}/SI.${site}/sn.last`, { signal });
  return new Uint8Array(await response.arrayBuffer());
}
