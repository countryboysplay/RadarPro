// Plain TypeScript mirrors of `crates/mrms-web/src/wasm_api.rs`'s JSON wire
// shapes -- no logic of its own, matching this codebase's "no logic of its
// own in the browser glue" convention (see `apps/web/src/forecast/types.ts`/
// `apps/web/src/alerts/types.ts`): `mrms` decides everything about
// discovery, decoding, and units; this app only ever renders what
// `MrmsHandle` hands back.
//
// MRMS is an observation, never a forecast (Global Contract) -- these types
// deliberately carry no run/lead/ensemble field, only `validTime` (the one
// real-world instant a snapshot observes). See `crates/mrms-web/src/
// wasm_api.rs`'s module doc for the full "never borrow forecast-flavored
// naming" discipline this mirrors.

/** The two concrete products `mrms-web`'s `MrmsHandle` accepts. */
export type MrmsProductId = "reflectivity" | "precip_rate";

/** Mirrors `SnapshotMetadataJson` -- the snapshot `discoverLatestSnapshot`
 * resolved to (not yet fetched/decoded). */
export interface MrmsSnapshotMetadata {
  product: MrmsProductId;
  /** ISO-8601 UTC timestamp. */
  snapshotTime: string;
  /** Diagnostic only (which NOAA object this is) -- carries no meaning this
   * app's own logic depends on. */
  objectKey: string;
}

/** Mirrors `GridGeometryJson`. `lonMinDeg`/`lonMaxDeg`/`originLonDeg` are in
 * MRMS's native `[0, 360)` longitude convention (see `wasm_api.rs`'s own
 * doc comment) -- e.g. CONUS's western edge is ~230.005, not ~-129.995.
 * Convert with `lonNativeToSigned` before comparing against a MapLibre
 * viewport's conventional -180..180 longitudes. */
export interface MrmsGridGeometry {
  width: number;
  height: number;
  originLatDeg: number;
  originLonDeg: number;
  latStepDeg: number;
  lonStepDeg: number;
  latMinDeg: number;
  latMaxDeg: number;
  lonMinDeg: number;
  lonMaxDeg: number;
}

/** Mirrors `MrmsGridMetadataJson` -- resolved by `fetchSnapshot`. Deliberately
 * excludes the decoded value array itself (24.5 million cells; see that
 * method's own doc comment in `wasm_api.rs`) -- render it via
 * `renderCurrentGrid`. */
export interface MrmsGridMetadata {
  product: MrmsProductId;
  unit: string;
  /** ISO-8601 UTC timestamp -- the observed instant. MRMS's only
   * timestamp; never a run/lead pair (Global Contract). */
  validTime: string;
  geometry: MrmsGridGeometry;
}

/** Convert a longitude in MRMS's native `[0, 360)` convention to the
 * conventional signed `-180..180` range every other part of this app (map
 * viewport, `renderCurrentGrid`'s own `centerLon` input) uses. Values
 * already `<= 180` pass through unchanged. */
export function lonNativeToSigned(lonNativeDeg: number): number {
  return lonNativeDeg > 180 ? lonNativeDeg - 360 : lonNativeDeg;
}

/** This grid's geographic bounding box, converted to conventional signed
 * longitude -- the box `useMrmsOverlay` intersects the current map viewport
 * against. */
export interface MrmsSignedBounds {
  west: number;
  east: number;
  south: number;
  north: number;
}

export function signedBoundsFromGeometry(geometry: MrmsGridGeometry): MrmsSignedBounds {
  return {
    west: lonNativeToSigned(geometry.lonMinDeg),
    east: lonNativeToSigned(geometry.lonMaxDeg),
    south: geometry.latMinDeg,
    north: geometry.latMaxDeg,
  };
}

/** A camera derived from the live map's current viewport -- plain signed
 * decimal degrees, no MapLibre type here (Global Contract: "Mapping is
 * behind an adapter" -- `MapView` computes this from `map.getBounds()` and
 * hands it down; neither this module nor `useMrmsOverlay` ever touches a
 * MapLibre instance directly). Mirrors the shape `renderCurrentGrid` itself
 * takes (center + half-extent), just reactive instead of the fixed
 * constants `useForecastProvider` uses for its own dedicated panel -- see
 * this stage's brief for why MRMS needs the real thing where the forecast
 * panel didn't. */
export interface MrmsViewport {
  centerLon: number;
  centerLat: number;
  halfExtentLon: number;
  halfExtentLat: number;
}
