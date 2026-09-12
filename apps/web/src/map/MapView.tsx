import { useEffect, useRef, type RefObject } from "react";
// MapLibre GL JS >=5 has no default export; `MapLibreMap` is the library's
// own non-colliding alias for its `Map` class (distinct from the global
// JS `Map`).
import { MapLibreMap } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import type { RadarSite } from "../sites";

/**
 * MapLibre's free public demo style/tiles
 * (https://demotiles.maplibre.org/style.json).
 *
 * ---------------------------------------------------------------------
 * UNRESOLVED PRODUCTION CONCERN -- placeholder, not a silently-accepted
 * gap. Per `Agent Context/reference/DATA_SOURCES.md`: "Do not rely on
 * community public tile infrastructure for a high-traffic production app;
 * use a suitable provider or self-hosted tiles." This demo style is a
 * low-detail basemap with no usage guarantees, meant for MapLibre's own
 * examples -- it is used here only because this stage's job is proving the
 * radar-over-map integration, not sourcing production tiles. A real
 * deployment needs a proper tile provider (e.g. MapTiler, Stadia Maps,
 * AWS Location Service) or self-hosted vector tiles before this ships.
 * ---------------------------------------------------------------------
 */
const PLACEHOLDER_MAP_STYLE_URL = "https://demotiles.maplibre.org/style.json";

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
export function MapView({ site, canvasRef, canvasSize }: MapViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const mapRef = useRef<MapLibreMap | null>(null);

  // Create the map once.
  useEffect(() => {
    if (!containerRef.current) return;
    const map = new MapLibreMap({
      container: containerRef.current,
      style: PLACEHOLDER_MAP_STYLE_URL,
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

  // Recenter on the selected site, and keep the radar canvas's on-screen
  // position/size synchronized to the map viewport (pan/zoom/resize).
  useEffect(() => {
    const map = mapRef.current;
    const canvas = canvasRef.current;
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
        }}
      />
    </div>
  );
}
