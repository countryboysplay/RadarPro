import { useEffect, useRef } from "react";

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

/** Vertical slice through the current radar volume along two map-picked
 * points, 0-18 km MSL. Gaps between tilts are left empty, never filled. */
export function CrossSectionPanel({ line, picking, onStartPick, onClear, renderKey, render }: CrossSectionPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const complete = line?.b ? { a: line.a, b: line.b } : null;

  // Ref, not a dep: callers pass a fresh closure every render, and the slice
  // should only recompute when the line or `renderKey` changes.
  const renderRef = useRef(render);
  renderRef.current = render;

  useEffect(() => {
    const ctx = canvasRef.current?.getContext("2d");
    if (!ctx || !line?.b) return;
    ctx.clearRect(0, 0, COLS, ROWS);
    const bytes = renderRef.current(line.a, line.b, COLS, ROWS, TOP_KM);
    if (bytes) ctx.putImageData(new ImageData(new Uint8ClampedArray(bytes), COLS, ROWS), 0, 0);
  }, [line, renderKey]);

  const hint = picking ? (line ? "click the end point on the map" : "click the start point on the map") : null;

  return (
    <div className="cross-section-panel">
      <button type="button" onClick={onStartPick} disabled={picking}>
        {line ? "Pick new line" : "Pick line on map"}
      </button>{" "}
      {line && (
        <button type="button" onClick={onClear}>
          Clear
        </button>
      )}
      {hint && <div className="cross-section-hint">{hint}</div>}
      {complete && (
        <>
          <canvas
            ref={canvasRef}
            width={COLS}
            height={ROWS}
            style={{ width: "100%", imageRendering: "pixelated", background: "#111" }}
            aria-label="Radar vertical cross-section"
          />
          <div className="cross-section-axis">0–{TOP_KM} km MSL · start (left) → end (right)</div>
        </>
      )}
    </div>
  );
}
