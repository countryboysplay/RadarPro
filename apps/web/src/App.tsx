// RadarPro single-panel radar workstation -- Stage S05 (Radar Workstation).
// See `Agent Context/context/stages/S05-radar-workstation.md`.
//
// Builds on S04's live-radar map view (`MapView`, `useScanPoller`,
// `radar-web`'s decode/render pipeline) with: moment/elevation pickers
// driven by the decoded volume's own metadata, previous/next/loop
// animation over a bounded scan-history cache (`useScanHistory`), a
// mouse-driven data probe + geographic cursor readout, range rings, a
// legend built from the active color table's own stops, a minimal
// color-table editor, and a handful of keyboard shortcuts.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import "./App.css";
import { MapView } from "./map/MapView";
import { RADAR_CANVAS_SIZE, useRadarRenderer, type ProbeResult } from "./radar/useRadarRenderer";
import { DEFAULT_SITE_ICAO, findSite, RADAR_SITES } from "./sites";
import { type PollEvent } from "./scan/useScanPoller";
import { useScanHistory } from "./scan/useScanHistory";
import { InfoPanel } from "./ui/InfoPanel";
import { PlaybackControls } from "./ui/PlaybackControls";
import { ProbePanel } from "./ui/ProbePanel";
import { Legend } from "./ui/Legend";
import { ColorTableEditor } from "./ui/ColorTableEditor";
import { AlertsPanel } from "./ui/AlertsPanel";
import { AlertDetail } from "./ui/AlertDetail";
import { useAlertPoller } from "./alerts/useAlertPoller";
import type { AlertJson } from "./alerts/types";
import { ForecastPanel } from "./forecast/ForecastPanel";

/** Fixed reference radii (km) for the range rings drawn around the
 * selected site -- a documented, reasonable default (not derived from any
 * per-scan value) rather than the sweep's real unambiguous range. */
const RANGE_RING_RADII_KM = [50, 100, 150, 200];
/** Points per ring; 128 is smooth at any zoom level this app's map
 * supports without generating a wastefully large GeoJSON payload. */
const RANGE_RING_NUM_POINTS = 128;

/**
 * Keyboard shortcuts (ignored while typing in the color-table editor's
 * textarea or any `<select>`/`<input>`, so they never fight with editing):
 *   - Left / Right arrow: previous / next scan (from the held history)
 *   - Space: play / pause the loop
 *   - Up / Down arrow: step elevation (sweep index) up / down
 *   - L: jump to the latest scan and resume live polling
 */
function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "TEXTAREA" || tag === "INPUT" || tag === "SELECT" || target.isContentEditable;
}

function describePollEvent(event: PollEvent): string {
  switch (event.type) {
    case "checking":
      return "checking for new scan…";
    case "downloaded":
      return `new scan received: ${event.volume.key.split("/").pop()}`;
    case "up-to-date":
      return `up to date (${event.volume.key.split("/").pop()})`;
    case "none-available":
      return "no scans available yet for today (UTC)";
    case "error":
      return `error: ${event.message}`;
  }
}

export default function App() {
  const [icao, setIcao] = useState(DEFAULT_SITE_ICAO);
  const site = findSite(icao) ?? RADAR_SITES[0];

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const {
    status,
    error,
    adapterName,
    backend,
    decodeVolume,
    momentWireCodesForSweep,
    defaultSweepIndexForMoment,
    selectAndRender,
    loadColorTable,
    resetColorTable,
    activeColorTableJson,
    probeGate,
    rangeRings,
  } = useRadarRenderer(canvasRef);

  const history = useScanHistory(icao);

  // S06: NWS alerts. `useAlertPoller` owns the whole poll loop + wasm
  // `AlertStoreHandle`; this component only ever reads its current active
  // set/geojson and tracks which one (if any) is selected for the details
  // panel -- see that hook's module docs for the nationwide-poll-scope and
  // never-second-guess-the-store rationale.
  const alerts = useAlertPoller();
  const [selectedAlertKey, setSelectedAlertKey] = useState<string | null>(null);
  // Last-known content for every key ever seen active, kept only so a
  // still-open details panel can keep showing an alert's final content
  // (with a "no longer active" banner) after `AlertStore` removes it from
  // the active set, instead of the panel's content abruptly vanishing.
  // This never changes *whether*/*when* an alert is considered active --
  // that remains entirely `alerts.activeAlerts`, read fresh every render.
  const lastKnownAlertsRef = useRef<Map<string, AlertJson>>(new Map());
  for (const { key, alert } of alerts.activeAlerts) {
    lastKnownAlertsRef.current.set(key, alert);
  }
  const selectedActiveEntry = alerts.activeAlerts.find((a) => a.key === selectedAlertKey) ?? null;
  const selectedAlert = selectedActiveEntry?.alert ?? (selectedAlertKey ? lastKnownAlertsRef.current.get(selectedAlertKey) ?? null : null);
  const handleAlertClick = useCallback((key: string | null) => {
    setSelectedAlertKey(key);
  }, []);

  // Decoded-volume/selection state. Kept small and derived -- the raw
  // bytes themselves never enter React state (GLOBAL_CONTRACT).
  const [volumeMeta, setVolumeMeta] = useState<{
    siteIcao: string;
    sweepCount: number;
    elevationDegs: number[];
  } | null>(null);
  const [sweepIndex, setSweepIndex] = useState<number | null>(null);
  const [momentCode, setMomentCode] = useState<string | null>(null);
  const [sweepMoments, setSweepMoments] = useState<string[]>([]);
  const [colorTableVersion, setColorTableVersion] = useState(0);

  // Mirrors of the state above for use inside callbacks/effects that must
  // not themselves depend on (and re-run for) every selection change.
  const sweepIndexRef = useRef<number | null>(null);
  const momentCodeRef = useRef<string | null>(null);
  useEffect(() => {
    sweepIndexRef.current = sweepIndex;
  }, [sweepIndex]);
  useEffect(() => {
    momentCodeRef.current = momentCode;
  }, [momentCode]);

  // Switching sites invalidates any previous selection -- a new volume's
  // sweep/moment layout may not resemble the old site's at all.
  useEffect(() => {
    setVolumeMeta(null);
    setSweepIndex(null);
    setMomentCode(null);
    setSweepMoments([]);
  }, [icao]);

  /** Pick/validate a moment for `sweepIndex`, preferring the
   * currently-selected moment if that sweep still carries it. */
  const resolveMomentForSweep = useCallback(
    (newSweepIndex: number): string[] => momentWireCodesForSweep(newSweepIndex),
    [momentWireCodesForSweep],
  );

  /** Change the displayed elevation, keeping the same moment when
   * possible (falling back to that sweep's first available moment
   * otherwise), and re-render immediately -- no re-decode. */
  const changeSweep = useCallback(
    (newSweepIndex: number) => {
      const moments = resolveMomentForSweep(newSweepIndex);
      const preferred = momentCodeRef.current;
      const newMoment = preferred && moments.includes(preferred) ? preferred : (moments[0] ?? null);
      setSweepIndex(newSweepIndex);
      setSweepMoments(moments);
      if (newMoment) {
        setMomentCode(newMoment);
        selectAndRender(newSweepIndex, newMoment);
      }
    },
    [resolveMomentForSweep, selectAndRender],
  );

  const changeMoment = useCallback(
    (newMoment: string) => {
      setMomentCode(newMoment);
      if (sweepIndexRef.current !== null) selectAndRender(sweepIndexRef.current, newMoment);
    },
    [selectAndRender],
  );

  const stepElevation = useCallback(
    (delta: number) => {
      if (sweepIndexRef.current === null || !volumeMeta) return;
      const next = Math.max(0, Math.min(volumeMeta.sweepCount - 1, sweepIndexRef.current + delta));
      if (next !== sweepIndexRef.current) changeSweep(next);
    },
    [volumeMeta, changeSweep],
  );

  // The scan currently selected by history playback -- decode it (once
  // per distinct held frame, never once per moment/elevation change) and
  // select/render whatever elevation+moment was already chosen, falling
  // back to sensible defaults the first time or if the new volume doesn't
  // carry the previous selection.
  const currentEntry = history.entries[history.currentIndex] ?? null;
  const currentKey = currentEntry?.key ?? null;
  useEffect(() => {
    if (status !== "ready" || !currentKey) return;
    const bytes = history.currentBytes();
    if (!bytes) return;

    const meta = decodeVolume(bytes);
    if (!meta) return;

    let newSweepIndex = sweepIndexRef.current;
    if (newSweepIndex === null || newSweepIndex >= meta.sweepCount) {
      newSweepIndex = defaultSweepIndexForMoment(momentCodeRef.current ?? "REF") ?? 0;
    }
    const moments = momentWireCodesForSweep(newSweepIndex);
    const preferred = momentCodeRef.current;
    const newMoment = preferred && moments.includes(preferred) ? preferred : (moments[0] ?? null);

    setVolumeMeta(meta);
    setSweepIndex(newSweepIndex);
    setSweepMoments(moments);
    if (newMoment) {
      setMomentCode(newMoment);
      selectAndRender(newSweepIndex, newMoment);
    }
    // Intentionally keyed on (status, currentKey) only: sweepIndex/
    // momentCode are read via refs so changing them does not itself
    // re-decode; decodeVolume/defaultSweepIndexForMoment/
    // momentWireCodesForSweep/selectAndRender are stable per-renderer-
    // instance callbacks (this project's eslint config does not enable
    // react-hooks/exhaustive-deps).
  }, [status, currentKey]);

  // Range rings: recomputed only when the site or renderer readiness
  // changes -- plain coordinate geometry from radar-web, handed to
  // MapView (the MapLibre-aware adapter) to actually draw.
  const [rangeRingsGeo, setRangeRingsGeo] = useState<number[][][] | null>(null);
  useEffect(() => {
    if (status !== "ready") {
      setRangeRingsGeo(null);
      return;
    }
    setRangeRingsGeo(rangeRings(site.lat, site.lon, RANGE_RING_RADII_KM, RANGE_RING_NUM_POINTS));
  }, [status, site.lat, site.lon, rangeRings]);

  // Data probe + geographic cursor readout.
  const [cursor, setCursor] = useState<{ lat: number; lon: number } | null>(null);
  const [probe, setProbe] = useState<ProbeResult | null>(null);
  const lastProbeAtRef = useRef(0);

  const handleCursorMove = useCallback(
    (lat: number, lon: number) => {
      setCursor({ lat, lon });
      const now = performance.now();
      // Throttle the actual wasm probe call (~30/s) -- the lat/lon readout
      // itself stays smooth since `setCursor` above runs on every event.
      if (now - lastProbeAtRef.current < 33) return;
      lastProbeAtRef.current = now;
      if (sweepIndexRef.current === null || !momentCodeRef.current) {
        setProbe(null);
        return;
      }
      setProbe(probeGate(site.lat, site.lon, sweepIndexRef.current, momentCodeRef.current, lat, lon));
    },
    [probeGate, site.lat, site.lon],
  );
  const handleCursorLeave = useCallback(() => {
    setCursor(null);
    setProbe(null);
  }, []);

  // Color-table editor plumbing: `activeColorTableJson` reads current wasm
  // state directly (not React state), so a version counter forces a
  // re-read after any apply/reset.
  const activeTableJson = useMemo(() => {
    if (status !== "ready" || !momentCode) return null;
    return activeColorTableJson(momentCode);
    // `colorTableVersion` is a deliberate cache-buster: `activeColorTableJson`
    // reads live wasm state that can change (via loadColorTable/
    // resetColorTable) without any of this memo's other deps changing.
  }, [status, momentCode, colorTableVersion, activeColorTableJson]);

  const handleApplyColorTable = useCallback(
    (json: string) => {
      const result = loadColorTable(json);
      if (result.ok) {
        setColorTableVersion((v) => v + 1);
        if (sweepIndexRef.current !== null && momentCodeRef.current) {
          selectAndRender(sweepIndexRef.current, momentCodeRef.current);
        }
      }
      return result;
    },
    [loadColorTable, selectAndRender],
  );

  const handleResetColorTable = useCallback(() => {
    if (!momentCode) return;
    resetColorTable(momentCode);
    setColorTableVersion((v) => v + 1);
    if (sweepIndexRef.current !== null && momentCodeRef.current) {
      selectAndRender(sweepIndexRef.current, momentCodeRef.current);
    }
  }, [momentCode, resetColorTable, selectAndRender]);

  // Keyboard shortcuts -- see the module doc comment for the full list.
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (isEditableTarget(e.target)) return;
      switch (e.key) {
        case "ArrowLeft":
          e.preventDefault();
          history.previous();
          break;
        case "ArrowRight":
          e.preventDefault();
          history.next();
          break;
        case " ":
          e.preventDefault();
          history.togglePlay();
          break;
        case "ArrowUp":
          e.preventDefault();
          stepElevation(1);
          break;
        case "ArrowDown":
          e.preventDefault();
          stepElevation(-1);
          break;
        case "l":
        case "L":
          e.preventDefault();
          history.jumpToLatest();
          break;
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [history, stepElevation]);

  const elevationDeg = sweepIndex !== null ? (volumeMeta?.elevationDegs[sweepIndex] ?? null) : null;
  const activeTableParsedUnits = useMemo(() => {
    if (!activeTableJson) return null;
    try {
      return (JSON.parse(activeTableJson) as { units?: string }).units ?? null;
    } catch {
      return null;
    }
  }, [activeTableJson]);

  return (
    <div style={{ position: "fixed", inset: 0 }}>
      <MapView
        site={site}
        canvasRef={canvasRef}
        canvasSize={RADAR_CANVAS_SIZE}
        rangeRings={rangeRingsGeo}
        onCursorMove={handleCursorMove}
        onCursorLeave={handleCursorLeave}
        alerts={alerts.geojson}
        selectedAlertKey={selectedAlertKey}
        onAlertClick={handleAlertClick}
      />

      <div className="hud-panel hud-panel-left">
        <h1>RadarPro — workstation (S05)</h1>

        <label>
          Site{" "}
          <select value={icao} onChange={(e) => setIcao(e.target.value)}>
            {RADAR_SITES.map((s) => (
              <option key={s.icao} value={s.icao}>
                {s.icao} — {s.name}
              </option>
            ))}
          </select>
        </label>

        <label>
          Elevation{" "}
          <select
            value={sweepIndex ?? ""}
            disabled={!volumeMeta}
            onChange={(e) => changeSweep(Number(e.target.value))}
          >
            {volumeMeta?.elevationDegs.map((deg, i) => (
              <option key={i} value={i}>
                {i}: {deg.toFixed(2)}°
              </option>
            ))}
          </select>
        </label>

        <label>
          Moment{" "}
          <select value={momentCode ?? ""} disabled={sweepMoments.length === 0} onChange={(e) => changeMoment(e.target.value)}>
            {sweepMoments.map((code) => (
              <option key={code} value={code}>
                {code}
              </option>
            ))}
          </select>
        </label>

        <dl>
          <dt>GPU</dt>
          <dd>
            {status === "loading" && "initializing…"}
            {status === "error" && `error: ${error}`}
            {status === "ready" && `${adapterName || "(adapter name withheld)"} (${backend})`}
          </dd>
          <dt>Scan feed</dt>
          <dd>{describePollEvent(history.pollEvent)}</dd>
        </dl>

        <InfoPanel
          siteIcao={site.icao}
          siteName={site.name}
          volumeStartTimeMillis={currentEntry?.startTimeMillis ?? null}
          elevationDeg={elevationDeg}
          sweepIndex={sweepIndex}
          sweepCount={volumeMeta?.sweepCount ?? null}
          momentCode={momentCode}
          units={activeTableParsedUnits}
        />

        <PlaybackControls
          playMode={history.playMode}
          currentIndex={history.currentIndex}
          entriesCount={history.entries.length}
          frameMs={history.frameMs}
          onPrevious={history.previous}
          onNext={history.next}
          onTogglePlay={history.togglePlay}
          onJumpLatest={history.jumpToLatest}
          onFrameMsChange={history.setFrameMs}
        />

        <div className="keyboard-hint">
          ← / → prev/next scan · Space play/pause · ↑ / ↓ elevation · L latest
        </div>
      </div>

      <div className="hud-panel hud-panel-right">
        <Legend activeColorTableJson={activeTableJson} momentCode={momentCode ?? "—"} />
        <ProbePanel cursor={cursor} probe={probe} />
        {momentCode && (
          <ColorTableEditor
            momentCode={momentCode}
            activeColorTableJson={activeTableJson}
            onApply={handleApplyColorTable}
            onResetToBuiltinDefault={handleResetColorTable}
          />
        )}
      </div>

      <div className="hud-panel hud-panel-alerts">
        <AlertsPanel
          status={alerts.status}
          error={alerts.error}
          alerts={alerts.activeAlerts}
          selectedKey={selectedAlertKey}
          onSelect={handleAlertClick}
          lastPolledAt={alerts.lastPolledAt}
          heldCount={alerts.heldCount}
        />
      </div>

      {selectedAlert && (
        <div className="hud-panel hud-panel-alert-detail">
          <AlertDetail alert={selectedAlert} isActive={selectedActiveEntry !== null} onClose={() => setSelectedAlertKey(null)} />
        </div>
      )}

      {/* S08 follow-up: GEFS/HRRR forecast provider switcher + shared
          render path -- see `ForecastPanel`'s doc comment for why this is
          a dedicated panel rather than a MapView overlay in this stage. */}
      <div className="hud-panel hud-panel-forecast">
        <ForecastPanel />
      </div>
    </div>
  );
}
