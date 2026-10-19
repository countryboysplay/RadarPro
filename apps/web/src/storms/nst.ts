// NWS Storm Tracking Information (Level 3 product 58, "NST"): each storm
// cell's current position, past track and 15-60 min forecast track, plus the
// average storm motion. Latest issue only, from tgftp (see `fetchNst`).

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
}

export interface StormTracks {
  scanTimeMs: number;
  cells: StormCell[];
  avgMotion: StormMotionReport | null;
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

/** Parse a raw NST product (WMO/AWIPS text header allowed in front). Throws on anything else. */
export function parseNst(bytes: Uint8Array): StormTracks {
  const v = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  // The message header starts with the product code (58); its Product
  // Description Block starts 18 bytes later with a -1 divider.
  let m = 0;
  while (m < 64 && !(v.getUint16(m) === 58 && v.getInt16(m + 18) === -1)) m++;
  if (m === 64) throw new Error("not a storm tracking (NST) product");

  const radar: LonLat = [v.getInt32(m + 24) / 1000, v.getInt32(m + 20) / 1000];
  // Days since 1969-12-31 (day 1 is 1970-01-01), then seconds of day.
  const scanTimeMs = (v.getUint16(m + 40) - 1) * 86_400_000 + v.getUint32(m + 42) * 1000;
  // Packet coordinates are 1/4 km, +I east, +J north of the radar.
  const point = (o: number) => offset(radar, v.getInt16(o) / 4, v.getInt16(o + 2) / 4);

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
  const symbology = m + v.getUint32(m + 108) * 2;
  let p = symbology + 10;
  for (let layer = v.getUint16(symbology + 8); layer > 0; layer--) {
    const layerEnd = p + 6 + v.getUint32(p + 2);
    p += 6;
    // Each cell's ID packet (15) precedes its own past/forecast packets.
    for (; p < layerEnd; p += 4 + v.getUint16(p + 2)) {
      const code = v.getUint16(p);
      const cell = cells[cells.length - 1];
      if (code === 15) {
        cells.push({ id: String.fromCharCode(bytes[p + 8], bytes[p + 9]), at: point(p + 4), past: [], forecast: [], motion: null });
      } else if (code === 23 && cell) {
        cell.past = track(p + 4, p + 4 + v.getUint16(p + 2));
      } else if (code === 24 && cell) {
        cell.forecast = track(p + 4, p + 4 + v.getUint16(p + 2));
        cell.motion = trackMotion(cell.forecast);
      }
    }
  }

  const avg = /AVG SPEED\s+(\d+)\s+KTS\s+AVG DIRECTION\s+(\d+)\s+DEG/.exec(new TextDecoder("latin1").decode(bytes));
  return {
    scanTimeMs,
    cells,
    avgMotion: avg ? { speedKts: Number(avg[1]), fromDeg: Number(avg[2]) } : null,
  };
}

/** Latest NST for `icao`. tgftp has no CORS: desktop relays it, the browser build goes through Vite's `/tgftp` proxy. */
export async function fetchNst(icao: string, signal?: AbortSignal): Promise<Uint8Array> {
  const site = icao.toLowerCase();
  if (isDesktop()) {
    const { invoke } = await import("@tauri-apps/api/core");
    return new Uint8Array(await invoke<ArrayBuffer>("nws_radar_product", { product: "58sti", site }));
  }
  const response = await fetchWithRetry(`/tgftp/SL.us008001/DF.of/DC.radar/DS.58sti/SI.${site}/sn.last`, { signal });
  return new Uint8Array(await response.arrayBuffer());
}
