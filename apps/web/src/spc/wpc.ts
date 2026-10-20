// WPC rainfall products: the Excessive Rainfall Outlook (Days 1-3) and the
// QPF rain forecast, from NOAA's map services (ArcGIS, CORS-open, GeoJSON).
// Each becomes a `PlacefileLayer`, like the SPC outlook.

import type { PlaceItem } from "../placefiles/placefile";
import type { PlacefileLayer } from "../placefiles/usePlacefiles";
import { area, getJson, layer, rgba, rings, type Feature, type WpcKind } from "./spc";

const SERVICES = "https://mapservices.weather.noaa.gov/vector/rest/services";
/** Layer ids in each map service. */
const LAYERS: Record<WpcKind, string> = {
  ero1: "hazards/wpc_precip_hazards/MapServer/0",
  ero2: "hazards/wpc_precip_hazards/MapServer/1",
  ero3: "hazards/wpc_precip_hazards/MapServer/2",
  qpf1: "precip/wpc_qpf/MapServer/1",
  qpf2: "precip/wpc_qpf/MapServer/2",
  qpf3: "precip/wpc_qpf/MapServer/3",
  qpf72: "precip/wpc_qpf/MapServer/9",
};

/** Risk colours as on WPC's ERO maps, by the first word of `outlook`. */
const ERO_COLORS: Record<string, string> = { Marginal: "#4fc35a", Slight: "#f2d23c", Moderate: "#e8283a", High: "#d35ee8" };

/** WPC's QPF colour scale (inches, lower bound of each band). */
const QPF_COLORS: [number, string][] = [
  [0.01, "#7fff00"], [0.1, "#00cd00"], [0.25, "#008b00"], [0.5, "#104e8b"], [0.75, "#1e90ff"], [1, "#00b2ee"],
  [1.25, "#00eeee"], [1.5, "#8968cd"], [1.75, "#912cee"], [2, "#8b008b"], [2.5, "#8b0000"], [3, "#cd0000"],
  [4, "#ee4000"], [5, "#ff7f00"], [7, "#cd8500"], [10, "#ffd700"], [15, "#ffff00"], [20, "#ffaeb9"],
];
const qpfColor = (inches: number) => QPF_COLORS.reduce((color, [min, c]) => (inches >= min ? c : color), QPF_COLORS[0][1]);

interface EroProps {
  outlook: string;
  dn: number;
  valid_time: string;
}

interface QpfProps {
  qpf: number;
  valid_time: string;
}

export async function fetchWpc(kind: WpcKind, signal: AbortSignal): Promise<PlacefileLayer> {
  // ~2 km simplification: the full-detail QPF is 7-16 MB, this is 0.5-1.2 MB.
  const url = `${SERVICES}/${LAYERS[kind]}/query?where=1%3D1&outFields=*&outSR=4326&maxAllowableOffset=0.02&geometryPrecision=3&f=geojson`;
  const day = kind === "qpf72" ? "Days 1-3" : `Day ${kind.slice(-1)}`;
  if (kind.startsWith("ero")) {
    const data = await getJson<{ features: Feature<EroProps>[] }>(url, signal);
    // Lowest risk first, so higher risk draws on top.
    const items = [...data.features]
      .sort((a, b) => a.properties.dn - b.properties.dn)
      .flatMap(({ geometry, properties: p }) => {
        const color = ERO_COLORS[p.outlook.split(" ")[0]] ?? "#ffffff";
        return area(rings(geometry), rgba(color, 0.28), color, 1.5, `WPC excessive rainfall ${day}: ${p.outlook}\n${p.valid_time}`);
      });
    return layer(url, "WPC excessive rainfall", items);
  }
  const data = await getJson<{ features: Feature<QpfProps>[] }>(url, signal);
  // Each contour covers everything at or above its amount. The next band's rings are
  // cut out of each fill (even-odd), so fills don't stack into an opaque blot. The hover
  // goes on the outlines, so it reads like a contour map rather than listing every band.
  const base = { anchor: null, threshold: 999 };
  const bands = [...data.features].sort((a, b) => a.properties.qpf - b.properties.qpf).map((f) => ({ p: f.properties, contours: rings(f.geometry) }));
  const items = bands.flatMap(({ p, contours }, i): PlaceItem[] => {
    const color = qpfColor(p.qpf);
    const hover = `WPC rain forecast ${day}: ${p.qpf} in or more\n${p.valid_time}`;
    return [
      { ...base, kind: "polygon", contours: [...contours, ...(bands[i + 1]?.contours ?? [])], color: rgba(color, 0.3), hover: "" },
      ...contours.map((points): PlaceItem => ({ ...base, kind: "line", points, width: 1, color, hover })),
    ];
  });
  return layer(url, "WPC rain forecast", items);
}
