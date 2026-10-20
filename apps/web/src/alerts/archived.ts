// Warnings, advisories and special weather statements in effect at a past
// time, from IEM's archive of NWS polygons (CORS-open), shaped like the live
// `weather-alerts` output so the list, map and detail panel show them as-is.

import { getJson } from "../spc/spc";
import type { AlertFeatureCollection, AlertGeometry, AlertSeverity, HeldAlert } from "./types";
import { severityRank } from "./types";

const IEM = "https://mesonet.agron.iastate.edu/geojson";

interface SbwProps {
  phenomena: string;
  significance: string;
  ps: string;
  wfo: string;
  eventid: number;
  year: number;
  product_id: string;
  issue: string;
  polygon_begin: string;
  polygon_end: string;
  tornadotag: string | null;
  hailtag: number | null;
  windtag: number | null;
  damagetag: string | null;
}

interface SpsProps {
  issue: string;
  expire: string;
  max_hail_size: string | null;
  max_wind_gust: string | null;
  wfo: string;
}

interface Feature<P> {
  id?: string;
  geometry: AlertGeometry | null;
  properties: P;
}

// IEM has no CAP severity; these are what NWS's CAP feed usually sends for each.
function severityOf(phenomena: string, significance: string): AlertSeverity {
  if (phenomena === "TO" && significance === "W") return "Extreme";
  if (significance === "W") return "Severe";
  if (significance === "Y") return "Minor";
  return "Unknown";
}

const time = (ms: number) => new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

function held(key: string, id: string, event: string, severity: AlertSeverity, wfo: string, issued: number, effective: number, expires: number, tags: string[], geometry: AlertGeometry | null): HeldAlert {
  const headline = [...tags, `until ${time(expires)}`].join(" · ");
  return {
    key,
    alert: {
      id,
      geometry,
      messageType: "Alert",
      event,
      severity,
      certainty: "Unknown",
      urgency: "Unknown",
      sender: `NWS ${wfo}`,
      senderId: wfo,
      issued,
      effective,
      expires,
      ends: null,
      headline,
      // Filled in from the product text when the alert is opened.
      description: null,
      instruction: null,
      affectedAreas: [],
    },
  };
}

export function archivedAlerts(sbw: Feature<SbwProps>[], sps: Feature<SpsProps>[]): HeldAlert[] {
  const alerts = [
    ...sbw.map(({ geometry, properties: p }) => {
      const tags = [
        p.tornadotag && `Tornado ${p.tornadotag.toLowerCase()}`,
        p.damagetag && `${p.damagetag.toLowerCase()} damage`,
        p.hailtag && `Hail ${p.hailtag} in`,
        p.windtag && `Wind ${p.windtag} mph`,
      ].filter((t): t is string => !!t);
      const key = `${p.wfo}.${p.phenomena}.${p.significance}.${p.eventid}.${p.year}`;
      return held(key, p.product_id, p.ps, severityOf(p.phenomena, p.significance), p.wfo, Date.parse(p.issue), Date.parse(p.polygon_begin), Date.parse(p.polygon_end), tags, geometry);
    }),
    ...sps.map(({ id, geometry, properties: p }) => {
      const tags = [p.max_hail_size && Number(p.max_hail_size) > 0 && `Hail ${p.max_hail_size} in`, p.max_wind_gust && `Wind ${p.max_wind_gust} mph`].filter((t): t is string => !!t);
      const issued = Date.parse(p.issue);
      return held(`SPS.${id}`, id ?? "", "Special Weather Statement", "Moderate", p.wfo, issued, issued, Date.parse(p.expire), tags, geometry);
    }),
  ];
  return alerts.sort((a, b) => severityRank(a.alert.severity) - severityRank(b.alert.severity));
}

/** The warnings and SPS in effect at `atMs`. */
export async function fetchArchivedAlerts(atMs: number, signal: AbortSignal): Promise<HeldAlert[]> {
  const iso = new Date(atMs).toISOString().slice(0, 16) + "Z";
  const [sbw, sps] = await Promise.all([
    getJson<{ features: Feature<SbwProps>[] }>(`${IEM}/sbw.geojson?ts=${iso}`, signal),
    getJson<{ features: Feature<SpsProps>[] }>(`${IEM}/sps.geojson?valid=${iso}`, signal),
  ]);
  return archivedAlerts(sbw.features, sps.features);
}

export function alertsGeoJson(alerts: HeldAlert[]): AlertFeatureCollection {
  return {
    type: "FeatureCollection",
    features: alerts.map(({ key, alert: { geometry, ...properties } }) => ({ type: "Feature", id: key, properties, geometry })),
  };
}
