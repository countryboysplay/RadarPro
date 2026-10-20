import { useEffect, useRef, useState } from "react";
import { fetchRaobSounding, fetchSounding, surfaceParcel, type Sounding, type SoundingLevel } from "../forecast/sounding";
import { celsiusToFahrenheit } from "../rainbow/units";
import type { LatLonPoint } from "./CrossSectionPanel";

export interface SoundingPanelProps {
  point: LatLonPoint | null;
  picking: boolean;
  onStartPick: () => void;
  onClear: () => void;
}

/** Rail controls: arm a map click for a sounding (HRRR or observed). The plot is `SoundingView`, in the right panel. */
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
      {picking && <div className="cross-section-hint">Click the map for a sounding</div>}
    </div>
  );
}

export interface SoundingViewProps {
  point: LatLonPoint;
  validMs: number;
  /** The HRRR run for the NOAA fallback (the loaded one, or an assumed recent one); null for none. */
  hrrrRunInitMs: number | null;
  /** Observed tab: the latest balloon launch at or before this (the archived scan, else now). */
  observedAtMs: number;
}

type Kind = "hrrr" | "observed";

/** Skew-T and hodograph at `point`: the HRRR column valid `validMs`, or the nearest observed balloon sounding. */
export function SoundingView({ point, validMs, hrrrRunInitMs, observedAtMs }: SoundingViewProps) {
  const [kind, setKind] = useState<Kind>("hrrr");
  const [state, setState] = useState<{ sounding: Sounding | null; error: string | null }>({ sounding: null, error: null });
  // Finished soundings by point and hour, so stepping or looping the forecast doesn't refetch them.
  const cache = useRef(new Map<string, Sounding>());
  // Balloons launch twice a day: key observed soundings by the 12-h slot, not the minute.
  const observedSlot = Math.floor(observedAtMs / 43_200_000);

  useEffect(() => {
    const key = kind === "hrrr" ? `${point.lat}|${point.lon}|${validMs}` : `obs|${point.lat}|${point.lon}|${observedSlot}`;
    const cached = cache.current.get(key);
    if (cached) {
      setState({ sounding: cached, error: null });
      return;
    }
    const controller = new AbortController();
    setState({ sounding: null, error: null });
    const request =
      kind === "hrrr"
        ? fetchSounding(point.lat, point.lon, validMs, hrrrRunInitMs, controller.signal)
        : fetchRaobSounding(point.lat, point.lon, (observedSlot + 1) * 43_200_000 - 1);
    request.then(
      (sounding) => {
        cache.current.set(key, sounding);
        if (!controller.signal.aborted) setState({ sounding, error: null });
      },
      (err) => !controller.signal.aborted && setState({ sounding: null, error: err instanceof Error ? err.message : String(err) }),
    );
    return () => controller.abort();
  }, [kind, point.lat, point.lon, validMs, hrrrRunInitMs, observedSlot]);

  const tabs = (
    <div className="sounding-tabs" role="group" aria-label="Sounding source">
      {(["hrrr", "observed"] as const).map((k) => (
        <button key={k} type="button" aria-pressed={kind === k} onClick={() => setKind(k)}>
          {k === "hrrr" ? "HRRR" : "Observed"}
        </button>
      ))}
    </div>
  );
  const { sounding, error } = state;
  const shownMs = kind === "hrrr" ? validMs : (sounding?.validMs ?? observedAtMs);
  const valid = new Date(shownMs).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit", hour12: true });
  if (error) return <div className="sounding-view">{tabs}<div className="probe-empty">No sounding: {error}</div></div>;
  if (!sounding) return <div className="sounding-view">{tabs}<div className="probe-empty">Loading {kind === "hrrr" ? `HRRR sounding for ${valid}` : "nearest balloon sounding"}…</div></div>;

  const parcel = surfaceParcel(sounding.levels);
  const sfc = sounding.levels[0];
  // The coarse column stops at 500 mb, which cuts the parcel's CAPE short; use HRRR's own there.
  const cape = sounding.coarse ? sounding.modelCape : parcel.cape;
  return (
    <div className="sounding-view">
      {tabs}
      <div className="sounding-meta">
        {sounding.site ? (
          <>
            <span className="sounding-observed">{sounding.site}</span> · launched {valid}
          </>
        ) : (
          <>
            <span className="sounding-valid">Valid {valid}</span> · {point.lat.toFixed(2)}, {point.lon.toFixed(2)}
          </>
        )}
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

// Data colours in the usual skew-T convention, kept apart from the chrome inks (DESIGN.md "Sounding plots").
const PLOT = { t: "#ff5a4d", td: "#3fbf5f", parcel: "#ecebe4", km3: "#ff5a4d", km6: "#3fbf5f", km9: "#e6d94a", km12: "#4ab8e6" };

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
      <polyline points={polyline(parcel)} fill="none" stroke={PLOT.parcel} strokeDasharray="4 3" />
      <polyline points={polyline(levels.filter((l) => !Number.isNaN(l.td)).map((l) => ({ p: l.p, t: l.td })))} fill="none" stroke={PLOT.td} strokeWidth={2} />
      <polyline points={polyline(levels.map((l) => ({ p: l.p, t: l.t })))} fill="none" stroke={PLOT.t} strokeWidth={2} />
    </svg>
  );
}

const HODO_SIZE = 190;

/** Height bands chasers read: 0-3, 3-6, 6-9, 9-12 km AGL. */
function hodoColor(aglM: number): string {
  if (aglM <= 3000) return PLOT.km3;
  if (aglM <= 6000) return PLOT.km6;
  if (aglM <= 9000) return PLOT.km9;
  return PLOT.km12;
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
