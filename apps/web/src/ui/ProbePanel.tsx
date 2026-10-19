import { Icon } from "./Icon";
import type { ProbeResult } from "../radar/useRadarRenderer";

export interface CursorLatLon {
  lat: number;
  lon: number;
}

export interface ProbePanelProps {
  cursor: CursorLatLon | null;
  probe: ProbeResult | null;
}

/** The gate's state in words; shared by this panel and the map's cursor label. */
export function probeText(probe: ProbeResult | null): string {
  if (!probe) return "off sweep coverage";
  if (probe.state === "valid") return `${probe.value?.toFixed(2)} ${probe.units ?? ""}`;
  if (probe.state === "missing") return "missing / below threshold";
  return "range-folded (ambiguous)";
}

/**
 * The cursor's gate as a course row (control symbol, value, slant range,
 * azimuth from the site) plus its lat/lon, rendered so
 * "valid" / "missing" / "range-folded" are visibly distinct states (not
 * just differently-worded text) -- matching the renderer's own never-
 * collapse guarantee (`GateProbeResult`, `GLOBAL_CONTRACT.md`).
 */
export function ProbePanel({ cursor, probe }: ProbePanelProps) {
  if (!cursor) return <div className="probe-empty">Hover the map to read a gate.</div>;
  return (
    <div className="probe-panel">
      <table className="course-table">
        <tbody>
          <tr>
            <td className="course-symbol">
              <Icon name="control" size={16} />
            </td>
            <td className={`probe-value probe-value-${probe?.state ?? "none"}`}>
              {probeText(probe)}
            </td>
            <td className="num">{probe ? `${probe.slantRangeKm.toFixed(1)} km` : ""}</td>
            <td className="num">{probe ? `${probe.azimuthDeg.toFixed(0).padStart(3, "0")}°` : ""}</td>
          </tr>
        </tbody>
      </table>
      <div className="probe-latlon">
        {cursor.lat.toFixed(4)}°, {cursor.lon.toFixed(4)}°
      </div>
    </div>
  );
}
