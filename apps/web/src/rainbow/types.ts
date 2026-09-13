// S09d Part A: plain-data description of the Rainbow Tiles API's layer/
// palette surface (KB §4, §5, §8 of `rainbow_weather_api_knowledge_base.md`,
// scraped doc.rainbow.ai v0.35.2, 2026-09-13). No logic of its own beyond
// small lookups -- `useRainbowOverlay` decides what to *do* with a
// selection; this module only names what selections exist and which
// per-layer rules apply, so the UI (`RainbowToggle`) and the tile-URL
// builders (`snapshot.ts`, `desktopTiles.ts`) share one source of truth
// instead of three copies of "clouds has no forecast_time" scattered around.

/** The four documented Tiles API layers (KB §4, §8's `TilesLayer` enum).
 * String literals double as this app's own wire identifier for the layer --
 * `apps/desktop/src-tauri/src/rainbow.rs`'s `RainbowLayer::path_segment`
 * uses the identical strings by convention, not a shared type across the
 * JS/Rust boundary (see that module's doc comment on why). */
export type RainbowTileLayer = "precip" | "precip-global" | "clouds" | "radars";

/** Per-layer rules this app enforces before ever building a request --
 * mirrors `RainbowLayer`'s methods in `rainbow.rs` one-for-one so a
 * mismatch between the UI's assumptions and the native validation would be
 * an obvious diff, not a silent drift. */
export interface RainbowTileLayerInfo {
  id: RainbowTileLayer;
  /** Sidebar dropdown label. */
  label: string;
  /** KB §4.4/§4.5 (FAQ §10): precip/precip-global go to 12, clouds/radars
   * stop at 7. */
  maxZoom: number;
  /** KB §4.2/§4.3 vs §4.4/§4.5: only precip/precip-global have a
   * forecast_time dimension; clouds/radars are observation-only. */
  supportsForecastTime: boolean;
  /** KB §4.2/§4.3/§4.5: `color`/`coverage` apply to precip, precip-global,
   * radars -- undocumented (and omitted) for clouds. */
  supportsColorCoverage: boolean;
  /** KB §4.5 only: `use_precip_type` is a radars-only query param. */
  supportsUsePrecipType: boolean;
}

export const RAINBOW_TILE_LAYERS: readonly RainbowTileLayerInfo[] = [
  {
    id: "precip",
    label: "Precipitation",
    maxZoom: 12,
    supportsForecastTime: true,
    supportsColorCoverage: true,
    supportsUsePrecipType: false,
  },
  {
    id: "precip-global",
    label: "Precipitation (global)",
    maxZoom: 12,
    supportsForecastTime: true,
    supportsColorCoverage: true,
    supportsUsePrecipType: false,
  },
  {
    id: "clouds",
    label: "Clouds",
    maxZoom: 7,
    supportsForecastTime: false,
    supportsColorCoverage: false,
    supportsUsePrecipType: false,
  },
  {
    id: "radars",
    label: "Radars",
    maxZoom: 7,
    supportsForecastTime: false,
    supportsColorCoverage: true,
    supportsUsePrecipType: true,
  },
];

/** Default layer -- same one S09b originally shipped precip-only, kept as
 * this stage's default selection for continuity. */
export const DEFAULT_RAINBOW_TILE_LAYER: RainbowTileLayer = "precip";

/** Looks up a layer's rule set; throws on an unrecognized id rather than
 * silently falling back to a default (`RainbowTileLayer` is a closed union,
 * so reaching the `undefined` branch here means a real bug -- e.g. a stale
 * value read back from somewhere outside TypeScript's own type checking --
 * not a case to paper over). */
export function rainbowTileLayerInfo(id: RainbowTileLayer): RainbowTileLayerInfo {
  const found = RAINBOW_TILE_LAYERS.find((layer) => layer.id === id);
  if (!found) throw new Error(`unknown Rainbow tile layer "${id}"`);
  return found;
}

/** Tile color palette codes (KB §5) -- `dbz_u8` is the one non-numeric id
 * (raw reflectivity encoding, KB §5.1), everything else is a small integer
 * the API also accepts as a bare string. */
export interface RainbowPaletteOption {
  id: string;
  name: string;
}

export const RAINBOW_PALETTES: readonly RainbowPaletteOption[] = [
  { id: "0", name: "Rainbow" },
  { id: "1", name: "TWC" },
  { id: "2", name: "Dark Sky" },
  { id: "3", name: "Meteored" },
  { id: "4", name: "Nexrad" },
  { id: "5", name: "Rainviewer" },
  { id: "6", name: "Selex" },
  { id: "7", name: "Titan" },
  { id: "8", name: "RV Universal Blue" },
  { id: "9", name: "RV TWC" },
  { id: "dbz_u8", name: "Raw dBZ" },
];

/** Default palette -- `0` (Rainbow), the API's own documented default
 * (KB §4.2's `color` query param table). */
export const DEFAULT_RAINBOW_PALETTE = "0";

/** `forecast_time`'s documented domain (KB §4.2/§4.3): `[0, 14400]` seconds,
 * step `600` -- 25 labeled steps for the dropdown ("Now" .. "+4 h"), never a
 * free-form scrubber per this stage's explicit UI decision. */
export interface RainbowForecastTimeStep {
  value: number;
  label: string;
}

function forecastTimeLabel(seconds: number): string {
  if (seconds === 0) return "Now";
  const minutes = seconds / 60;
  if (minutes % 60 === 0) return `+${minutes / 60} h`;
  return `+${minutes} min`;
}

export const RAINBOW_FORECAST_TIME_MAX_SECONDS = 14_400;
export const RAINBOW_FORECAST_TIME_STEP_SECONDS = 600;

export const RAINBOW_FORECAST_TIME_STEPS: readonly RainbowForecastTimeStep[] = Array.from(
  { length: RAINBOW_FORECAST_TIME_MAX_SECONDS / RAINBOW_FORECAST_TIME_STEP_SECONDS + 1 },
  (_, i) => {
    const value = i * RAINBOW_FORECAST_TIME_STEP_SECONDS;
    return { value, label: forecastTimeLabel(value) };
  },
);
