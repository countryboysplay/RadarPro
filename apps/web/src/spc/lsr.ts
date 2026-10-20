// NWS Local Storm Reports from IEM (CORS-open, any time window), as a
// placefile layer with a generated icon sheet, so drawing and hover reuse
// `placefiles/draw.ts`.

import type { PlaceItem } from "../placefiles/placefile";
import type { PlacefileLayer } from "../placefiles/usePlacefiles";
import { getJson } from "./spc";

/** Reports shown: what a chaser acts on. Icon number, letter and SPC's map colour. */
const KINDS: Record<string, { icon: number; letter: string; color: string; name: string }> = {
  TORNADO: { icon: 1, letter: "T", color: "#e8283a", name: "Tornado" },
  "FUNNEL CLOUD": { icon: 2, letter: "C", color: "#f08a3c", name: "Funnel cloud" },
  HAIL: { icon: 3, letter: "H", color: "#2fae4f", name: "Hail" },
  "TSTM WND DMG": { icon: 4, letter: "W", color: "#2f6fe0", name: "Wind damage" },
  "TSTM WND GST": { icon: 5, letter: "G", color: "#2f6fe0", name: "Wind gust" },
  "FLASH FLOOD": { icon: 6, letter: "F", color: "#1f8f8f", name: "Flash flood" },
};

/** How far back reports go, live and before an archived scan. */
export const LSR_WINDOW_HOURS = 3;
const ICON_PX = 18;

let sheet: ImageBitmap | null = null;

/** One row of round markers, one per kind, drawn once. */
function iconSheet(): ImageBitmap {
  if (sheet) return sheet;
  const kinds = Object.values(KINDS);
  const canvas = new OffscreenCanvas(ICON_PX * kinds.length, ICON_PX);
  const ctx = canvas.getContext("2d")!;
  ctx.font = `bold 11px sans-serif`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  for (const k of kinds) {
    const cx = (k.icon - 0.5) * ICON_PX;
    const cy = ICON_PX / 2;
    ctx.beginPath();
    ctx.arc(cx, cy, ICON_PX / 2 - 1.5, 0, 2 * Math.PI);
    ctx.fillStyle = k.color;
    ctx.fill();
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = "#ffffff";
    ctx.stroke();
    ctx.fillStyle = "#ffffff";
    ctx.fillText(k.letter, cx, cy + 0.5);
  }
  sheet = canvas.transferToImageBitmap();
  return sheet;
}

interface LsrProps {
  typetext: string;
  magf: number | null;
  unit: string | null;
  qualifier: string | null;
  city: string;
  county: string;
  st: string;
  source: string;
  remark: string | null;
  valid: string;
  lon: number;
  lat: number;
}

/** "Hail 1.75 in", "Wind gust 70 mph (est.)", "Tornado". */
function headline(p: LsrProps, name: string): string {
  if (p.magf === null || p.magf === 0) return name;
  const unit = p.unit === "Inch" ? "in" : (p.unit ?? "").toLowerCase();
  const estimated = p.qualifier === "E" ? " (est.)" : "";
  return `${name} ${p.magf} ${unit}${estimated}`.trim();
}

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

export function lsrLayer(features: { properties: LsrProps }[], url: string): PlacefileLayer {
  const items: PlaceItem[] = [];
  // Oldest first, so the newest report draws on top and wins the hover.
  const sorted = [...features].sort((a, b) => Date.parse(a.properties.valid) - Date.parse(b.properties.valid));
  for (const { properties: p } of sorted) {
    const kind = KINDS[p.typetext];
    if (!kind || !Number.isFinite(p.lon) || !Number.isFinite(p.lat)) continue;
    const hover = [
      `${headline(p, kind.name)} · ${time(Date.parse(p.valid))}`,
      `${p.city}, ${p.county} Co., ${p.st}`,
      `Source: ${p.source}`,
      p.remark ?? "",
    ]
      .filter(Boolean)
      .join("\n");
    items.push({ kind: "icon", anchor: null, threshold: 999, color: "", at: [p.lon, p.lat], angle: 0, file: 1, index: kind.icon, hover });
  }
  return {
    url,
    file: {
      title: "Storm reports",
      refreshSeconds: 0,
      iconFiles: new Map([[1, { url: "", width: ICON_PX, height: ICON_PX, hotX: ICON_PX / 2, hotY: ICON_PX / 2 }]]),
      fonts: new Map(),
      items,
    },
    sheets: new Map([[1, iconSheet()]]),
  };
}

/** Reports in the `LSR_WINDOW_HOURS` before `atMs` (now when null). */
export async function fetchLsrs(atMs: number | null, signal: AbortSignal): Promise<PlacefileLayer> {
  const iso = (ms: number) => new Date(ms).toISOString().slice(0, 16) + "Z";
  const end = atMs ?? Date.now();
  const query = atMs === null ? `hours=${LSR_WINDOW_HOURS}` : `sts=${iso(end - LSR_WINDOW_HOURS * 3_600_000)}&ets=${iso(end)}`;
  const url = `https://mesonet.agron.iastate.edu/geojson/lsr.geojson?${query}`;
  // An archived window (sts/ets) never changes; getJson caches it like `valid=` answers.
  const data = await getJson<{ features: { properties: LsrProps }[] }>(url, signal);
  return lsrLayer(data.features, url);
}
