// GRLevelX placefile parser (http://www.grlevelx.com/manuals/gis/files_places.htm).
// Handles Title, Refresh/RefreshSeconds, Threshold, Color, Font, IconFile,
// Icon, Text, Place, Line, Polygon and Object blocks; Triangles and Image
// blocks are skipped.

export type LonLat = [number, number];

export interface PlaceIconFile {
  url: string;
  width: number;
  height: number;
  hotX: number;
  hotY: number;
}

export interface PlaceFont {
  px: number;
  bold: boolean;
  italic: boolean;
  face: string;
}

/** Inside an `Object:` block, `anchor` is the object's position and
 * `points`/`at` are pixel offsets [x right, y up]; otherwise `anchor` is
 * null and they are [lon, lat]. */
interface PlaceBase {
  anchor: LonLat | null;
  /** Hidden when the view is wider than this many miles. */
  threshold: number;
  color: string;
  hover: string;
}

export type PlaceItem =
  | (PlaceBase & {
      kind: "icon";
      at: LonLat;
      angle: number;
      file: number;
      index: number;
    })
  | (PlaceBase & { kind: "text"; at: LonLat; font: number; text: string })
  | (PlaceBase & { kind: "line"; points: LonLat[]; width: number })
  | (PlaceBase & { kind: "polygon"; contours: LonLat[][] });

export interface Placefile {
  title: string;
  /** 0 when the file asks for no refresh. */
  refreshSeconds: number;
  iconFiles: Map<number, PlaceIconFile>;
  fonts: Map<number, PlaceFont>;
  items: PlaceItem[];
}

/** Splits on commas outside double quotes; quoted fields lose their quotes. */
function fields(args: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  for (const ch of args) {
    if (ch === '"') quoted = !quoted;
    else if (ch === "," && !quoted) {
      out.push(cur.trim());
      cur = "";
    } else cur += ch;
  }
  out.push(cur.trim());
  return out;
}

const unescape = (s: string | undefined) => (s ?? "").replace(/\\n/g, "\n");

function rgba(parts: string[]): string {
  const [r, g, b] = parts.map(Number);
  const a = parts.length > 3 ? Number(parts[3]) / 255 : 1;
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

/** Placefile coordinates are lat, lon (or x, y inside an object). */
function point(lat: string, lon: string, inObject: boolean): LonLat | null {
  const a = Number(lat);
  const b = Number(lon);
  if (!Number.isFinite(a) || !Number.isFinite(b)) return null;
  return inObject ? [a, b] : [b, a];
}

export function parsePlacefile(text: string, baseUrl = ""): Placefile {
  const file: Placefile = {
    title: "",
    refreshSeconds: 0,
    iconFiles: new Map(),
    fonts: new Map(),
    items: [],
  };
  let threshold = 999;
  let color = "rgba(255, 255, 255, 1)";
  let anchor: LonLat | null = null;
  // The Line/Polygon being read, or "skip" inside Triangles/Image.
  let block: Extract<PlaceItem, { kind: "line" | "polygon" }> | "skip" | null = null;
  let contour: LonLat[] = [];

  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith(";") || line.startsWith("//")) continue;
    const colon = line.indexOf(":");
    const key = colon > 0 && /^[A-Za-z]+$/.test(line.slice(0, colon)) ? line.slice(0, colon).toLowerCase() : null;
    const args = key ? line.slice(colon + 1).trim() : line;

    if (key === "end") {
      if (block && block !== "skip") {
        if (block.kind === "polygon" && contour.length > 0) block.contours.push(contour);
        const ok = block.kind === "line" ? block.points.length >= 2 : block.contours.length > 0;
        if (ok) file.items.push(block);
        block = null;
      } else if (block === "skip") block = null;
      else anchor = null;
      continue;
    }
    if (block && !key) {
      if (block === "skip") continue;
      const f = fields(args);
      const p = point(f[0], f[1], anchor !== null);
      if (!p) continue;
      if (block.kind === "line") block.points.push(p);
      else {
        // shortcut: one fill color per polygon (first vertex's), not GR's per-vertex blend.
        if (block.contours.length === 0 && contour.length === 0 && f.length >= 5) block.color = rgba(f.slice(2, 6));
        // Repeating a contour's first point closes it; the next point starts a new one.
        const first = contour[0];
        contour.push(p);
        if (contour.length > 2 && first[0] === p[0] && first[1] === p[1]) {
          block.contours.push(contour);
          contour = [];
        }
      }
      continue;
    }

    const f = fields(args);
    const base = () => ({ anchor, threshold, color });
    switch (key) {
      case "title":
        file.title = args;
        break;
      case "refresh":
        file.refreshSeconds = Number(args) * 60 || 0;
        break;
      case "refreshseconds":
        file.refreshSeconds = Number(args) || 0;
        break;
      case "threshold":
        threshold = Number(args) || 999;
        break;
      case "color":
        color = rgba(args.split(/[\s,]+/));
        break;
      case "font":
        file.fonts.set(Number(f[0]), {
          px: Number(f[1]) || 11,
          bold: (Number(f[2]) & 1) !== 0,
          italic: (Number(f[2]) & 2) !== 0,
          face: f[3] || "sans-serif",
        });
        break;
      case "iconfile": {
        let url = f[5] ?? "";
        try {
          url = new URL(url, baseUrl || undefined).href;
        } catch {
          // Not a URL (a local GR path); the icons simply won't load.
        }
        file.iconFiles.set(Number(f[0]), {
          url,
          width: Number(f[1]),
          height: Number(f[2]),
          hotX: Number(f[3]),
          hotY: Number(f[4]),
        });
        break;
      }
      case "icon": {
        const at = point(f[0], f[1], anchor !== null);
        if (at)
          file.items.push({
            ...base(),
            kind: "icon",
            at,
            angle: Number(f[2]) || 0,
            file: Number(f[3]),
            index: Number(f[4]),
            hover: unescape(f[5]),
          });
        break;
      }
      case "text": {
        const at = point(f[0], f[1], anchor !== null);
        if (at)
          file.items.push({
            ...base(),
            kind: "text",
            at,
            font: Number(f[2]),
            text: unescape(f[3]),
            hover: unescape(f[4]),
          });
        break;
      }
      case "place": {
        const at = point(f[0], f[1], anchor !== null);
        const label = f.slice(2).join(",").trim();
        if (at)
          file.items.push({
            ...base(),
            kind: "text",
            at,
            font: 1,
            text: label,
            hover: "",
          });
        break;
      }
      case "line":
        block = {
          ...base(),
          kind: "line",
          points: [],
          width: Number(f[0]) || 1,
          hover: unescape(f[2]),
        };
        break;
      case "polygon":
        block = { ...base(), kind: "polygon", contours: [], hover: "" };
        contour = [];
        break;
      case "triangles":
      case "image":
        block = "skip";
        break;
      case "object":
        anchor = point(f[0], f[1], false);
        break;
    }
  }
  return file;
}
