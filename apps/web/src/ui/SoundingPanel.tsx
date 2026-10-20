import { useEffect, useRef, useState } from "react";
import { fetchSounding, surfaceParcel, type Sounding, type SoundingLevel } from "../forecast/sounding";
import { celsiusToFahrenheit } from "../rainbow/units";
import type { LatLonPoint } from "./CrossSectionPanel";

export interface SoundingPanelProps {
  point: LatLonPoint | null;
  picking: boolean;
  onStartPick: () => void;
  onClear: () => void;
}

/** Rail controls: arm a map click for an HRRR sounding. The plot is `SoundingView`, in the right panel. */
export function SoundingPanel({ point, picking, onStartPick, onClear }: SoundingPanelProps) {
  return (
    <div className="sounding-panel">
      <div className="course-actions">
        <button type="button" onClick={onStartPick} disabled={picking}>
          {point ? "New sounding" : "Sounding at a point"}
        </button>
        {point && (
          <button type="button" className="button-quiet" onClick={onClear}>
            Clear
          </button>
        )}
      </div>
      {picking && <div className="cross-section-hint">Click the map for an HRRR sounding</div>}
    </div>
  );
}

export interface SoundingViewProps {
  point: LatLonPoint;
  validMs: number;
  /** The app's HRRR run, for the NOAA fallback; null when HRRR isn't loaded. */
  hrrrRunInitMs: number | null;
}

/** Skew-T and hodograph of the HRRR column at `point`, valid `validMs`. */
export function SoundingView({ point, validMs, hrrrRunInitMs }: SoundingViewProps) {
  const [state, setState] = useState<{ sounding: Sounding | null; error: string | null }>({ sounding: null, error: null });
  // Finished soundings by point and hour, so stepping or looping the forecast doesn't refetch them.
  const cache = useRef(new Map<string, Sounding>());

  useEffect(() => {
    const key = `${point.lat}|${point.lon}|${validMs}`;
    const cached = cache.current.get(key);
    if (cached) {
      setState({ sounding: cached, error: null });
      return;
    }
    const controller = new AbortController();
    setState({ sounding: null, error: null });
    fetchSounding(point.lat, point.lon, validMs, hrrrRunInitMs, controller.signal).then(
      (sounding) => {
        cache.current.set(key, sounding);
        setState({ sounding, error: null });
      },
      (err) => !controller.signal.aborted && setState({ sounding: null, error: err instanceof Error ? err.message : String(err) }),
    );
    return () => controller.abort();
  }, [point.lat, point.lon, validMs, hrrrRunInitMs]);

  const valid = new Date(validMs).toISOString().slice(0, 16).replace("T", " ") + "Z";
  const { sounding, error } = state;
  if (error) return <div className="probe-empty">No sounding: {error}</div>;
  if (!sounding) return <div className="probe-empty">Loading HRRR sounding for {valid}…</div>;

  const parcel = surfaceParcel(sounding.levels);
  const sfc = sounding.levels[0];
  // The coarse column stops at 500 mb, which cuts the parcel's CAPE short; use HRRR's own there.
  const cape = sounding.coarse ? sounding.modelCape : parcel.cape;
  return (
    <div className="sounding-view">
      <div className="sounding-meta">
        <span className="sounding-valid">Valid {valid}</span> · {point.lat.toFixed(2)}, {point.lon.toFixed(2)}
      </div>
      <div className="sounding-plots">
        <SkewT levels={sounding.levels} parcel={parcel.path} />
        <Hodograph levels={sounding.levels} />
      </div>
      <table className="course-table">
        <tbody>
          <tr>
            <td>Surface T / Td</td>
            <td className="num">
              {celsiusToFahrenheit(sfc.t).toFixed(0)} / {celsiusToFahrenheit(sfc.td).toFixed(0)} °F
            </td>
          </tr>
          <tr>
            <td>SBCAPE</td>
            <td className="num">{cape === null ? "—" : `${Math.round(cape).toLocaleString()} J/kg`}</td>
          </tr>
          <tr>
            <td>LCL height</td>
            <td className="num">{Math.round(parcel.lclAgl).toLocaleString()} m AGL</td>
          </tr>
        </tbody>
      </table>
      <div className="cross-section-axis">
        {sounding.source}
        {sounding.coarse && " — Open-Meteo didn't answer, so this is the coarse NOAA column"}
      </div>
    </div>
  );
}

const SKEW_W = 300;
const SKEW_H = 320;
const P_BOTTOM = 1050;
const P_TOP = 100;

function skewY(p: number): number {
  return SKEW_H - (SKEW_H * Math.log(P_BOTTOM / p)) / Math.log(P_BOTTOM / P_TOP);
}

/** 45° skew: −40..50 °C across the bottom edge. */
function skewX(t: number, p: number): number {
  return ((t + 40) / 90) * SKEW_W + (SKEW_H - skewY(p));
}

function polyline(points: { p: number; t: number }[]): string {
  return points.map(({ p, t }) => `${skewX(t, p).toFixed(1)},${skewY(p).toFixed(1)}`).join(" ");
}

function SkewT({ levels, parcel }: { levels: SoundingLevel[]; parcel: { p: number; t: number }[] }) {
  const isotherms = Array.from({ length: 16 }, (_, i) => -100 + i * 10);
  return (
    <svg className="sounding-skewt" viewBox={`0 0 ${SKEW_W} ${SKEW_H}`} role="img" aria-label="Skew-T diagram">
      {isotherms.map((t) => (
        <line key={t} x1={skewX(t, P_BOTTOM)} y1={SKEW_H} x2={skewX(t, P_TOP)} y2={0} stroke={t === 0 ? "var(--g6)" : "var(--g4)"} />
      ))}
      {[1000, 850, 700, 500, 300, 200].map((p) => (
        <g key={p}>
          <line x1={0} x2={SKEW_W} y1={skewY(p)} y2={skewY(p)} stroke="var(--g4)" />
          <text x={2} y={skewY(p) - 2} fill="var(--g6)" fontSize={10}>
            {p}
          </text>
        </g>
      ))}
      <polyline points={polyline(parcel)} fill="none" stroke="var(--g8)" strokeDasharray="4 3" />
      <polyline points={polyline(levels.map((l) => ({ p: l.p, t: l.td })))} fill="none" stroke="var(--ok)" strokeWidth={2} />
      <polyline points={polyline(levels.map((l) => ({ p: l.p, t: l.t })))} fill="none" stroke="var(--danger)" strokeWidth={2} />
    </svg>
  );
}

const HODO_SIZE = 190;

/** Height bands chasers read: 0-3, 3-6, 6-9, 9-12 km AGL. */
function hodoColor(aglM: number): string {
  if (aglM <= 3000) return "var(--danger)";
  if (aglM <= 6000) return "var(--ok)";
  if (aglM <= 9000) return "var(--forecast)";
  return "var(--accent)";
}

function Hodograph({ levels }: { levels: SoundingLevel[] }) {
  const zsfc = levels[0].z;
  const used = levels.filter((l) => l.z - zsfc <= 12_000);
  const ringStep = 20;
  const maxKt = Math.max(ringStep * 2, Math.ceil(Math.max(...used.map((l) => Math.hypot(l.u, l.v))) / ringStep) * ringStep);
  const scale = HODO_SIZE / 2 / maxKt;
  const c = HODO_SIZE / 2;
  const x = (u: number) => c + u * scale;
  const y = (v: number) => c - v * scale;
  return (
    <svg className="sounding-hodo" viewBox={`0 0 ${HODO_SIZE} ${HODO_SIZE}`} role="img" aria-label="Hodograph">
      {Array.from({ length: maxKt / ringStep }, (_, i) => (i + 1) * ringStep).map((r) => (
        <g key={r}>
          <circle cx={c} cy={c} r={r * scale} fill="none" stroke="var(--g4)" />
          <text x={c + r * scale * 0.71 + 2} y={c - r * scale * 0.71} fill="var(--g6)" fontSize={9}>
            {r}
          </text>
        </g>
      ))}
      <line x1={0} x2={HODO_SIZE} y1={c} y2={c} stroke="var(--g4)" />
      <line x1={c} x2={c} y1={0} y2={HODO_SIZE} stroke="var(--g4)" />
      {used.slice(1).map((l, i) => (
        <line key={l.p} x1={x(used[i].u)} y1={y(used[i].v)} x2={x(l.u)} y2={y(l.v)} stroke={hodoColor(l.z - zsfc)} strokeWidth={2.5} strokeLinecap="round" />
      ))}
    </svg>
  );
}
