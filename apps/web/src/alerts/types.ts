// Plain TypeScript mirrors of `crates/weather-alerts/src/json.rs`'s output
// shapes -- this file has no logic of its own beyond severity display
// helpers, matching this codebase's "no logic of its own in the browser
// glue" convention: `weather-alerts` decides everything about lifecycle
// and normalization; this app only ever renders what it hands back.

/** CAP `severity`. `"Unknown"` is a valid, common value, not an error --
 * see `crates/weather-alerts/src/model.rs`. */
export type AlertSeverity = "Unknown" | "Minor" | "Moderate" | "Severe" | "Extreme";
export type AlertCertainty = "Unknown" | "Unlikely" | "Possible" | "Likely" | "Observed";
export type AlertUrgency = "Unknown" | "Past" | "Future" | "Expected" | "Immediate";
export type AlertMessageType = "Alert" | "Update" | "Cancel";
export type ExpiryReason = "TimeExpired" | "AbsentFromPolls";

export interface AlertPolygonGeometry {
  type: "Polygon";
  coordinates: [number, number][][];
}
export interface AlertMultiPolygonGeometry {
  type: "MultiPolygon";
  coordinates: [number, number][][][];
}
export type AlertGeometry = AlertPolygonGeometry | AlertMultiPolygonGeometry;

/** Mirrors `AlertPropertiesJson` (`crates/weather-alerts/src/json.rs`) --
 * every timestamp is epoch milliseconds (UTC), directly usable as
 * `new Date(ms)`. */
export interface AlertProperties {
  messageType: AlertMessageType;
  event: string;
  severity: AlertSeverity;
  certainty: AlertCertainty;
  urgency: AlertUrgency;
  sender: string;
  senderId: string;
  issued: number;
  effective: number;
  expires: number;
  ends: number | null;
  headline: string | null;
  description: string | null;
  instruction: string | null;
  affectedAreas: string[];
  /** NWS hazard tags from CAP `parameters`, `[key, value]` in display
   * order, e.g. `["maxHailSize", "1.75"]` (see `TAG_KEYS` in
   * `crates/weather-alerts/src/parse.rs`). */
  tags: [string, string][];
}

/** One GeoJSON feature from `AlertStoreHandle.activeAlertsGeoJson` --
 * `id` is the alert's stable `AlertKey` (not its own rotating CAP message
 * id), matching `crate::json::alerts_to_geojson`. */
export interface AlertFeature {
  type: "Feature";
  id: string;
  properties: AlertProperties;
  geometry: AlertGeometry | null;
}

export interface AlertFeatureCollection {
  type: "FeatureCollection";
  features: AlertFeature[];
}

/** Mirrors `AlertJson` (`crates/weather-alerts/src/json.rs`) -- a full
 * alert's fields (properties, flattened, plus its own current CAP message
 * `id` and geometry), used for change events and the details panel. */
export interface AlertJson extends AlertProperties {
  /** This alert's *current* CAP message id (rotates on every update) --
   * not the stable key; the stable key is carried separately as
   * `AlertChangeJson.key`. */
  id: string;
  geometry: AlertGeometry | null;
}

export type AlertChangeJson =
  | { type: "New"; key: string; alert: AlertJson }
  | { type: "Updated"; key: string; alert: AlertJson }
  | { type: "Cancelled"; key: string; alert: AlertJson }
  | { type: "Expired"; key: string; alert: AlertJson; reason: ExpiryReason };

/** A held alert as tracked by `useAlertPoller`'s reducer: the stable key
 * plus its latest known content. */
export interface HeldAlert {
  key: string;
  alert: AlertJson;
}

/** This project's own severity color choices for the alert overlay/list
 * (a documented design choice, not a claimed replication of any specific
 * commercial radar tool's palette -- see `GLOBAL_CONTRACT.md`: "must have
 * its own ... assets"). Chosen for: (1) intuitive escalation from cool/
 * muted (Minor/Unknown) to hot/saturated (Extreme), (2) enough contrast
 * against both the light OpenFreeMap basemap and the dark HUD panels this
 * app already uses, (3) distinguishability for the common forms of color
 * vision deficiency (protanopia/deuteranopia) -- red/orange/yellow/gray
 * are kept far apart in both hue *and* lightness, not relying on hue
 * alone. */
export const SEVERITY_COLORS: Record<AlertSeverity, string> = {
  Extreme: "#d32f2f", // saturated red
  Severe: "#f57c00", // orange
  Moderate: "#fbc02d", // yellow
  Minor: "#4a90d9", // blue
  Unknown: "#9e9e9e", // neutral gray
};

/** Fixed severity rank for sorting (most severe first) -- matches the
 * order `SEVERITY_COLORS` documents (Extreme > Severe > Moderate > Minor
 * > Unknown). */
const SEVERITY_RANK: Record<AlertSeverity, number> = {
  Extreme: 0,
  Severe: 1,
  Moderate: 2,
  Minor: 3,
  Unknown: 4,
};

export function severityRank(severity: AlertSeverity): number {
  return SEVERITY_RANK[severity];
}

export function formatAlertTime(epochMillis: number): string {
  return new Date(epochMillis).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
    hour12: true,
  });
}

const lower = (s: string) => s.toLowerCase();
const TAG_TEXT: Record<string, (v: string) => string | null> = {
  tornadoDetection: (v) => `Tornado ${lower(v)}`,
  waterspoutDetection: (v) => `Waterspout ${lower(v)}`,
  tornadoDamageThreat: (v) => `${v.charAt(0)}${lower(v.slice(1))} tornado damage`,
  thunderstormDamageThreat: (v) => `${v.charAt(0)}${lower(v.slice(1))} damage`,
  flashFloodDamageThreat: (v) => `${v.charAt(0)}${lower(v.slice(1))} flood damage`,
  maxHailSize: (v) => (Number(v) > 0 ? `Hail ${v} in` : null),
  maxWindGust: (v) => (parseFloat(v) > 0 ? `Wind ${lower(v)}` : null),
};

/** "Tornado radar indicated · Hail 1.75 in · Wind 60 mph", or "" with no tags. */
export function formatTags(tags: [string, string][]): string {
  return tags
    .map(([key, value]) => TAG_TEXT[key]?.(value))
    .filter(Boolean)
    .join(" · ");
}
