import type { ProbeResult } from "../radar/useRadarRenderer";

export interface CursorLatLon {
  lat: number;
  lon: number;
}

export interface ProbePanelProps {
  cursor: CursorLatLon | null;
  probe: ProbeResult | null;
}

/**
 * Geographic cursor readout (lat/lon, and azimuth/range from the site once
 * `probeGate` resolves them) plus the data-probe value itself, rendered so
 * "valid" / "missing" / "range-folded" are visibly distinct states (not
 * just differently-worded text) -- matching the renderer's own never-
 * collapse guarantee (`GateProbeResult`, `GLOBAL_CONTRACT.md`).
 */
export function ProbePanel({ cursor, probe }: ProbePanelProps) {
  return (
    <div className="probe-panel">
      <div className="probe-panel-title">Cursor</div>
      {cursor ? (
        <div className="probe-latlon">
          {cursor.lat.toFixed(4)}°, {cursor.lon.toFixed(4)}°
        </div>
      ) : (
        <div className="probe-latlon probe-empty">(move over the map)</div>
      )}

      {probe ? (
        <>
          <div className="probe-polar">
            az {probe.azimuthDeg.toFixed(1)}° · range {probe.slantRangeKm.toFixed(1)} km
          </div>
          <div className={`probe-value probe-value-${probe.state}`}>
            {probe.state === "valid" && (
              <>
                {probe.value?.toFixed(2)} {probe.units}
              </>
            )}
            {probe.state === "missing" && <>missing / below threshold</>}
            {probe.state === "range_folded" && <>range-folded (ambiguous)</>}
          </div>
        </>
      ) : (
        <div className="probe-value probe-value-none">(off sweep coverage)</div>
      )}
    </div>
  );
}
