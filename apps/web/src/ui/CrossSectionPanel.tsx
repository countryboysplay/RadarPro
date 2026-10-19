import { useEffect, useRef, useState } from "react";
import { Icon } from "./Icon";

export interface LatLonPoint {
  lat: number;
  lon: number;
}

const COLS = 480;
const ROWS = 200;
const TOP_KM = 18;

export interface CrossSectionPanelProps {
  line: { a: LatLonPoint; b: LatLonPoint | null } | null;
  picking: boolean;
  onStartPick: () => void;
  onClear: () => void;
  /** Re-render trigger: changes whenever the volume/moment/palette does. */
  renderKey: string;
  render: (a: LatLonPoint, b: LatLonPoint, cols: number, rows: number, topKm: number) => Uint8Array | null;
}

/** Great-circle length (km) and initial bearing (deg true) of a course leg. */
export function legLengthAndBearing(a: LatLonPoint, b: LatLonPoint): { km: number; deg: number } {
  const rad = Math.PI / 180;
  const [p1, p2, dl] = [a.lat * rad, b.lat * rad, (b.lon - a.lon) * rad];
  const h = Math.sin((p2 - p1) / 2) ** 2 + Math.cos(p1) * Math.cos(p2) * Math.sin(dl / 2) ** 2;
  const km = 2 * 6371.0088 * Math.asin(Math.sqrt(h));
  const y = Math.sin(dl) * Math.cos(p2);
  const x = Math.cos(p1) * Math.sin(p2) - Math.sin(p1) * Math.cos(p2) * Math.cos(dl);
  return { km, deg: ((Math.atan2(y, x) / rad) + 360) % 360 };
}

/** Vertical slice through the current radar volume along two map-picked
 * points, 0-18 km MSL. Gaps between tilts are left empty, never filled. */
export function CrossSectionPanel({ line, picking, onStartPick, onClear, renderKey, render }: CrossSectionPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const complete = line?.b ? { a: line.a, b: line.b } : null;

  // Ref, not a dep: callers pass a fresh closure every render, and the slice
  // should only recompute when the line or `renderKey` changes.
  const renderRef = useRef(render);
  renderRef.current = render;
  const [sliced, setSliced] = useState(true);

  useEffect(() => {
    const ctx = canvasRef.current?.getContext("2d");
    if (!ctx || !line?.b) return;
    ctx.clearRect(0, 0, COLS, ROWS);
    const bytes = renderRef.current(line.a, line.b, COLS, ROWS, TOP_KM);
    if (bytes) ctx.putImageData(new ImageData(new Uint8ClampedArray(bytes), COLS, ROWS), 0, 0);
    setSliced(bytes !== null);
  }, [line, renderKey]);

  const hint = picking ? (line ? "Click the finish on the map" : "Click the start on the map") : null;
  const leg = complete ? legLengthAndBearing(complete.a, complete.b) : null;

  return (
    <div className="cross-section-panel">
      <div className="course-actions">
        <button type="button" onClick={onStartPick} disabled={picking}>
          {line ? "Set a new leg" : "Set leg on map"}
        </button>
        {line && (
          <button type="button" className="button-quiet" onClick={onClear}>
            Clear
          </button>
        )}
      </div>
      {hint && <div className="cross-section-hint">{hint}</div>}
      {leg && (
        <table className="course-table">
          <tbody>
            <tr>
              <td className="course-symbol">
                <Icon name="start" size={16} />
                <Icon name="finish" size={16} />
              </td>
              <td>Cross-section leg</td>
              <td className="num">{leg.km.toFixed(1)} km</td>
              <td className="num">{leg.deg.toFixed(0).padStart(3, "0")}°</td>
            </tr>
          </tbody>
        </table>
      )}
      {complete && (
        <>
          <canvas
            ref={canvasRef}
            width={COLS}
            height={ROWS}
            className="cross-section-canvas"
            aria-label="Radar vertical cross-section"
          />
          {!sliced && <div className="segment-empty">No decoded scan to slice yet.</div>}
          <div className="cross-section-axis">0–{TOP_KM} km MSL · start at left, finish at right · gaps between tilts left empty</div>
        </>
      )}
    </div>
  );
}
