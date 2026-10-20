// SPC products chasers read: the Day 1 outlook (SPC's own GeoJSON and
// colours), and watch and mesoscale discussion outlines (IEM, which also
// serves them for any past time). All three send CORS headers.
//
// Each becomes a `PlacefileLayer`, so drawing and hover text reuse
// `placefiles/draw.ts`.

import type { LonLat, PlaceItem } from "../placefiles/placefile";
import type { PlacefileLayer } from "../placefiles/usePlacefiles";

export type OutlookKind = "off" | "cat" | "torn" | "wind" | "hail";

export const OUTLOOK_NAMES: Record<OutlookKind, string> = {
  off: "Off",
  cat: "Categorical",
  torn: "Tornado",
  wind: "Wind",
  hail: "Hail",
};

const IEM_API = "https://mesonet.agron.iastate.edu/api/1";

/** NWS standard watch colours. */
const WATCH_COLORS: Record<string, string> = { TOR: "#ffff00", SVR: "#db7093" };
const MD_COLOR = "#4fb3ff";

type Rings = LonLat[][];
interface Geometry {
  type: string;
  coordinates?: unknown;
  geometries?: Geometry[];
}
interface Feature<P> {
  geometry: Geometry | null;
  properties: P;
}

/** Every ring of a Polygon, MultiPolygon or GeometryCollection of them. */
function rings(g: Geometry | null): Rings {
  if (!g) return [];
  if (g.type === "Polygon") return g.coordinates as Rings;
  if (g.type === "MultiPolygon") return (g.coordinates as Rings[]).flat();
  if (g.type === "GeometryCollection") return (g.geometries ?? []).flatMap(rings);
  return [];
}

function rgba(hex: string, alpha: number): string {
  const n = parseInt(hex.replace("#", ""), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
}

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

/** A filled area (hover anywhere inside) plus its outline. */
function area(contours: Rings, fill: string, stroke: string, width: number, hover: string): PlaceItem[] {
  if (contours.length === 0) return [];
  const base = { anchor: null, threshold: 999 };
  return [
    { ...base, kind: "polygon", contours, color: fill, hover },
    ...contours.map((points): PlaceItem => ({ ...base, kind: "line", points, width, color: stroke, hover: "" })),
  ];
}

function layer(url: string, title: string, items: PlaceItem[]): PlacefileLayer {
  return { url, file: { title, refreshSeconds: 0, iconFiles: new Map(), fonts: new Map(), items }, sheets: new Map() };
}

/** Archived answers (`valid=`/`ts=`, or an `ets=` end time) never change; kept for the session so loops reuse them. */
// shortcut: reports filed late for a replay of the last hour or two stay missing until restart; fine for older events.
const archived = new Map<string, Promise<unknown>>();

export async function getJson<T>(url: string, signal: AbortSignal): Promise<T> {
  const cached = archived.get(url);
  if (cached) return cached as Promise<T>;
  // Archived answers are shared through the cache, so they run to the end rather than die with one caller.
  const isArchived = /[?&](valid|ets|ts)=/.test(url);
  const request = fetch(url, isArchived ? undefined : { signal }).then((response) => {
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return response.json() as Promise<T>;
  });
  if (isArchived) {
    archived.set(url, request);
    request.catch(() => archived.delete(url));
  }
  return request;
}

interface OutlookProps {
  LABEL: string;
  LABEL2: string;
  stroke: string;
  fill: string;
}

const SIGNIFICANT: Record<Exclude<OutlookKind, "off">, string> = { cat: "severe", torn: "(EF2+) tornadoes", wind: "(65 kt+) wind", hail: "(2 in+) hail" };

/** SPC's live Day 1 outlook; areas come lowest risk first, so higher risk draws on top. */
export async function fetchOutlook(kind: Exclude<OutlookKind, "off">, signal: AbortSignal): Promise<PlacefileLayer> {
  const url = `https://www.spc.noaa.gov/products/outlook/day1otlk_${kind}.lyr.geojson`;
  const data = await getJson<{ features: Feature<OutlookProps>[] }>(url, signal);
  const items = data.features.flatMap(({ geometry, properties: p }) => {
    const label = `SPC Day 1: ${p.LABEL2 || p.LABEL}`;
    // "SIGN" (significant severe) is black hatching on SPC's maps; a white outline reads on the dark map.
    if (p.LABEL === "SIGN") return area(rings(geometry), "rgba(0, 0, 0, 0)", "rgba(255, 255, 255, 0.85)", 2, `SPC Day 1: significant ${SIGNIFICANT[kind]} possible`);
    if (!p.fill) return [];
    return area(rings(geometry), rgba(p.fill, 0.28), p.stroke || p.fill, 1.5, label);
  });
  return layer(url, "SPC outlook", items);
}

/** `valid=` for IEM: the products in effect at that time (omitted for now). */
const validParam = (atMs: number | null) => (atMs === null ? "" : `valid=${new Date(atMs).toISOString().slice(0, 19)}Z`);

interface WatchProps {
  type: string;
  num: string;
  utc_expired: string;
}

export async function fetchWatches(atMs: number | null, signal: AbortSignal): Promise<PlacefileLayer> {
  const url = `${IEM_API}/spc_watch_outline.geojson?${validParam(atMs)}`;
  const data = await getJson<{ features: Feature<WatchProps>[] }>(url, signal);
  const items = data.features.flatMap(({ geometry, properties: p }) => {
    const color = WATCH_COLORS[p.type] ?? "#ffffff";
    const name = p.type === "TOR" ? "Tornado Watch" : p.type === "SVR" ? "Severe Thunderstorm Watch" : `${p.type} Watch`;
    return area(rings(geometry), rgba(color, 0.06), color, 2.5, `${name} ${p.num}\nuntil ${time(Date.parse(p.utc_expired))}`);
  });
  return layer(url, "Watches", items);
}

export interface MesoscaleDiscussion {
  num: string;
  productId: string;
  concerning: string;
  /** Percent chance SPC issues a watch, or null when it doesn't say. */
  watchChance: number | null;
  expiresMs: number;
}

interface MdProps {
  num: string;
  product_id: string;
  concerning: string;
  watch_confidence: string | null;
  expire: string;
}

/** Sentence case for SPC's capitals ("SEVERE POTENTIAL...WATCH POSSIBLE"). */
const sentence = (s: string) => s.charAt(0) + s.slice(1).toLowerCase();

export async function fetchMds(atMs: number | null, signal: AbortSignal): Promise<{ layer: PlacefileLayer; mds: MesoscaleDiscussion[] }> {
  const url = `${IEM_API}/nws/spc_mcd.geojson?${validParam(atMs)}`;
  const data = await getJson<{ features: Feature<MdProps>[] }>(url, signal);
  const mds: MesoscaleDiscussion[] = [];
  const items = data.features.flatMap(({ geometry, properties: p }) => {
    const chance = Number(p.watch_confidence);
    const md = {
      num: String(Number(p.num)),
      productId: p.product_id,
      concerning: sentence(p.concerning ?? ""),
      watchChance: Number.isFinite(chance) && p.watch_confidence !== null ? chance : null,
      expiresMs: Date.parse(p.expire),
    };
    mds.push(md);
    const lines = [`Mesoscale Discussion ${md.num}`, md.concerning, md.watchChance !== null ? `Watch chance ${md.watchChance}%` : "", `until ${time(md.expiresMs)}`];
    return area(rings(geometry), rgba(MD_COLOR, 0.06), MD_COLOR, 2, lines.filter(Boolean).join("\n"));
  });
  return { layer: layer(url, "Mesoscale discussions", items), mds };
}

/** Full text of an NWS product (an MD) by its IEM product id. */
export async function fetchProductText(productId: string, signal?: AbortSignal): Promise<string> {
  const response = await fetch(`${IEM_API}/nwstext/${encodeURIComponent(productId)}`, { signal });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.text();
}
