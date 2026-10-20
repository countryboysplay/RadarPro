import type { MapLibreMap } from "maplibre-gl";
import type { LonLat, PlaceItem } from "./placefile";
import type { PlacefileLayer } from "./usePlacefiles";

const KM_PER_MILE = 1.609344;
/** Hover reach (px) around an icon's hotspot, a text's centre or a line. */
const HOVER_RADIUS_PX = 12;

/** Half the view's width in miles, the "range" GR's Threshold is compared to.
 * shortcut: measured along the centre latitude, fine for a north-up map. */
function viewRangeMiles(map: MapLibreMap): number {
  const b = map.getBounds();
  const lat = (b.getNorth() + b.getSouth()) / 2;
  const km = ((b.getEast() - b.getWest()) / 2) * 111.32 * Math.cos((lat * Math.PI) / 180);
  return km / KM_PER_MILE;
}

/** Screen position of an item's point: geographic, or a pixel offset (y up) from its object. */
function screen(map: MapLibreMap, item: PlaceItem, pt: LonLat): { x: number; y: number } {
  if (!item.anchor) return map.project(pt);
  const a = map.project(item.anchor);
  return { x: a.x + pt[0], y: a.y - pt[1] };
}

function visible(map: MapLibreMap, layers: PlacefileLayer[]): { layer: PlacefileLayer; item: PlaceItem }[] {
  const range = viewRangeMiles(map);
  return layers.flatMap((layer) => layer.file.items.filter((item) => range <= item.threshold).map((item) => ({ layer, item })));
}

/** Placefile shapes, icons and text, in file order (later items on top). */
export function drawPlacefiles(map: MapLibreMap, canvas: HTMLCanvasElement, layers: PlacefileLayer[]) {
  const ctx = canvas.getContext("2d");
  if (!ctx || layers.length === 0) return;
  const { width, height } = canvas;
  ctx.save();
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  for (const { layer, item } of visible(map, layers)) {
    if (item.kind === "line") {
      ctx.strokeStyle = item.color;
      ctx.lineWidth = item.width;
      ctx.beginPath();
      item.points.forEach((pt, i) => {
        const p = screen(map, item, pt);
        if (i === 0) ctx.moveTo(p.x, p.y);
        else ctx.lineTo(p.x, p.y);
      });
      ctx.stroke();
    } else if (item.kind === "polygon") {
      ctx.fillStyle = item.color;
      ctx.beginPath();
      for (const contour of item.contours) {
        contour.forEach((pt, i) => {
          const p = screen(map, item, pt);
          if (i === 0) ctx.moveTo(p.x, p.y);
          else ctx.lineTo(p.x, p.y);
        });
        ctx.closePath();
      }
      ctx.fill("evenodd");
    } else if (item.kind === "icon") {
      const icons = layer.file.iconFiles.get(item.file);
      const sheet = layer.sheets.get(item.file);
      if (!icons || !sheet || item.index < 1) continue;
      const p = screen(map, item, item.at);
      if (p.x < -64 || p.y < -64 || p.x > width + 64 || p.y > height + 64) continue;
      const cols = Math.max(1, Math.floor(sheet.width / icons.width));
      const sx = ((item.index - 1) % cols) * icons.width;
      const sy = Math.floor((item.index - 1) / cols) * icons.height;
      ctx.save();
      ctx.translate(p.x, p.y);
      ctx.rotate((item.angle * Math.PI) / 180);
      ctx.drawImage(sheet, sx, sy, icons.width, icons.height, -icons.hotX, -icons.hotY, icons.width, icons.height);
      ctx.restore();
    } else {
      const font = layer.file.fonts.get(item.font);
      const p = screen(map, item, item.at);
      ctx.font = `${font?.italic ? "italic " : ""}${font?.bold ? "bold " : ""}${font?.px ?? 11}px ${font?.face ? `"${font.face}", ` : ""}sans-serif`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      // Dark halo so text reads over any echo.
      ctx.strokeStyle = "rgba(0, 0, 0, 0.8)";
      ctx.lineWidth = 3;
      ctx.strokeText(item.text, p.x, p.y);
      ctx.fillStyle = item.color;
      ctx.fillText(item.text, p.x, p.y);
    }
  }
  ctx.restore();
}

function distToSegment2(p: { x: number; y: number }, a: { x: number; y: number }, b: { x: number; y: number }): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2));
  return (p.x - a.x - t * dx) ** 2 + (p.y - a.y - t * dy) ** 2;
}

/** Hover text of the topmost visible item at screen point `px`, or null. */
export function placeHoverAt(map: MapLibreMap, layers: PlacefileLayer[], px: { x: number; y: number }): string | null {
  const r2 = HOVER_RADIUS_PX * HOVER_RADIUS_PX;
  const items = visible(map, layers);
  for (let i = items.length - 1; i >= 0; i--) {
    const { item } = items[i];
    if (!item.hover || item.kind === "polygon") continue;
    if (item.kind === "line") {
      const pts = item.points.map((pt) => screen(map, item, pt));
      if (pts.some((a, j) => j > 0 && distToSegment2(px, pts[j - 1], a) <= r2)) return item.hover;
      continue;
    }
    const p = screen(map, item, item.at);
    if ((p.x - px.x) ** 2 + (p.y - px.y) ** 2 <= r2) return item.hover;
  }
  return null;
}
