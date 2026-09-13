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
import {
  loadPersistedDefaultSite,
  persistDefaultSite,
  loadPersistedFavoriteMoments,
  persistFavoriteMoments,
  loadPersistedCacheLimit,
  persistCacheLimit,
} from "./platform/desktop";
import { type PollEvent } from "./scan/useScanPoller";
import { MAX_HISTORY_SCANS, useScanHistory } from "./scan/useScanHistory";
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
import { useForecastProvider } from "./forecast/useForecastProvider";
import { useRainbowOverlay } from "./rainbow/useRainbowOverlay";
import { RainbowToggle } from "./ui/RainbowToggle";
import { useMrmsOverlay, type MrmsViewport } from "./mrms/useMrmsOverlay";
import type { MrmsProductId } from "./mrms/types";
import { MrmsPanel } from "./ui/MrmsPanel";
import { SettingsPanel } from "./ui/SettingsPanel";
import { Sidebar } from "./ui/Sidebar";
import { SidebarSection } from "./ui/SidebarSection";
import { UnifiedTimeline } from "./timeline/UnifiedTimeline";

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

  // S10 desktop shell: restore the last-selected default radar site on
  // launch. Resolves to `null` (no-op) in a plain browser tab -- see
  // `platform/desktop.ts`'s doc comment -- so this has no effect on the
  // standalone web app. Only overrides the built-in `DEFAULT_SITE_ICAO`
  // if a persisted, still-valid site was actually found. `siteHydrated`
  // gates the persist effect below so a slow initial load can never race
  // with (and get clobbered by) an eager write of the just-mounted
  // built-in default -- see this effect pair's combined doc comment.
  const [siteHydrated, setSiteHydrated] = useState(false);
  useEffect(() => {
    let cancelled = false;
    loadPersistedDefaultSite().then((savedIcao) => {
      if (cancelled) return;
      if (savedIcao && findSite(savedIcao)) {
        setIcao(savedIcao);
      }
      setSiteHydrated(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // Persist every site change as the new default -- but only once the
  // effect above has finished trying to load a previous one, so we never
  // overwrite a saved site with the built-in default before we've had a
  // chance to read it. No-op in a browser tab regardless.
  useEffect(() => {
    if (!siteHydrated) return;
    persistDefaultSite(icao);
  }, [icao, siteHydrated]);

  // S10 Phase 2: user-adjustable scan-history cache size, persisted the
  // same way as the default site above -- desktop-only persistence
  // (`loadPersistedCacheLimit`/`persistCacheLimit` are no-ops in a plain
  // browser tab), but the in-session state itself works everywhere.
  // `cacheLimitHydrated` gates the persist effect for the same reason
  // `siteHydrated` does above: never overwrite a saved value with the
  // built-in default before we've had a chance to read it.
  const [cacheLimit, setCacheLimit] = useState(MAX_HISTORY_SCANS);
  const [cacheLimitHydrated, setCacheLimitHydrated] = useState(false);
  useEffect(() => {
    let cancelled = false;
    loadPersistedCacheLimit().then((saved) => {
      if (cancelled) return;
      if (saved !== null) setCacheLimit(saved);
      setCacheLimitHydrated(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);
  useEffect(() => {
    if (!cacheLimitHydrated) return;
    persistCacheLimit(cacheLimit);
  }, [cacheLimit, cacheLimitHydrated]);

  // S10 Phase 2: favorited/starred moment codes (e.g. "REF", "VEL"),
  // surfaced as a star toggle + quick-select row next to the Moment picker
  // below. Same hydrate-then-persist pattern as the default site/cache
  // size above.
  const [favoriteMoments, setFavoriteMoments] = useState<string[]>([]);
  const [favoritesHydrated, setFavoritesHydrated] = useState(false);
  useEffect(() => {
    let cancelled = false;
    loadPersistedFavoriteMoments().then((saved) => {
      if (cancelled) return;
      if (saved) setFavoriteMoments(saved);
      setFavoritesHydrated(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);
  useEffect(() => {
    if (!favoritesHydrated) return;
    persistFavoriteMoments(favoriteMoments);
  }, [favoriteMoments, favoritesHydrated]);
  const toggleFavoriteMoment = useCallback((code: string) => {
    setFavoriteMoments((prev) => (prev.includes(code) ? prev.filter((c) => c !== code) : [...prev, code]));
  }, []);

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

  const history = useScanHistory(icao, cacheLimit);

  // S09: forecast state lifted here (out of `ForecastPanel`, which used to
  // own this hook entirely internally) so the unified timeline below can
  // both read forecast run/lead/grid metadata and drive `setLeadHours` --
  // see `ForecastPanel`'s doc comment. Still exactly one `useForecastProvider`
  // handle for the whole app.
  const forecastCanvasRef = useRef<HTMLCanvasElement>(null);
  const forecast = useForecastProvider(forecastCanvasRef);

  // Load a default provider on mount -- otherwise the forecast panel would
  // start on a blank "select a model" state with nothing to look at.
  //
  // Deliberately no ref-guard against re-running this: React 18
  // `StrictMode` (see `main.tsx`) double-invokes a mount effect in dev
  // (mount -> cleanup -> mount) specifically to flush out effects that
  // aren't safe to re-run -- `useForecastProvider`'s own unmount effect
  // already bumps its generation counter and frees the handle on that
  // first (simulated) cleanup, so a `didInitRef`-style "only call this
  // once, ever" guard here would suppress the second `selectProvider` call
  // the real remount needs, leaving the pipeline permanently stuck on its
  // now-invalidated first generation with no new one ever started
  // (confirmed during S08's own browser verification, when this effect
  // still lived in `ForecastPanel`). Calling `selectProvider` again on
  // every genuine mount is correct and cheap -- `selectProvider` itself is
  // the one place stale in-flight work gets discarded, via that same
  // generation counter.
  useEffect(() => {
    forecast.selectProvider("gefs");
    // Intentionally run once per real mount (empty deps -- this project's
    // eslint config does not enable react-hooks/exhaustive-deps):
    // `selectProvider` is a stable callback from a hook instance that lives
    // for this component's whole lifetime.
  }, []);

  // S06: NWS alerts. `useAlertPoller` owns the whole poll loop + wasm
  // `AlertStoreHandle`; this component only ever reads its current active
  // set/geojson and tracks which one (if any) is selected for the details
  // panel -- see that hook's module docs for the nationwide-poll-scope and
  // never-second-guess-the-store rationale.
  // S09b: Rainbow Weather precip nowcast overlay -- optional/keyed, config-
  // gated. Owns its own toggle state + snapshot resolution; see the hook's
  // doc comment and `RainbowToggle`/`MapView`'s mutual-exclusivity comment.
  const rainbow = useRainbowOverlay();

  // S09 Phase 3: MRMS national-mosaic overlay -- `enabled`/`productId`
  // (unlike Rainbow's own hook, which owns its toggle state internally)
  // live here, not inside `useMrmsOverlay` itself, specifically so this
  // component can enforce three-way mutual exclusivity with Rainbow (see
  // `handleMrmsToggle`/`handleRainbowToggle` below and `MapView`'s own
  // mutual-exclusivity doc comment) by actually flipping the *other*
  // overlay's toggle off, not just hiding it visually -- leaving Rainbow's
  // hook "on" in the background while invisible would keep it polling/
  // resolving snapshots for no visible effect, a wasteful and confusing
  // state no checkbox should be able to reach.
  const [mrmsEnabled, setMrmsEnabled] = useState(false);
  const [mrmsProductId, setMrmsProductId] = useState<MrmsProductId>("reflectivity");
  const [mrmsViewport, setMrmsViewport] = useState<MrmsViewport | null>(null);
  const mrmsCanvasRef = useRef<HTMLCanvasElement>(null);
  const mrms = useMrmsOverlay(mrmsCanvasRef, mrmsProductId, mrmsEnabled, mrmsViewport);

  const handleMrmsToggle = useCallback(() => {
    setMrmsEnabled((prev) => {
      const next = !prev;
      if (next && rainbow.enabled) rainbow.toggle(); // enforce mutual exclusivity -- see comment above.
      return next;
    });
  }, [rainbow]);

  const handleRainbowToggle = useCallback(() => {
    if (!rainbow.enabled && mrmsEnabled) setMrmsEnabled(false); // enforce mutual exclusivity -- see comment above.
    rainbow.toggle();
  }, [rainbow, mrmsEnabled]);

  const alerts = useAlertPoller();
  const [selectedAlertKey, setSelectedAlertKey] = useState<string | null>(null);

  // S09c UI shell: one collapsible sidebar replaces the always-on floating
  // hud-panel-* boxes -- closed by default so the map is the unobstructed
  // default view (see the stage file). `alertsSectionOpen` is controlled
  // (rather than left to `SidebarSection`'s own internal state) so
  // selecting an alert -- from the map click handled below, or from the
  // list inside the section itself -- can force both the sidebar and the
  // Alerts section open, matching this panel's old behavior of appearing
  // immediately as a floating box the instant an alert was selected.
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [alertsSectionOpen, setAlertsSectionOpen] = useState(false);
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

  // Auto-reveal the Alerts section (and sidebar) whenever an alert becomes
  // selected -- including via a map click, which happens with no sidebar
  // DOM in view at all. Without this, selecting an alert while the
  // sidebar is closed would have no visible effect, a regression from the
  // old always-on floating `AlertDetail` panel.
  useEffect(() => {
    if (selectedAlertKey) {
      setSidebarOpen(true);
      setAlertsSectionOpen(true);
    }
  }, [selectedAlertKey]);

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
        rainbowTileUrlTemplate={rainbow.tileUrlTemplate}
        rainbowEnabled={rainbow.enabled}
        mrmsCanvasRef={mrmsCanvasRef}
        mrmsEnabled={mrmsEnabled}
        onMrmsViewportChange={setMrmsViewport}
      />

      {/* S09c UI shell: slim always-visible top/bottom chrome for the most
          glanceable controls (GPU/scan-feed status, playback), modeled on
          RadarScope's own minimal top/bottom bars -- everything else lives
          behind the sidebar toggle below. */}
      <header className="top-bar">
        <div className="top-bar-left">
          <button
            type="button"
            className="sidebar-toggle"
            onClick={() => setSidebarOpen((v) => !v)}
            aria-expanded={sidebarOpen}
            aria-controls="radarpro-sidebar"
            title={sidebarOpen ? "Close sidebar" : "Open sidebar"}
          >
            {sidebarOpen ? "✕" : "☰"}
          </button>
          <span className="top-bar-title">RadarPro</span>
        </div>
        <div className="top-bar-status">
          <span
            className={`top-bar-status-item${status === "error" ? " top-bar-status-error" : ""}`}
            title="GPU renderer status"
          >
            {status === "loading" && "GPU: initializing…"}
            {status === "error" && `GPU error: ${error}`}
            {status === "ready" && `GPU: ${adapterName || "(adapter name withheld)"} (${backend})`}
          </span>
          <span className="top-bar-status-item" title="Scan feed status">
            {describePollEvent(history.pollEvent)}
          </span>
        </div>
      </header>

      <Sidebar id="radarpro-sidebar" open={sidebarOpen}>
        <h1>RadarPro — workstation</h1>

        <SidebarSection title="Radar" defaultOpen>
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
            <select
              value={momentCode ?? ""}
              disabled={sweepMoments.length === 0}
              onChange={(e) => changeMoment(e.target.value)}
            >
              {sweepMoments.map((code) => (
                <option key={code} value={code}>
                  {code}
                </option>
              ))}
            </select>{" "}
            {momentCode && (
              <button
                type="button"
                onClick={() => toggleFavoriteMoment(momentCode)}
                aria-pressed={favoriteMoments.includes(momentCode)}
                title={favoriteMoments.includes(momentCode) ? `Remove ${momentCode} from favorites` : `Favorite ${momentCode}`}
                style={{ background: "none", border: "none", cursor: "pointer", fontSize: "1.1em", lineHeight: 1, padding: "0 0.2em" }}
              >
                {favoriteMoments.includes(momentCode) ? "★" : "☆"}
              </button>
            )}
          </label>

          {/* S10 Phase 2: quick-select row for favorited moments that are
              actually available on the current sweep -- a favorite for a
              moment this sweep doesn't carry (e.g. VEL favorited while
              looking at a surveillance-only sweep) is simply omitted rather
              than shown disabled, keeping this a minimal proof of concept
              rather than a speculative framework. */}
          {favoriteMoments.filter((code) => sweepMoments.includes(code)).length > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: "0.3em", margin: "0.3em 0" }}>
              {favoriteMoments
                .filter((code) => sweepMoments.includes(code))
                .map((code) => (
                  <button
                    key={code}
                    type="button"
                    onClick={() => changeMoment(code)}
                    aria-pressed={code === momentCode}
                    style={{
                      fontSize: "0.85em",
                      padding: "0.1em 0.5em",
                      fontWeight: code === momentCode ? "bold" : "normal",
                    }}
                  >
                    ★ {code}
                  </button>
                ))}
            </div>
          )}

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
        </SidebarSection>

        <SidebarSection title="Alerts" open={alertsSectionOpen} onToggle={setAlertsSectionOpen}>
          <AlertsPanel
            status={alerts.status}
            error={alerts.error}
            alerts={alerts.activeAlerts}
            selectedKey={selectedAlertKey}
            onSelect={handleAlertClick}
            lastPolledAt={alerts.lastPolledAt}
            heldCount={alerts.heldCount}
          />
          {selectedAlert && (
            <AlertDetail
              alert={selectedAlert}
              isActive={selectedActiveEntry !== null}
              onClose={() => setSelectedAlertKey(null)}
            />
          )}
        </SidebarSection>

        {/* S08 follow-up: GEFS/HRRR forecast provider switcher + shared
            render path -- see `ForecastPanel`'s doc comment for why this is
            a dedicated panel rather than a MapView overlay in this stage. */}
        <SidebarSection title="Forecast">
          <ForecastPanel canvasRef={forecastCanvasRef} forecast={forecast} />
        </SidebarSection>

        <SidebarSection title="Rainbow">
          <RainbowToggle
            configured={rainbow.configured}
            enabled={rainbow.enabled}
            status={rainbow.status}
            error={rainbow.error}
            onToggle={handleRainbowToggle}
          />
        </SidebarSection>

        {/* S09 Phase 3: NOAA MRMS national radar-mosaic observation overlay
            -- see `MrmsPanel`'s doc comment and `MapView`'s mutual-
            exclusivity comment for why this, Rainbow, and the live radar
            sweep can never all show at once. */}
        <SidebarSection title="MRMS">
          <MrmsPanel
            productId={mrmsProductId}
            onProductChange={setMrmsProductId}
            enabled={mrmsEnabled}
            onToggle={handleMrmsToggle}
            phase={mrms.phase}
            error={mrms.error}
            snapshot={mrms.snapshot}
            grid={mrms.grid}
            onRefresh={mrms.refresh}
          />
        </SidebarSection>

        <SidebarSection title="Settings">
          <SettingsPanel
            scanPollEvent={history.pollEvent}
            alertStatus={alerts.status}
            alertError={alerts.error}
            alertLastPolledAt={alerts.lastPolledAt}
            cacheLimit={cacheLimit}
            cacheEntriesCount={history.entries.length}
            onCacheLimitChange={setCacheLimit}
          />
        </SidebarSection>
      </Sidebar>

      <footer className="bottom-bar">
        {/* S09: unified real-time-axis timeline -- see
            `timeline/UnifiedTimeline.tsx`'s doc comment. Drives the same
            `history`/`forecast` selections `PlaybackControls`/`ForecastPanel`
            already read/write; it is an additional way to set them, not a
            replacement rendering path. */}
        <UnifiedTimeline history={history} forecast={forecast} />
        <PlaybackControls
          playMode={history.playMode}
          currentIndex={history.currentIndex}
          entriesCount={history.entries.length}
          currentTimeMillis={currentEntry?.startTimeMillis ?? null}
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
      </footer>
    </div>
  );
}
