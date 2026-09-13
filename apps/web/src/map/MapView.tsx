import { useEffect, useRef, type RefObject } from "react";
// MapLibre GL JS >=5 has no default export; `MapLibreMap` is the library's
// own non-colliding alias for its `Map` class (distinct from the global
// JS `Map`).
import { MapLibreMap } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import type { RadarSite } from "../sites";
import type { AlertFeatureCollection } from "../alerts/types";
import { SEVERITY_COLORS, severityRank } from "../alerts/types";
import { pointInAlertGeometry } from "../alerts/geometryHitTest";

/**
 * Draw `rings` (radar-web's `rangeRingsGeoJson` output -- plain `[lon,
 * lat]` coordinate geometry, no MapLibre knowledge otherwise) into a 2D
 * `<canvas>` positioned above the radar sweep canvas, projecting each
 * point through the live `map` instance (`map.project`).
 *
 * A native MapLibre GeoJSON source + `line` layer was tried first (the
 * literal reading of this stage's brief) and does work as a map layer --
 * but it paints into the *map's own* canvas, which sits *underneath* the
 * radar sweep `<canvas>` (see the layout comment below on why those two
 * canvases must be separate DOM siblings). The radar canvas is opaque
 * (S04's black sweep background, `radar-render`'s own clear color -- not
 * something this task touches) and covers the exact on-screen area a
 * site-centered ring would need to appear in, so a native map-layer ring
 * is invisible in practice: confirmed during this task's own browser
 * verification (a ring added as a GeoJSON layer rendered with zero visible
 * pixels, entirely occluded by the radar canvas drawn on top of it).
 *
 * This dedicated overlay canvas -- one z-index above the radar canvas --
 * is the fix: still driven entirely by `map.project()` (this remains the
 * one MapLibre-aware component; `radar-web` still only ever hands back
 * plain coordinate geometry), just painted to a surface that can actually
 * sit above the opaque sweep image, the same way real radar workstations
 * draw range rings over the reflectivity image rather than under it.
 */
function drawRangeRings(map: MapLibreMap, canvas: HTMLCanvasElement, rings: number[][][] | null) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);
  if (!rings || rings.length === 0) return;

  ctx.strokeStyle = "#7aa6c2";
  ctx.lineWidth = 1;
  ctx.globalAlpha = 0.65;
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
 * that opaque sweep canvas, regardless of MapLibre's own layer order.
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
 * *underneath* the opaque radar sweep `<canvas>` (see their doc comments).
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
 */
function syncRainbowLayer(map: MapLibreMap, tileUrlTemplate: string | null, appliedRef: { current: string | null }) {
  if (appliedRef.current === tileUrlTemplate) return; // already in the desired state.

  if (map.getLayer(RAINBOW_LAYER_ID)) map.removeLayer(RAINBOW_LAYER_ID);
  if (map.getSource(RAINBOW_SOURCE_ID)) map.removeSource(RAINBOW_SOURCE_ID);
  appliedRef.current = null;

  if (!tileUrlTemplate) return;

  map.addSource(RAINBOW_SOURCE_ID, {
    type: "raster",
    tiles: [tileUrlTemplate],
    tileSize: 256,
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
 * OpenFreeMap's "Liberty" style (https://openfreemap.org) -- a full
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
const MAP_STYLE_URL = "https://tiles.openfreemap.org/styles/liberty";

/**
 * Approximate radius (km) framed by `radar-web`'s rendered sweep image
 * around the site, used only to size/position the canvas overlay on top of
 * the map. This is a fixed, documented approximation, not the sweep's real
 * per-scan maximum range: `SweepInfo` (see `radar-web/src/browser.rs`)
 * does not currently expose the exact `max_range_km` value the renderer's
 * camera actually framed (`browser.rs` computes it internally from the
 * decoded sweep's gate geometry but never returns it), and per this
 * stage's brief, exact geographic alignment is explicitly out of scope
 * ("You do not need pixel-perfect geographic projection of the radar
 * sweep onto the map for this stage... a reasonable, documented
 * approximation... is acceptable"). 460 km is a round, conservative
 * approximation of WSR-88D's typical base-reflectivity unambiguous range.
 * A follow-up stage doing real geographic reprojection should either
 * expose the renderer's actual computed range through `SweepInfo`, or
 * (better) drive the map overlay from `radar-geo`'s real polar-to-map
 * projection instead of this flat circular approximation.
 */
const APPROX_SWEEP_RADIUS_KM = 460;

/** Standard Web-Mercator meters-per-pixel formula at a 256px tile size --
 * MapLibre's (like Mapbox GL's) public `getZoom()` API is calibrated to
 * this classic tile pyramid even though it renders 512px tiles internally,
 * so this formula applies directly to `map.getZoom()`'s return value. This
 * is itself an approximation (Mercator distortion means "meters per pixel"
 * varies with latitude, which the `cos(lat)` term accounts for, but not
 * with longitude-direction distortion near the poles -- irrelevant for
 * CONUS/territory WSR-88D sites). */
function metersPerPixel(latitudeDeg: number, zoom: number): number {
  return (156543.03392804097 * Math.cos((latitudeDeg * Math.PI) / 180)) / Math.pow(2, zoom);
}

export interface MapViewProps {
  site: RadarSite;
  canvasRef: RefObject<HTMLCanvasElement>;
  canvasSize: number;
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
  rangeRings = null,
  onCursorMove,
  onCursorLeave,
  alerts = null,
  selectedAlertKey = null,
  onAlertClick,
  rainbowTileUrlTemplate = null,
  rainbowEnabled = false,
}: MapViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<MapLibreMap | null>(null);
  const ringsCanvasRef = useRef<HTMLCanvasElement>(null);
  const alertsCanvasRef = useRef<HTMLCanvasElement>(null);
  const rangeRingsRef = useRef<number[][][] | null>(rangeRings);
  rangeRingsRef.current = rangeRings;
  const alertsRef = useRef<AlertFeatureCollection | null>(alerts);
  alertsRef.current = alerts;
  const selectedAlertKeyRef = useRef<string | null>(selectedAlertKey);
  selectedAlertKeyRef.current = selectedAlertKey;
  const appliedRainbowUrlRef = useRef<string | null>(null);

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
    return () => {
      map.remove();
      mapRef.current = null;
    };
    // Intentionally created once (empty deps -- `site`'s initial value is
    // only used as the map's starting center); site changes are handled by
    // the effect below via `easeTo`, not by recreating the map instance.
  }, []);

  // S09b: keep the Rainbow raster layer in sync with the resolved tile URL
  // template (add/remove/replace via `syncRainbowLayer`). Needs the style
  // loaded before touching sources/layers, so waits for `load` if the map
  // isn't there yet (the toggle can flip -- and this effect can run --
  // before the initial style finishes loading).
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    function apply() {
      syncRainbowLayer(map!, rainbowTileUrlTemplate, appliedRainbowUrlRef);
    }
    if (map.isStyleLoaded()) {
      apply();
    } else {
      map.once("load", apply);
      return () => {
        map.off("load", apply);
      };
    }
  }, [rainbowTileUrlTemplate]);

  // Recenter on the selected site, and keep the radar canvas's on-screen
  // position/size synchronized to the map viewport (pan/zoom/resize).
  useEffect(() => {
    const map = mapRef.current;
    const canvas = canvasRef.current;
    const ringsCanvas = ringsCanvasRef.current;
    const alertsCanvas = alertsCanvasRef.current;
    if (!map || !canvas) return;

    function updateOverlay() {
      if (!map || !canvas) return;
      const centerPx = map.project([site.lon, site.lat]);
      const zoom = map.getZoom();
      const mpp = metersPerPixel(site.lat, zoom);
      const sizePx = (2 * APPROX_SWEEP_RADIUS_KM * 1000) / mpp;
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
      }
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
  }, [site, canvasRef]);

  // Redraw the range rings whenever the geometry itself changes (new site,
  // or the first time it becomes available after wasm load) -- projecting
  // through `map.project()` needs no "style loaded" wait the way adding a
  // GeoJSON source would, since it never touches the map's own style/layers.
  useEffect(() => {
    const map = mapRef.current;
    const ringsCanvas = ringsCanvasRef.current;
    if (!map || !ringsCanvas) return;
    drawRangeRings(map, ringsCanvas, rangeRings);
  }, [rangeRings]);

  // Redraw alert polygons whenever the active set or selection changes
  // (map pan/zoom/resize is handled by `updateOverlay` above).
  useEffect(() => {
    const map = mapRef.current;
    const alertsCanvas = alertsCanvasRef.current;
    if (!map || !alertsCanvas) return;
    drawAlerts(map, alertsCanvas, alerts, selectedAlertKey);
  }, [alerts, selectedAlertKey]);

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

    function handleClick(e: { lngLat: { lat: number; lng: number } }) {
      onAlertClick?.(alertAtPoint(e.lngLat));
    }
    function handleHoverMove(e: { lngLat: { lat: number; lng: number } }) {
      map!.getCanvas().style.cursor = alertAtPoint(e.lngLat) ? "pointer" : "";
    }

    map.on("click", handleClick);
    map.on("mousemove", handleHoverMove);
    return () => {
      map.off("click", handleClick);
      map.off("mousemove", handleHoverMove);
      map.getCanvas().style.cursor = "";
    };
  }, [onAlertClick]);

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
          // S09b mutual-exclusivity decision: hidden whenever the user has
          // the Rainbow toggle on. This canvas is opaque and sits above
          // MapLibre's own canvas (see this component's other doc comments
          // on why range rings/alerts had to move to overlay canvases for
          // exactly that reason) -- a Rainbow raster layer, added the
          // normal native-MapLibre way per this stage's brief, paints
          // *underneath* it and would be completely invisible with the
          // live sweep still showing. Rather than adding a third bespoke
          // overlay-canvas workaround for what's just a full-frame raster
          // image (no benefit over MapLibre's own raster source for that),
          // the two are made mutually exclusive: turning Rainbow on hides
          // the radar canvas, turning it off restores it. Driven by
          // `rainbowEnabled` (the toggle's intent) rather than by whether a
          // tile URL has actually resolved yet, so the switch feels
          // immediate rather than lagging a network round trip.
          opacity: rainbowEnabled ? 0 : 1,
        }}
      />
      {/* Range rings, one z-index above the (opaque) radar sweep canvas --
          see `drawRangeRings`'s doc comment for why a native MapLibre
          layer (painted into the map's own canvas, underneath the sweep
          canvas) would be invisible here. */}
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
          "manual overlay canvas above the opaque radar canvas" fix,
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
