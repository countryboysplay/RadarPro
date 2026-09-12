// Plain lon/lat point-in-polygon hit testing for alert geometry, used by
// `MapView`'s click handling to resolve "which alert (if any) did the user
// click" without depending on MapLibre's own feature-query machinery (which
// only works for *native* GeoJSON layers -- see `MapView.tsx`'s doc
// comments on why alert polygons are drawn to a manual canvas overlay
// instead, the same reason range rings are).
//
// This is a UI hit-testing convenience, not a scientific/geodesic
// computation: it treats [lon, lat] as a flat Cartesian plane (the standard
// even-odd ray-casting algorithm), which is a fine approximation at the
// scale of a single CAP alert polygon (a county/state-sized area, never
// spanning the antimeridian in NWS's domestic feed) and is exactly what the
// polygon will look like once actually projected to screen space anyway.

import type { AlertGeometry } from "./types";

export type LonLat = readonly [number, number];

/** Even-odd ray-casting test: is `point` inside `ring` (a single closed
 * linear ring)? Standard algorithm (see e.g. the PNPOLY reference
 * implementation); works for both convex and concave rings. */
function pointInRing(point: LonLat, ring: readonly LonLat[]): boolean {
  const [x, y] = point;
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i];
    const [xj, yj] = ring[j];
    const intersects = yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi;
    if (intersects) inside = !inside;
  }
  return inside;
}

/** Is `point` inside a polygon-with-holes (`rings[0]` exterior,
 * `rings[1..]` holes)? Summing the even-odd result across *every* ring
 * (exterior and holes alike) is the standard way this rule naturally
 * handles holes: a point inside the exterior and inside exactly one hole
 * flips twice (net: outside), matching GeoJSON's own ring-winding-agnostic
 * definition. */
function pointInPolygonRings(point: LonLat, rings: readonly (readonly LonLat[])[]): boolean {
  let inside = false;
  for (const ring of rings) {
    if (pointInRing(point, ring)) inside = !inside;
  }
  return inside;
}

/** Is `point` inside `geometry` (a `Polygon` or `MultiPolygon`)? */
export function pointInAlertGeometry(point: LonLat, geometry: AlertGeometry): boolean {
  if (geometry.type === "Polygon") {
    return pointInPolygonRings(point, geometry.coordinates as LonLat[][]);
  }
  return geometry.coordinates.some((polygon) => pointInPolygonRings(point, polygon as LonLat[][]));
}
