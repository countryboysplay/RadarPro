import { useEffect, useRef, type RefObject } from "react";
// MapLibre GL JS >=5 has no default export; `MapLibreMap` is the library's
// own non-colliding alias for its `Map` class (distinct from the global
// JS `Map`).
import { MapLibreMap, type ExpressionSpecification } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { RADAR_SITES, type RadarSite } from "../sites";
import type { AlertFeatureCollection } from "../alerts/types";
import { SEVERITY_COLORS, severityRank } from "../alerts/types";
import { pointInAlertGeometry } from "../alerts/geometryHitTest";
import type { MyLocation } from "../location/useMyLocation";
import type { MrmsViewport } from "../mrms/types";
import { isDesktop } from "../platform/desktop";
import { registerDesktopRainbowProtocol } from "../rainbow/desktopTiles";
import {
  FORECAST_CENTER_LAT,
  FORECAST_CENTER_LON,
  FORECAST_HALF_EXTENT_LAT,
  FORECAST_HALF_EXTENT_LON,
} from "../forecast/useForecastProvider";

// S10 follow-up: register the `rainbow-tile://` custom MapLibre protocol
// once per app lifetime, only inside the desktop shell -- this is the one
// MapLibre-aware module (see this file's own "map/render adapter boundary"
// doc comment below), so it owns registering a MapLibre-level concern like
// a custom protocol, not `useRainbowOverlay`/`App.tsx`. A no-op in a plain
// browser tab; `registerDesktopRainbowProtocol` is itself also idempotent,
// so this is safe even though module-level code only really runs once
// anyway. See `../rainbow/desktopTiles.ts`'s doc comment for why the
// desktop shell needs a custom protocol at all (api.rainbow.ai's total
// lack of CORS headers).
if (isDesktop()) registerDesktopRainbowProtocol();

/**
 * Draw `rings` (radar-web's `rangeRingsGeoJson` output -- plain `[lon,
 * lat]` coordinate geometry, no MapLibre knowledge otherwise) into a 2D
 * `<canvas>` positioned above the radar sweep canvas, projecting each
 * point through the live `map` instance (`map.project`).
 *
 * A native MapLibre `line` layer would paint into the map's own canvas,
 * underneath the radar sweep `<canvas>`, so rings would sit under echoes.
 * This overlay canvas one z-index above draws them on top, the way radar
 * workstations do; it stays the one MapLibre-aware piece (`radar-web`
 * only hands back plain coordinate geometry).
 */
function drawRangeRings(map: MapLibreMap, canvas: HTMLCanvasElement, rings: number[][][] | null) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);
  if (!rings || rings.length === 0) return;

  ctx.strokeStyle = "#a3a89f";
  ctx.lineWidth = 1;
  ctx.globalAlpha = 0.5;
  for (const ring of rings) {
    if (ring.length === 0) continue;
    ctx.beginPath();
    ring.forEach(([lon, lat], i) => {
      const p = map.project([lon, lat]);
      if (i === 0) ctx.moveTo(p.x, p.y);
      else ctx.lineTo(p.x, p.y);
    });
    ctx.closePath();
    ctx.stroke();
  }
}

/**
 * Paint the model field (rendered plate carrée over the fixed forecast
 * extent) onto the map. Longitude is linear in both projections, latitude
 * isn't, so each source row is drawn to its own Mercator band.
 */
function drawForecastField(map: MapLibreMap, canvas: HTMLCanvasElement, field: HTMLCanvasElement | null) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (!field || !canvas.clientWidth) return;
  const scale = canvas.width / canvas.clientWidth;
  ctx.setTransform(scale, 0, 0, scale, 0, 0);
  ctx.imageSmoothingEnabled = false;
  const west = FORECAST_CENTER_LON - FORECAST_HALF_EXTENT_LON;
  const east = FORECAST_CENTER_LON + FORECAST_HALF_EXTENT_LON;
  const north = FORECAST_CENTER_LAT + FORECAST_HALF_EXTENT_LAT;
  const rowDeg = (2 * FORECAST_HALF_EXTENT_LAT) / field.height;
  for (let row = 0; row < field.height; row++) {
    const top = map.project([west, north - row * rowDeg]);
    const bottom = map.project([east, north - (row + 1) * rowDeg]);
    ctx.drawImage(field, 0, row, field.width, 1, top.x, top.y, bottom.x - top.x, bottom.y - top.y + 0.5);
  }
}

export interface CourseLeg {
  a: { lat: number; lon: number };
  b: { lat: number; lon: number } | null;
}

/** Course ink: the one colour reserved for marks the user places on the radar. */
const COURSE_INK = "#b56cff";
const COURSE_CASING = "#0e1110";

/**
 * Draw the user's cross-section leg the way an orienteering course is
 * printed: start triangle pointing down the leg, the leg line, a finish
 * double circle. Drawn on the rings canvas (after the rings, never
 * clearing it), with a dark casing so the ink separates from the echoes
 * beneath. `progress` (0-1) draws the leg partway for its entry motion.
 */
function drawCourse(map: MapLibreMap, canvas: HTMLCanvasElement, course: CourseLeg | null, progress = 1) {
  const ctx = canvas.getContext("2d");
  if (!ctx || !course) return;
  const a = map.project([course.a.lon, course.a.lat]);
  const b = course.b ? map.project([course.b.lon, course.b.lat]) : null;
  const angle = b ? Math.atan2(b.y - a.y, b.x - a.x) : -Math.PI / 2;
  const triangle = [0, (2 * Math.PI) / 3, (4 * Math.PI) / 3].map((t) => ({
    x: a.x + 13 * Math.cos(angle + t),
    y: a.y + 13 * Math.sin(angle + t),
  }));
  const end = b && { x: a.x + (b.x - a.x) * progress, y: a.y + (b.y - a.y) * progress };

  ctx.save();
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  for (const [color, width] of [
    [COURSE_CASING, 7],
    [COURSE_INK, 3],
  ] as const) {
    ctx.strokeStyle = color;
    ctx.lineWidth = width;
    ctx.beginPath();
    triangle.forEach((p, i) => (i === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
    ctx.closePath();
    ctx.stroke();
    if (b && end) {
      // The leg runs between the symbols, not through them.
      const len = Math.hypot(end.x - a.x, end.y - a.y);
      const [ux, uy] = [Math.cos(angle), Math.sin(angle)];
      if (len > 26) {
        ctx.beginPath();
        ctx.moveTo(a.x + ux * 14, a.y + uy * 14);
        ctx.lineTo(end.x - (progress === 1 ? ux * 12 : 0), end.y - (progress === 1 ? uy * 12 : 0));
        ctx.stroke();
      }
      if (progress === 1) {
        for (const r of [11, 6.5]) {
          ctx.beginPath();
          ctx.arc(b.x, b.y, r, 0, 2 * Math.PI);
          ctx.stroke();
        }
      }
    }
  }
  ctx.restore();
}

/**
 * Draw NWS alert polygons/multipolygons (S06) into a dedicated overlay
 * canvas, styled by severity (`SEVERITY_COLORS`), the selected alert (if
 * any) outlined more heavily.
 *
 * # Also needed the overlay-canvas treatment, same as range rings
 *
 * A native MapLibre `geojson` source + `fill`/`line` layer was tried
 * first here too. It does render -- but for the same structural reason
 * documented above `drawRangeRings`: it paints into the *map's own*
 * canvas, a plain sibling `<div>`'s internal canvas with no explicit
 * `zIndex`, while the radar sweep `<canvas>` is a separate, absolutely
 * positioned DOM element at `zIndex: 1` layered on top of that whole map
 * div. Nothing drawn by a native MapLibre layer can ever appear above
 * that sweep canvas, regardless of MapLibre's own layer order.
 * Confirmed during this task's own browser verification: an alert
 * polygon whose on-screen position happened to fall under the radar
 * canvas's ~460km-radius box around the selected site rendered zero
 * visible pixels there (while the same polygon's portion outside that box
 * rendered fine) -- exactly the range-rings bug, and for exactly the same
 * reason, since alert polygons are frequently centered on or near the
 * currently-selected radar site (that is the whole point of overlaying
 * them on a radar workstation). This canvas sits one `zIndex` above the
 * radar canvas (matching `drawRangeRings`'s fix) so alert shapes are never
 * occluded regardless of where they fall relative to the sweep image.
 */
function drawAlerts(
  map: MapLibreMap,
  canvas: HTMLCanvasElement,
  alerts: AlertFeatureCollection | null,
  selectedKey: string | null,
) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);
  if (!alerts) return;

  function strokeRing(ring: [number, number][]) {
    if (ring.length === 0) return;
    ctx!.beginPath();
    ring.forEach(([lon, lat], i) => {
      const p = map.project([lon, lat]);
      if (i === 0) ctx!.moveTo(p.x, p.y);
      else ctx!.lineTo(p.x, p.y);
    });
    ctx!.closePath();
  }

  for (const feature of alerts.features) {
    if (!feature.geometry) continue; // no drawn shape -- normal, list/detail-only.
    const color = SEVERITY_COLORS[feature.properties.severity];
    const selected = feature.id === selectedKey;
    ctx.fillStyle = color;
    ctx.strokeStyle = color;
    ctx.lineWidth = selected ? 3 : 1.5;
    ctx.globalAlpha = 0.22;
    ctx.beginPath();
    const polygons =
      feature.geometry.type === "Polygon" ? [feature.geometry.coordinates] : feature.geometry.coordinates;
    for (const polygon of polygons) {
      for (const ring of polygon) {
        strokeRing(ring as [number, number][]);
      }
    }
    ctx.fill("evenodd");
    ctx.globalAlpha = selected ? 0.95 : 0.75;
    for (const polygon of polygons) {
      for (const ring of polygon) {
        strokeRing(ring as [number, number][]);
        ctx.stroke();
      }
    }
  }
  ctx.globalAlpha = 1;
}

/**
 * The user's own position, on the alerts canvas so it sits above warning
 * polygons: a faint accuracy circle, a Paper Ink dot on the course casing,
 * and an arrow ahead of it along the heading. Neutral ink on purpose: the
 * user didn't place it (not purple) and it isn't a selection (not steel blue).
 */
function drawMyLocation(map: MapLibreMap, canvas: HTMLCanvasElement, loc: MyLocation | null) {
  const ctx = canvas.getContext("2d");
  if (!ctx || !loc) return;
  const p = map.project([loc.lon, loc.lat]);
  const accuracyPx = radiusPx(map, loc, loc.accuracyM / 1000);
  ctx.save();
  if (accuracyPx > 12) {
    ctx.fillStyle = "rgba(236, 235, 228, 0.12)";
    ctx.strokeStyle = "rgba(236, 235, 228, 0.5)";
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(p.x, p.y, accuracyPx, 0, 2 * Math.PI);
    ctx.fill();
    ctx.stroke();
  }
  ctx.fillStyle = "#ecebe4";
  ctx.strokeStyle = COURSE_CASING;
  ctx.lineWidth = 3;
  if (loc.headingDeg !== null) {
    // Screen angle: heading is from north, the map may be rotated.
    const a = ((loc.headingDeg - map.getBearing() - 90) * Math.PI) / 180;
    const at = (r: number, da: number) => [p.x + r * Math.cos(a + da), p.y + r * Math.sin(a + da)] as const;
    ctx.beginPath();
    ctx.moveTo(...at(20, 0));
    ctx.lineTo(...at(11, 0.5));
    ctx.lineTo(...at(11, -0.5));
    ctx.closePath();
    ctx.stroke();
    ctx.fill();
  }
  ctx.beginPath();
  ctx.arc(p.x, p.y, 7, 0, 2 * Math.PI);
  ctx.stroke();
  ctx.fill();
  ctx.restore();
}

const RAINBOW_SOURCE_ID = "rainbow-precip-source";
const RAINBOW_LAYER_ID = "rainbow-precip-layer";

/**
 * Add/update/remove the Rainbow precip raster source+layer to match
 * `tileUrlTemplate` (`null` removes it). MapLibre raster sources can't have
 * their `tiles` array updated in place, so a changed template (a newly
 * resolved snapshot) is handled the same way as turning the layer off: tear
 * down and re-add.
 *
 * Layer order: inserted before the style's first `symbol` layer (if any) so
 * basemap labels stay legible on top of the precip imagery, matching how
 * most slippy-map weather overlays are conventionally stacked; falls back
 * to "on top of everything" if the style has no symbol layer.
 *
 * # Why a native MapLibre layer is *not* the occlusion problem here
 *
 * `drawRangeRings`/`drawAlerts` above had to move off native MapLibre
 * layers because those paint into the map's own canvas, which sits
 * *underneath* the radar sweep `<canvas>` (see their doc comments).
 * A Rainbow raster layer added the normal MapLibre way would land in that
 * same underneath-the-sweep-canvas layer -- so with the live radar visible,
 * it would render but never be seen, exactly like an unfixed range ring.
 * Rather than adding a *third* manual-canvas workaround for what is, unlike
 * rings/alerts, a full-viewport raster image (no benefit to hand-drawing it
 * pixel-by-pixel -- MapLibre's native raster source is exactly the tool for
 * this, per this stage's brief), this is handled at the call site
 * (`MapView`'s render below) by hiding the radar sweep canvas whenever the
 * Rainbow toggle is on. See that comment for the full mutual-exclusivity
 * rationale.
 *
 * `maxZoom` (S09d Part A) caps the raster source at the currently selected
 * layer's documented zoom ceiling (`precip`/`precip-global` 0-12,
 * `clouds`/`radars` 0-7 -- KB §4.4/§4.5): MapLibre's own `maxzoom` on a
 * raster source stops requesting new tiles past that zoom and instead
 * upscales the last fetched one, which is the standard, correct way to
 * respect a tile source's documented ceiling rather than requesting tiles
 * the API would reject. Whenever the layer selection itself changes,
 * `tileUrlTemplate`'s own path segment changes too (it embeds the layer
 * name), so the identity check below already forces a rebuild with the new
 * `maxZoom` in the same pass -- no separate dedupe needed for this alone.
 */
function syncRainbowLayer(
  map: MapLibreMap,
  tileUrlTemplate: string | null,
  maxZoom: number,
  appliedRef: { current: string | null },
) {
  if (appliedRef.current === tileUrlTemplate) return; // already in the desired state.

  if (map.getLayer(RAINBOW_LAYER_ID)) map.removeLayer(RAINBOW_LAYER_ID);
  if (map.getSource(RAINBOW_SOURCE_ID)) map.removeSource(RAINBOW_SOURCE_ID);
  appliedRef.current = null;

  if (!tileUrlTemplate) return;

  map.addSource(RAINBOW_SOURCE_ID, {
    type: "raster",
    tiles: [tileUrlTemplate],
    tileSize: 256,
    maxzoom: maxZoom,
  });
  const firstSymbolLayerId = map.getStyle()?.layers?.find((l) => l.type === "symbol")?.id;
  map.addLayer(
    {
      id: RAINBOW_LAYER_ID,
      type: "raster",
      source: RAINBOW_SOURCE_ID,
      paint: { "raster-opacity": 0.75 },
    },
    firstSymbolLayerId,
  );
  appliedRef.current = tileUrlTemplate;
}

/**
 * OpenFreeMap's "Dark" style (https://openfreemap.org) -- a full
 * OpenStreetMap-derived vector basemap (streets, cities/labels, land
 * use, etc.), served from OpenFreeMap's own infrastructure.
 *
 * Replaces the earlier MapLibre demo-tiles placeholder
 * (`https://demotiles.maplibre.org/style.json`, a deliberately
 * low-detail example style with no usage guarantees, unsuitable for
 * real use per `Agent Context/reference/DATA_SOURCES.md`: "Do not rely
 * on community public tile infrastructure for a high-traffic production
 * app; use a suitable provider or self-hosted tiles"). OpenFreeMap is
 * purpose-built for exactly that concern -- unlimited, no API key, no
 * rate limit, explicitly positioned as production-usable -- verified
 * reachable (HTTP 200, real style JSON) before wiring it in here rather
 * than assumed. It is not a formally SLA-backed commercial provider
 * (MapTiler/Stadia Maps/AWS Location Service remain the options if that
 * becomes a requirement, e.g. contractual uptime guarantees), and
 * OpenStreetMap-derived data requires attribution -- MapLibre renders
 * OpenFreeMap's/OSM's built-in attribution control by default; do not
 * remove it.
 */
const MAP_STYLE_URL = "https://tiles.openfreemap.org/styles/dark";

/** Click radius (px) for picking a radar-site dot. */
const SITE_HIT_RADIUS_PX = 10;

/** Nearest radar site to screen point `px` within `SITE_HIT_RADIUS_PX`. */
function siteAtPoint(map: MapLibreMap, px: { x: number; y: number }): RadarSite | null {
  let best: RadarSite | null = null;
  let bestD2 = SITE_HIT_RADIUS_PX * SITE_HIT_RADIUS_PX;
  for (const site of RADAR_SITES) {
    const p = map.project([site.lon, site.lat]);
    const d2 = (p.x - px.x) ** 2 + (p.y - px.y) ** 2;
    if (d2 <= bestD2) {
      best = site;
      bestD2 = d2;
    }
  }
  return best;
}

const SITES_GEOJSON = {
  type: "FeatureCollection" as const,
  features: RADAR_SITES.map((s) => ({
    type: "Feature" as const,
    properties: { icao: s.icao },
    geometry: { type: "Point" as const, coordinates: [s.lon, s.lat] },
  })),
};

/** Fallback sweep radius (km, WSR-88D's typical reflectivity range) until the
 * renderer reports its real range (`sweepRangeKm`, from radar-web's `renderedRangeKm`). */
const APPROX_SWEEP_RADIUS_KM = 460;

/** Screen-pixel radius of `rangeKm` around `site`, measured with the map's
 * own projection (east-west, so it is exact for the canvas's width). A
 * hand-rolled meters-per-pixel formula here used 256 px tiles while
 * MapLibre's zoom is 512 px tiles, drawing the radar at half size. */
function radiusPx(map: MapLibreMap, site: { lat: number; lon: number }, rangeKm: number): number {
  const dLon = rangeKm / (111.32 * Math.cos((site.lat * Math.PI) / 180));
  return map.project([site.lon + dLon, site.lat]).x - map.project([site.lon, site.lat]).x;
}

/**
 * S09 Phase 3: the live map's current viewport, expressed as the plain
 * center/half-extent camera `useMrmsOverlay`/`renderCurrentGrid` take (see
 * `MrmsViewport`'s own doc comment for why this crosses the mapping-adapter
 * boundary as plain numbers, not a MapLibre type). Derived straight from
 * `map.getBounds()` -- the same "regular flat-plane" approximation
 * `forecast_grid.wgsl` already documents (not a real map projection), which
 * is an acceptable, documented approximation for a raster overlay at this
 * scale (mirrors `APPROX_SWEEP_RADIUS_KM`'s own "reasonable approximation"
 * precedent above).
 */
function viewportFromMapBounds(map: MapLibreMap): MrmsViewport {
  const bounds = map.getBounds();
  const west = bounds.getWest();
  const east = bounds.getEast();
  const south = bounds.getSouth();
  const north = bounds.getNorth();
  return {
    centerLon: (west + east) / 2,
    centerLat: (south + north) / 2,
    halfExtentLon: (east - west) / 2,
    halfExtentLat: (north - south) / 2,
  };
}

export interface MapViewProps {
  site: RadarSite;
  canvasRef: RefObject<HTMLCanvasElement>;
  canvasSize: number;
  /** Called with a site's ICAO when its map dot is clicked. */
  onSiteClick?: (icao: string) => void;
  /** Real site-to-edge range (km) of the rendered sweep; `null` falls back
   * to `APPROX_SWEEP_RADIUS_KM`. */
  sweepRangeKm?: number | null;
  /** Range-ring geometry from `RadarWebRenderer`'s `rangeRingsGeoJson` free
   * function (plain `[lon, lat]` coordinates, one ring per radius) --
   * `null` until available. */
  rangeRings?: number[][][] | null;
  /** Fired on every map `mousemove`, with the cursor resolved to a map
   * lat/lon via MapLibre's own `unproject` (exposed as `event.lngLat`) --
   * the geographic half of S05's data-probe/cursor-readout feature. */
  onCursorMove?: (lat: number, lon: number) => void;
  onCursorLeave?: () => void;
  /** S06: currently-active NWS alerts (`useAlertPoller`'s `geojson`),
   * drawn as a severity-colored overlay -- `null` until the first poll
   * resolves. */
  alerts?: AlertFeatureCollection | null;
  /** Stable `AlertKey` of the alert currently shown in the details panel,
   * if any -- drawn with a heavier outline. */
  selectedAlertKey?: string | null;
  /** Fired when the user clicks an alert polygon (or clicks empty map
   * space while an alert is selected, with `key: null`, to deselect). */
  onAlertClick?: (key: string | null) => void;
  /** S09d follow-up ("click the map to set the point"): whether a Rainbow
   * Nowcast/Forecast point-pick is currently armed (`App.tsx`'s
   * `rainbowPointPickTarget !== null`). While true, the map cursor reads as
   * a crosshair instead of the default/alert-hover cursor, and the next
   * click is routed to `onPointPick` instead of alert click/select --
   * picking a point isn't trying to open an alert. */
  pointPickActive?: boolean;
  /** Fired with the clicked point's `{lat, lon}` when `pointPickActive` is
   * true and the map is clicked -- one-shot; the caller is responsible for
   * disarming `pointPickActive` afterward (this component never decides
   * *which* panel a pick was for, only that one happened). */
  onPointPick?: (lat: number, lon: number) => void;
  /** S09b: resolved Rainbow precip tile URL template (`{z}/{x}/{y}` still
   * literal, `{snapshot}`/`{forecast_time}` already substituted by
   * `useRainbowOverlay`), or `null` whenever the layer should not be on the
   * map (off, unconfigured, still resolving, or errored). */
  rainbowTileUrlTemplate?: string | null;
  /** S09b: the user's Rainbow toggle *intent* (`useRainbowOverlay`'s
   * `enabled`) -- distinct from `rainbowTileUrlTemplate` being non-null,
   * and what actually drives hiding the live radar canvas (see the render
   * comment below for why). */
  rainbowEnabled?: boolean;
  /** S09d Part A: the currently selected Rainbow layer's documented zoom
   * ceiling (`useRainbowOverlay`'s `layerInfo.maxZoom` -- 12 for
   * precip/precip-global, 7 for clouds/radars, KB §4.4/§4.5), applied to
   * the MapLibre raster source's own `maxzoom` in `syncRainbowLayer`.
   * Defaults to 12 (this app's default layer, `precip`'s, own ceiling) so
   * a caller that hasn't wired this up yet still gets a sane cap rather
   * than an unbounded one. */
  rainbowMaxZoom?: number;
  /** S09 Phase 3: canvas `useMrmsOverlay` paints its rendered MRMS frame
   * onto -- owned by the caller (`App.tsx`), same "hook owns the render
   * pipeline, `MapView` owns the DOM element's position/size" split
   * `canvasRef` (radar) already establishes. `undefined` until the caller
   * has one to give (never expected in practice -- `App.tsx` always
   * creates it via `useRef` up front). */
  mrmsCanvasRef?: RefObject<HTMLCanvasElement>;
  /** The user's MRMS toggle *intent* -- what actually drives hiding the
   * live radar canvas (see the render comment below), same shape as
   * `rainbowEnabled`. */
  mrmsEnabled?: boolean;
  /** UI polish pass: user-controlled opacity (`0`..`1`) for the live radar
   * sweep canvas, applied only in the "radar is the active layer" case --
   * see the render comment below for why this is combined with, not a
   * replacement for, the `rainbowEnabled`/`mrmsEnabled` mutual-exclusivity
   * gate. Lifted to `App.tsx` (`SidebarSection title="Radar"`'s opacity
   * slider) and threaded down the same way `rainbowEnabled`/`elevationDeg`-
   * style controls already are. Defaults to `1` (fully opaque) so behavior
   * is unchanged for a caller that hasn't wired this up. */
  radarOpacity?: number;
  /** The user's own position (dot + heading arrow), or null when off. */
  myLocation?: MyLocation | null;
  /** Bumped to pan the map to `myLocation` (the "Center on me" button). */
  centerOnMeRequest?: number;
  /** The cross-section leg, drawn on the map in course ink. */
  course?: CourseLeg | null;
  /** The painted model-field canvas, drawn on the map (in place of the radar) while `forecastOnMap`. */
  forecastCanvasRef?: RefObject<HTMLCanvasElement>;
  forecastOnMap?: boolean;
  /** Changes whenever the model field is repainted, so the map copy redraws. */
  forecastKey?: string;
  /** Fired on every map `move`/`zoom`/`resize`/`load`, with the map's
   * current viewport already resolved to the plain center/half-extent
   * camera `useMrmsOverlay` consumes (see `viewportFromMapBounds`) --
   * `useMrmsOverlay` itself debounces/generation-guards what it does with
   * this; `MapView` just reports it on every relevant map event, same as
   * `onCursorMove` reports every raw `mousemove`. */
  onMrmsViewportChange?: (viewport: MrmsViewport) => void;
}

/**
 * Full-viewport MapLibre map with the radar `<canvas>` (owned by the
 * caller -- see `useRadarRenderer`) overlaid on top, kept positioned/scaled
 * to approximate the selected site's sweep footprint as the map pans,
 * zooms, or resizes.
 *
 * # Map/render adapter boundary
 *
 * Per GLOBAL_CONTRACT ("Mapping is behind an adapter. Radar rendering must
 * not depend directly on React or MapLibre internals" / "Renderer receives
 * viewport/camera/projection inputs through an adapter"): this component
 * is that adapter. It knows about MapLibre; `radar-web`'s renderer (driven
 * by `useRadarRenderer`, used from `App.tsx`) does not -- it only ever
 * sees a plain `<canvas>` element and byte arrays. This component computes
 * a CSS position/size box from the map's viewport and applies it to the
 * canvas element via plain DOM styles; it never reaches into the renderer
 * or re-triggers a GPU render on pan/zoom (the rendered sweep image itself
 * does not change when the map moves, only where it is drawn on screen).
 */
export function MapView({
  site,
  canvasRef,
  canvasSize,
  sweepRangeKm = null,
  onSiteClick,
  rangeRings = null,
  onCursorMove,
  onCursorLeave,
  alerts = null,
  selectedAlertKey = null,
  onAlertClick,
  pointPickActive = false,
  onPointPick,
  rainbowTileUrlTemplate = null,
  rainbowEnabled = false,
  rainbowMaxZoom = 12,
  mrmsCanvasRef,
  mrmsEnabled = false,
  radarOpacity = 1,
  onMrmsViewportChange,
  course = null,
  myLocation = null,
  centerOnMeRequest = 0,
  forecastCanvasRef,
  forecastOnMap = false,
  forecastKey = "",
}: MapViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<MapLibreMap | null>(null);
  const ringsCanvasRef = useRef<HTMLCanvasElement>(null);
  const alertsCanvasRef = useRef<HTMLCanvasElement>(null);
  const selectedIcaoRef = useRef(site.icao);
  selectedIcaoRef.current = site.icao;
  const onSiteClickRef = useRef(onSiteClick);
  onSiteClickRef.current = onSiteClick;
  const sweepRangeKmRef = useRef(sweepRangeKm);
  sweepRangeKmRef.current = sweepRangeKm;
  const rangeRingsRef = useRef<number[][][] | null>(rangeRings);
  rangeRingsRef.current = rangeRings;
  const courseRef = useRef<CourseLeg | null>(course);
  courseRef.current = course;
  const forecastOverlayRef = useRef<HTMLCanvasElement>(null);
  const forecastFieldRef = useRef<HTMLCanvasElement | null>(null);
  forecastFieldRef.current = forecastOnMap ? (forecastCanvasRef?.current ?? null) : null;
  const alertsRef = useRef<AlertFeatureCollection | null>(alerts);
  alertsRef.current = alerts;
  const selectedAlertKeyRef = useRef<string | null>(selectedAlertKey);
  selectedAlertKeyRef.current = selectedAlertKey;
  const myLocationRef = useRef<MyLocation | null>(myLocation);
  myLocationRef.current = myLocation;
  const appliedRainbowUrlRef = useRef<string | null>(null);
  const onMrmsViewportChangeRef = useRef(onMrmsViewportChange);
  onMrmsViewportChangeRef.current = onMrmsViewportChange;
  // S09d follow-up: read inside the click/hover handlers below via refs
  // (rather than as effect dependencies) so a new `onPointPick` identity or
  // a `pointPickActive` flip doesn't tear down and re-attach the map click
  // listener -- same idiom `onMrmsViewportChangeRef` above already uses.
  const pointPickActiveRef = useRef(pointPickActive);
  pointPickActiveRef.current = pointPickActive;
  const onPointPickRef = useRef(onPointPick);
  onPointPickRef.current = onPointPick;

  // Create the map once.
  useEffect(() => {
    if (!containerRef.current) return;
    const map = new MapLibreMap({
      container: containerRef.current,
      style: MAP_STYLE_URL,
      center: [site.lon, site.lat],
      zoom: 6,
    });
    mapRef.current = map;
    // Radar-site dots (click to switch site -- see `siteAtPoint`). Sits under
    // the radar canvas, whose no-data pixels are transparent.
    map.on("load", () => {
      map.addSource("radar-sites", { type: "geojson", data: SITES_GEOJSON });
      map.addLayer({
        id: "radar-sites",
        type: "circle",
        source: "radar-sites",
        paint: {
          "circle-radius": ["case", ["==", ["get", "icao"], selectedIcaoRef.current], 6, 4],
          "circle-color": ["case", ["==", ["get", "icao"], selectedIcaoRef.current], COURSE_INK, "#a3a89f"],
          "circle-stroke-color": COURSE_CASING,
          "circle-stroke-width": 2,
        },
      });
    });
    return () => {
      map.remove();
      mapRef.current = null;
    };
    // Intentionally created once (empty deps -- `site`'s initial value is
    // only used as the map's starting center); site changes are handled by
    // the effect below via `easeTo`, not by recreating the map instance.
  }, []);

  // Highlight the selected site's dot.
  useEffect(() => {
    const map = mapRef.current;
    if (!map?.getLayer("radar-sites")) return;
    const isSelected: ExpressionSpecification = ["==", ["get", "icao"], site.icao];
    map.setPaintProperty("radar-sites", "circle-radius", ["case", isSelected, 6, 4]);
    map.setPaintProperty("radar-sites", "circle-color", ["case", isSelected, COURSE_INK, "#a3a89f"]);
  }, [site.icao]);

  // S09b: keep the Rainbow raster layer in sync with the resolved tile URL
  // template (add/remove/replace via `syncRainbowLayer`). Needs the style
  // loaded before touching sources/layers, so waits for `load` if the map
  // isn't there yet (the toggle can flip -- and this effect can run --
  // before the initial style finishes loading).
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    function apply() {
      syncRainbowLayer(map!, rainbowTileUrlTemplate, rainbowMaxZoom, appliedRainbowUrlRef);
    }
    if (map.isStyleLoaded()) {
      apply();
    } else {
      map.once("load", apply);
      return () => {
        map.off("load", apply);
      };
    }
  }, [rainbowTileUrlTemplate, rainbowMaxZoom]);

  // Recenter on the selected site, and keep the radar canvas's on-screen
  // position/size synchronized to the map viewport (pan/zoom/resize).
  useEffect(() => {
    const map = mapRef.current;
    const canvas = canvasRef.current;
    const ringsCanvas = ringsCanvasRef.current;
    const alertsCanvas = alertsCanvasRef.current;
    const mrmsCanvas = mrmsCanvasRef?.current ?? null;
    if (!map || !canvas) return;

    function updateOverlay() {
      if (!map || !canvas) return;
      const centerPx = map.project([site.lon, site.lat]);
      const sizePx = 2 * radiusPx(map, site, sweepRangeKmRef.current ?? APPROX_SWEEP_RADIUS_KM);
      canvas.style.width = `${sizePx}px`;
      canvas.style.height = `${sizePx}px`;
      canvas.style.left = `${centerPx.x - sizePx / 2}px`;
      canvas.style.top = `${centerPx.y - sizePx / 2}px`;

      // Keep the range-rings overlay canvas's backing buffer matching the
      // map's current on-screen size (it covers the full map, unlike the
      // fixed-size radar sweep canvas above), then redraw -- ring
      // positions depend on the map's current projection/zoom.
      if (ringsCanvas) {
        const mapCanvas = map.getCanvas();
        if (ringsCanvas.width !== mapCanvas.width || ringsCanvas.height !== mapCanvas.height) {
          ringsCanvas.width = mapCanvas.width;
          ringsCanvas.height = mapCanvas.height;
          ringsCanvas.style.width = mapCanvas.style.width;
          ringsCanvas.style.height = mapCanvas.style.height;
        }
        drawRangeRings(map, ringsCanvas, rangeRingsRef.current);
        drawCourse(map, ringsCanvas, courseRef.current);
      }

      // Same backing-buffer sync for the alert-polygon overlay -- see
      // `drawAlerts`'s doc comment for why this is a manual canvas, not a
      // native MapLibre layer.
      if (alertsCanvas) {
        const mapCanvas = map.getCanvas();
        if (alertsCanvas.width !== mapCanvas.width || alertsCanvas.height !== mapCanvas.height) {
          alertsCanvas.width = mapCanvas.width;
          alertsCanvas.height = mapCanvas.height;
          alertsCanvas.style.width = mapCanvas.style.width;
          alertsCanvas.style.height = mapCanvas.style.height;
        }
        drawAlerts(map, alertsCanvas, alertsRef.current, selectedAlertKeyRef.current);
        drawMyLocation(map, alertsCanvas, myLocationRef.current);
      }

      const forecastOverlay = forecastOverlayRef.current;
      if (forecastOverlay) {
        const mapCanvas = map.getCanvas();
        if (forecastOverlay.width !== mapCanvas.width || forecastOverlay.height !== mapCanvas.height) {
          forecastOverlay.width = mapCanvas.width;
          forecastOverlay.height = mapCanvas.height;
        }
        drawForecastField(map, forecastOverlay, forecastFieldRef.current);
      }

      // S09 Phase 3: same backing-buffer sync as rings/alerts above -- the
      // MRMS overlay is a full-viewport raster, not a fixed-size circle
      // like the radar sweep canvas, so it needs to cover exactly the same
      // pixel area the map itself does (see `useMrmsOverlay`'s doc comment
      // for why its render call is framed to match this exactly).
      if (mrmsCanvas) {
        const mapCanvas = map.getCanvas();
        if (mrmsCanvas.width !== mapCanvas.width || mrmsCanvas.height !== mapCanvas.height) {
          mrmsCanvas.width = mapCanvas.width;
          mrmsCanvas.height = mapCanvas.height;
          mrmsCanvas.style.width = mapCanvas.style.width;
          mrmsCanvas.style.height = mapCanvas.style.height;
        }
      }

      // Report the current viewport as a plain center/half-extent camera --
      // `useMrmsOverlay` (not this component) decides whether/when that's
      // worth actually re-rendering for (debounced, generation-guarded).
      onMrmsViewportChangeRef.current?.(viewportFromMapBounds(map));
    }

    map.easeTo({ center: [site.lon, site.lat], duration: 600 });

    // MapLibre fires `move` continuously during pan/zoom/fly animations,
    // `zoom` specifically on zoom changes, and `resize` when the map
    // container's size changes -- listening to all three (rather than just
    // `move`, which does cover zoom too) matches this stage's brief
    // explicitly and costs nothing extra since `updateOverlay` is cheap
    // and idempotent.
    map.on("move", updateOverlay);
    map.on("zoom", updateOverlay);
    map.on("resize", updateOverlay);
    map.on("load", updateOverlay);
    updateOverlay();

    return () => {
      map.off("move", updateOverlay);
      map.off("zoom", updateOverlay);
      map.off("resize", updateOverlay);
      map.off("load", updateOverlay);
    };
  }, [site, canvasRef, mrmsCanvasRef, sweepRangeKm]);

  // Redraw the range rings whenever the geometry itself changes (new site,
  // or the first time it becomes available after wasm load) -- projecting
  // through `map.project()` needs no "style loaded" wait the way adding a
  // GeoJSON source would, since it never touches the map's own style/layers.
  useEffect(() => {
    const map = mapRef.current;
    const ringsCanvas = ringsCanvasRef.current;
    if (!map || !ringsCanvas) return;
    drawRangeRings(map, ringsCanvas, rangeRings);
    drawCourse(map, ringsCanvas, courseRef.current);
  }, [rangeRings]);

  // Redraw the course leg when it changes; a newly completed leg draws
  // itself from start to finish over 200 ms (the one authored motion).
  useEffect(() => {
    const map = mapRef.current;
    const ringsCanvas = ringsCanvasRef.current;
    if (!map || !ringsCanvas) return;
    const animate = course?.b && !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const started = performance.now();
    let frame = 0;
    const draw = () => {
      const t = animate ? Math.min(1, (performance.now() - started) / 200) : 1;
      drawRangeRings(map, ringsCanvas, rangeRingsRef.current);
      drawCourse(map, ringsCanvas, course, 1 - (1 - t) ** 3);
      if (t < 1) frame = requestAnimationFrame(draw);
    };
    draw();
    return () => cancelAnimationFrame(frame);
  }, [course]);

  useEffect(() => {
    const map = mapRef.current;
    const overlay = forecastOverlayRef.current;
    if (map && overlay) drawForecastField(map, overlay, forecastFieldRef.current);
  }, [forecastOnMap, forecastKey]);

  // Redraw alert polygons and the user's position whenever the active set,
  // selection or position changes (map pan/zoom/resize is handled by
  // `updateOverlay` above).
  useEffect(() => {
    const map = mapRef.current;
    const alertsCanvas = alertsCanvasRef.current;
    if (!map || !alertsCanvas) return;
    drawAlerts(map, alertsCanvas, alerts, selectedAlertKey);
    drawMyLocation(map, alertsCanvas, myLocation);
  }, [alerts, selectedAlertKey, myLocation]);

  // Pan to the user's position on request; reads the ref so later fixes don't re-pan.
  useEffect(() => {
    const loc = myLocationRef.current;
    if (centerOnMeRequest && loc) mapRef.current?.easeTo({ center: [loc.lon, loc.lat], duration: 600 });
  }, [centerOnMeRequest]);

  // Alert click handling: resolve a map click to "which alert (if any) was
  // clicked" via plain lon/lat point-in-polygon hit testing against the
  // currently-active set (see `geometryHitTest.ts` -- native MapLibre
  // feature-picking only works for layers actually registered with the
  // map, and alert polygons are deliberately drawn to a manual canvas
  // instead; see `drawAlerts`'s doc comment). Also sets a pointer cursor on
  // hover for discoverability, and clicking empty map space deselects.
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;

    function alertAtPoint(lngLat: { lat: number; lng: number }): string | null {
      const current = alertsRef.current;
      if (!current) return null;
      const point: [number, number] = [lngLat.lng, lngLat.lat];
      let best: { key: string; severityRank: number } | null = null;
      for (const feature of current.features) {
        if (!feature.geometry) continue;
        if (!pointInAlertGeometry(point, feature.geometry)) continue;
        const rank = severityRank(feature.properties.severity);
        // Multiple overlapping alert polygons at one point are possible
        // (e.g. a Flood Watch under a Severe Thunderstorm Warning) --
        // surface the most severe one first, a reasonable UX choice, not
        // a lifecycle decision.
        if (!best || rank < best.severityRank) {
          best = { key: feature.id, severityRank: rank };
        }
      }
      return best?.key ?? null;
    }

    function handleClick(e: { lngLat: { lat: number; lng: number }; point: { x: number; y: number } }) {
      // S09d follow-up: a point-pick in progress consumes this click
      // entirely -- a user picking a Rainbow point isn't trying to open an
      // alert, so alert click/select must not also fire for it.
      if (pointPickActiveRef.current) {
        onPointPickRef.current?.(e.lngLat.lat, e.lngLat.lng);
        return;
      }
      // Alerts win over nearby site dots: switching sites wipes scan history.
      const alertKey = alertAtPoint(e.lngLat);
      const clickedSite = !alertKey && onSiteClickRef.current ? siteAtPoint(map!, e.point) : null;
      if (clickedSite) {
        onSiteClickRef.current?.(clickedSite.icao);
        return;
      }
      onAlertClick?.(alertKey);
    }
    function handleHoverMove(e: { lngLat: { lat: number; lng: number }; point: { x: number; y: number } }) {
      if (pointPickActiveRef.current) {
        map!.getCanvas().style.cursor = "crosshair";
        return;
      }
      const hit = alertAtPoint(e.lngLat) || (onSiteClickRef.current && siteAtPoint(map!, e.point));
      map!.getCanvas().style.cursor = hit ? "pointer" : "";
    }

    map.on("click", handleClick);
    map.on("mousemove", handleHoverMove);
    return () => {
      map.off("click", handleClick);
      map.off("mousemove", handleHoverMove);
      map.getCanvas().style.cursor = "";
    };
  }, [onAlertClick]);

  // S09d follow-up: set the crosshair cursor immediately when pick mode
  // arms/disarms, rather than waiting for the next `mousemove` (which
  // `handleHoverMove` above keeps in sync with while the mouse does move) --
  // otherwise the cursor would only change once the user starts moving the
  // mouse after clicking "Pick on map".
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    map.getCanvas().style.cursor = pointPickActive ? "crosshair" : "";
  }, [pointPickActive]);

  // Geographic cursor readout: MapLibre's own `mousemove`/`mouseout`
  // events already carry the cursor resolved to a map lat/lon via
  // `unproject` internally (`event.lngLat`) -- no manual projection math
  // needed here.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !onCursorMove) return;
    function handleMove(e: { lngLat: { lat: number; lng: number } }) {
      onCursorMove?.(e.lngLat.lat, e.lngLat.lng);
    }
    function handleLeave() {
      onCursorLeave?.();
    }
    map.on("mousemove", handleMove);
    map.on("mouseout", handleLeave);
    return () => {
      map.off("mousemove", handleMove);
      map.off("mouseout", handleLeave);
    };
  }, [onCursorMove, onCursorLeave]);

  return (
    // Two siblings, not parent/child: MapLibre takes full imperative
    // ownership of its container's DOM contents (it appends/removes its own
    // canvas, controls, and markers directly, outside of React). Nesting
    // our React-rendered radar `<canvas>` inside that same container risks
    // MapLibre reordering or displacing it -- confirmed during this stage's
    // own verification: with the canvas nested inside the map container,
    // the map's own canvas painted over it and the radar overlay was never
    // visible despite rendering correctly to its backing buffer. Keeping
    // them as siblings under a common positioned parent, with the radar
    // canvas layered on top via `zIndex`, avoids MapLibre ever touching it.
    <div style={{ position: "absolute", inset: 0 }}>
      <div ref={containerRef} style={{ position: "absolute", inset: 0 }} />
      <canvas
        ref={canvasRef}
        width={canvasSize}
        height={canvasSize}
        style={{
          position: "absolute",
          zIndex: 1,
          // Real position/size set imperatively by `updateOverlay` above;
          // these are just sane pre-layout defaults before the first map
          // event fires.
          top: 0,
          left: 0,
          pointerEvents: "none",
          // S09b/S09 Phase 3 mutual-exclusivity decision: hidden whenever
          // the user has the Rainbow toggle *or* the MRMS toggle on. This
          // canvas is opaque and sits above MapLibre's own canvas (see this
          // component's other doc comments on why range rings/alerts had to
          // move to overlay canvases for exactly that reason) -- a Rainbow
          // raster layer, added the normal native-MapLibre way per that
          // stage's brief, paints *underneath* it and would be completely
          // invisible with the live sweep still showing; the MRMS overlay
          // canvas below sits *above* it for the same reason (both are
          // full-frame raster images occupying this exact "opaque raster
          // over the map" slot). Rather than trying to show more than one
          // of {live radar, Rainbow nowcast, MRMS national mosaic}
          // simultaneously -- three full-coverage rasters stacked would
          // just occlude each other with no way to see through, and mixing
          // a live single-site sweep with a national mosaic of the same
          // physical quantity (reflectivity) invites exactly the kind of
          // observation-vs-observation confusion GLOBAL_CONTRACT's
          // distinguishability rule is about -- all three are made mutually
          // exclusive: only one is ever visible at a time, enforced at the
          // toggle level in `App.tsx` (turning one on turns the other two
          // off, not just visually here) so a checkbox's checked state
          // never lies about what's on screen. Driven by `rainbowEnabled`/
          // `mrmsEnabled` (the toggles' intent) rather than by whether
          // their data has actually resolved yet, so a switch feels
          // immediate rather than lagging a network/render round trip.
          //
          // UI polish pass: when the radar sweep *is* the active layer (the
          // `0` branch above unaffected -- Rainbow/MRMS still force full
          // invisibility), `radarOpacity` lets the user dial it down instead
          // of always fully opaque, e.g. to see map labels/city names
          // through it. Purely a display preference layered on top of the
          // same mutual-exclusivity gate, never a way to defeat it.
          opacity: rainbowEnabled || mrmsEnabled || forecastOnMap ? 0 : radarOpacity,
        }}
      />
      {/* S09 Phase 3: MRMS national-mosaic overlay -- a full-viewport raster
          `useMrmsOverlay` paints into via `putImageData`, positioned/sized
          exactly like the rings/alerts overlay canvases below (backing
          buffer synced to the map's own canvas dimensions in
          `updateOverlay`) rather than the radar canvas's fixed circular
          framing, since MRMS covers the whole current viewport reactively,
          not a fixed site-centered footprint. Sits at the same z-index
          "slot" as the radar canvas (see that canvas's own mutual-
          exclusivity comment above) -- opacity-gated by `mrmsEnabled` the
          same way the radar canvas is gated by "not Rainbow/MRMS". Renders
          only when a `mrmsCanvasRef` was actually supplied (always true in
          practice; optional only so this component doesn't hard-require a
          feature `App.tsx` might not wire up in some future embedding). */}
      {mrmsCanvasRef && (
        <canvas
          ref={mrmsCanvasRef}
          style={{
            position: "absolute",
            zIndex: 1,
            top: 0,
            left: 0,
            width: "100%",
            height: "100%",
            pointerEvents: "none",
            opacity: mrmsEnabled ? 1 : 0,
          }}
        />
      )}
      {/* Range rings, one z-index above the radar sweep canvas --
          see `drawRangeRings`'s doc comment for why a native MapLibre
          layer (painted into the map's own canvas, underneath the sweep
          canvas) would draw under the echoes. */}
      {/* Forecast radar past NOW: same slot as the radar, which it replaces while shown. */}
      <canvas
        ref={forecastOverlayRef}
        style={{
          position: "absolute",
          zIndex: 1,
          top: 0,
          left: 0,
          width: "100%",
          height: "100%",
          pointerEvents: "none",
        }}
      />
      <canvas
        ref={ringsCanvasRef}
        style={{
          position: "absolute",
          zIndex: 2,
          top: 0,
          left: 0,
          width: "100%",
          height: "100%",
          pointerEvents: "none",
        }}
      />
      {/* S06 alert polygons, one z-index above the range rings -- same
          "manual overlay canvas above the radar canvas" fix,
          `pointerEvents: none` since clicks are resolved via the `map`
          instance's own `click` event + lon/lat hit testing above, not via
          this canvas element directly (letting mouse events pass through
          to MapLibre's normal pan/zoom handling underneath). */}
      <canvas
        ref={alertsCanvasRef}
        style={{
          position: "absolute",
          zIndex: 3,
          top: 0,
          left: 0,
          width: "100%",
          height: "100%",
          pointerEvents: "none",
        }}
      />
    </div>
  );
}
