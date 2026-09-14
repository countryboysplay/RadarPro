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
import { useStormMotion } from "./radar/useStormMotion";
import { DEFAULT_SITE_ICAO, findSite, RADAR_SITES } from "./sites";
import {
  loadPersistedDefaultSite,
  persistDefaultSite,
  loadPersistedFavoriteMoments,
  persistFavoriteMoments,
  loadPersistedCacheLimit,
  persistCacheLimit,
  loadPersistedRadarOpacity,
  persistRadarOpacity,
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
import { useForecastProvider, type ForecastPhase } from "./forecast/useForecastProvider";
import type { ForecastProviderId } from "./forecast/types";
import { useRainbowOverlay } from "./rainbow/useRainbowOverlay";
import { RainbowToggle } from "./ui/RainbowToggle";
import { useRainbowNowcast } from "./rainbow/useRainbowNowcast";
import { RainbowNowcastPanel, RainbowNowcastResults, nowcastHasContent } from "./ui/RainbowNowcastPanel";
import { useRainbowWeather } from "./rainbow/useRainbowWeather";
import { RainbowWeatherPanel, RainbowWeatherResults, weatherHasContent } from "./ui/RainbowWeatherPanel";
import { RightPanel, type RightPanelSection } from "./ui/RightPanel";
import { useMrmsOverlay, type MrmsViewport } from "./mrms/useMrmsOverlay";
import type { MrmsProductId } from "./mrms/types";
import { MrmsPanel } from "./ui/MrmsPanel";
import { SettingsPanel } from "./ui/SettingsPanel";
import { Sidebar } from "./ui/Sidebar";
import { SidebarSection } from "./ui/SidebarSection";
import { SidebarCategory } from "./ui/SidebarCategory";
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

/** Pick which moment to display for a sweep given the previously-selected
 * moment and the real wire codes ([`useRadarRenderer.momentWireCodesForSweep`])
 * that sweep actually carries. `"SRV"` (Storm-Relative Velocity, S11 Phase
 * 2b) is never itself in `availableMoments` -- it is a synthetic product
 * derived from VEL, never a real decoded wire moment (see
 * `useRadarRenderer.selectAndRenderStormRelativeVelocity`'s doc comment) --
 * so a previously-selected `"SRV"` is preserved whenever the new sweep still
 * carries `"VEL"` to derive it from, exactly mirroring how any other
 * previously-selected moment is preserved when the new sweep still carries
 * it directly. */
function resolvePreferredMoment(preferred: string | null, availableMoments: string[]): string | null {
  if (preferred === "SRV") {
    return availableMoments.includes("VEL") ? "SRV" : (availableMoments[0] ?? null);
  }
  return preferred && availableMoments.includes(preferred) ? preferred : (availableMoments[0] ?? null);
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

/** Which single overlay `MapView` currently shows -- live NEXRAD sweep,
 * Rainbow's tile overlay, or NOAA MRMS's national mosaic. Mirrors exactly
 * the mutual exclusivity `handleMrmsToggle`/`handleRainbowToggle` already
 * enforce (`rainbow.enabled` and `mrmsEnabled` can never both be true), so
 * this is a pure display-derived value, never a fourth piece of state that
 * could drift out of sync with the two flags underneath it. */
type ActiveMapLayerId = "radar" | "rainbow" | "mrms";

/** One-line live status summary shown next to "Map Layers" even while the
 * category is collapsed -- sidebar-redesign requirement that every
 * category header states its current state without opening it. */
function summarizeMapLayers(activeMapLayer: ActiveMapLayerId): string {
  switch (activeMapLayer) {
    case "radar":
      return "Live Radar Sweep active";
    case "rainbow":
      return "Rainbow Tiles active";
    case "mrms":
      return "MRMS active";
  }
}

/** One-line summary for "Point Forecasts" (Rainbow Nowcast (Point) +
 * Rainbow Outlook (Point)) -- "no active queries" until the user has
 * fetched at least one, matching those panels' own "never claim there's a
 * result before the user pressed Get" convention. */
function summarizePointForecasts(
  nowcastLoading: boolean,
  nowcastReady: boolean,
  weatherLoading: boolean,
  weatherReady: boolean,
): string {
  const parts: string[] = [];
  if (nowcastLoading) parts.push("Nowcast loading…");
  else if (nowcastReady) parts.push("Nowcast ready");
  if (weatherLoading) parts.push("Outlook loading…");
  else if (weatherReady) parts.push("Outlook ready");
  return parts.length > 0 ? parts.join(", ") : "no active queries";
}

/** One-line summary for "Model Forecast (GEFS/HRRR)". */
function summarizeModelForecast(providerId: ForecastProviderId | null, phase: ForecastPhase): string {
  if (!providerId) return "no model selected";
  const name = providerId.toUpperCase();
  switch (phase) {
    case "idle":
      return "no model selected";
    case "ready":
      return `${name} ready`;
    case "error":
      return `${name} error`;
    default:
      return `${name} loading…`;
  }
}

/** One-line summary for "Alerts" -- also what decides that category's
 * adaptive default-open state (see the effect near `alertsSectionOpen`
 * below): collapsed by default, but opened automatically the first time
 * there's something in it, unlike every other category (always collapsed
 * by default). */
function summarizeAlerts(activeCount: number): string {
  return activeCount > 0 ? `${activeCount} active` : "none active";
}

/** One-line summary for "Settings". */
function summarizeSettings(cacheEntriesCount: number, cacheLimit: number): string {
  return `cache ${cacheEntriesCount}/${cacheLimit} scans`;
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

  // UI polish pass: user-adjustable live-radar opacity (0..100%, default
  // 100 -- unchanged behavior until the user touches the slider), persisted
  // the same hydrate-then-persist way as `cacheLimit` just above (desktop-
  // only; a no-op in a plain browser tab, but the in-session state itself
  // works everywhere). See `MapView`'s `radarOpacity` prop doc comment for
  // why this only ever affects the "radar is the active layer" case.
  const [radarOpacityPercent, setRadarOpacityPercent] = useState(100);
  const [radarOpacityHydrated, setRadarOpacityHydrated] = useState(false);
  useEffect(() => {
    let cancelled = false;
    loadPersistedRadarOpacity().then((saved) => {
      if (cancelled) return;
      if (saved !== null) setRadarOpacityPercent(saved);
      setRadarOpacityHydrated(true);
    });
    return () => {
      cancelled = true;
    };
  }, []);
  useEffect(() => {
    if (!radarOpacityHydrated) return;
    persistRadarOpacity(radarOpacityPercent);
  }, [radarOpacityPercent, radarOpacityHydrated]);

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
    decodeError,
    decodeVolume,
    momentWireCodesForSweep,
    defaultSweepIndexForMoment,
    selectAndRender,
    selectAndRenderStormRelativeVelocity,
    loadColorTable,
    resetColorTable,
    activeColorTableJson,
    probeGate,
    rangeRings,
  } = useRadarRenderer(canvasRef);

  // S11 Phase 2b: Storm-Relative Velocity's storm-motion input state -- see
  // that hook's doc comment for why this is plain local state (no external
  // default to track, unlike `useRainbowPoint`).
  const stormMotion = useStormMotion();

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
  const rainbow = useRainbowOverlay(site);

  // S09d Parts B/C: Rainbow Nowcast + Weather (Forecast) point APIs --
  // distinct from the tile overlay above (`rainbow`) and from each other;
  // each owns its own fetch-on-demand phase machine (see the hooks' doc
  // comments). Both default their point inputs to the currently selected
  // radar site's lat/lon (`site`) -- this app's stand-in for "map center"
  // (the same value `useRainbowOverlay`'s snapshot probe already keys off
  // of) -- passed down as a plain prop, never read from MapLibre directly.
  const rainbowNowcast = useRainbowNowcast({ lon: site.lon, lat: site.lat });
  const rainbowWeather = useRainbowWeather({ lon: site.lon, lat: site.lat });

  // User feedback: once a result appears in the right-hand dock there was no
  // way to get it back off the screen. A plain "dismissed" flag per result
  // (not a third hook phase) -- cleared automatically the moment a fresh
  // fetch starts, so pressing "Get" again always brings the dock back even
  // if the previous result was dismissed.
  const [nowcastDismissed, setNowcastDismissed] = useState(false);
  const [weatherDismissed, setWeatherDismissed] = useState(false);
  useEffect(() => {
    if (rainbowNowcast.phase === "loading") setNowcastDismissed(false);
  }, [rainbowNowcast.phase]);
  useEffect(() => {
    if (rainbowWeather.phase === "loading") setWeatherDismissed(false);
  }, [rainbowWeather.phase]);

  // S09d follow-up ("click the map to set the point"): which panel's point
  // a map click should go to, or `null` when no pick is armed. At most one
  // of {nowcast, weather} can be armed at a time -- only one map click can
  // go to one target -- so this is a single tri-state value, not two
  // independent booleans that could both end up true.
  const [rainbowPointPickTarget, setRainbowPointPickTarget] = useState<"nowcast" | "weather" | null>(null);

  const handleToggleNowcastPick = useCallback(() => {
    setRainbowPointPickTarget((prev) => (prev === "nowcast" ? null : "nowcast"));
  }, []);
  const handleToggleWeatherPick = useCallback(() => {
    setRainbowPointPickTarget((prev) => (prev === "weather" ? null : "weather"));
  }, []);

  // The actual map click, routed to whichever panel is armed -- MapView
  // reports it as plain lat/lon (see its `onPointPick` doc comment), never
  // reaching into either hook's point state itself. One-shot: the pick
  // disarms itself the moment a click lands, and setting the point here
  // never triggers a fetch (S09d's "nothing calls the network until the
  // user presses Get" applies just as much to a map-click-filled point as
  // a hand-typed one).
  const handleRainbowMapPointPick = useCallback(
    (lat: number, lon: number) => {
      if (rainbowPointPickTarget === "nowcast") rainbowNowcast.setPoint(lon, lat);
      else if (rainbowPointPickTarget === "weather") rainbowWeather.setPoint(lon, lat);
      setRainbowPointPickTarget(null);
    },
    [rainbowPointPickTarget, rainbowNowcast, rainbowWeather],
  );

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

  // Sidebar redesign: the three previously-independent overlay checkboxes
  // (live radar sweep / Rainbow tiles / MRMS) are now presented as a
  // single "Active Map Layer" `<select>` in the "Map Layers" category --
  // exclusivity becomes the control itself, not hidden logic across three
  // separate toggles. `activeMapLayer` is purely derived (see its type's
  // doc comment above) and this handler is purely a UI consolidation: it
  // drives the exact same `rainbow.enabled`/`mrmsEnabled` flags through
  // the exact same `handleRainbowToggle`/`handleMrmsToggle` functions
  // above (which already do the real mutual-exclusivity work), so nothing
  // downstream (`MapView`'s `rainbowEnabled`/`mrmsEnabled`/radar-opacity
  // gating) needs to change at all.
  const activeMapLayer: ActiveMapLayerId = mrmsEnabled ? "mrms" : rainbow.enabled ? "rainbow" : "radar";
  const handleActiveMapLayerChange = useCallback(
    (next: ActiveMapLayerId) => {
      if (next === "mrms") {
        if (!mrmsEnabled) handleMrmsToggle();
      } else if (next === "rainbow") {
        if (!rainbow.enabled) handleRainbowToggle();
      } else {
        // "radar": turn off whichever of the other two (at most one) is on.
        if (mrmsEnabled) handleMrmsToggle();
        if (rainbow.enabled) handleRainbowToggle();
      }
    },
    [mrmsEnabled, rainbow.enabled, handleMrmsToggle, handleRainbowToggle],
  );

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

  // Sidebar redesign: "Alerts" gets an adaptive default -- collapsed like
  // every other category *unless* there are already active alerts the
  // moment the sidebar's data first settles, in which case it defaults
  // open (neither "always open" nor "always closed" is right: an always-
  // closed Alerts category could hide a live severe-weather alert behind
  // an extra click; an always-open one would defeat "collapsed by
  // default" for the overwhelmingly common no-alerts case). Applied
  // exactly once, after the poller's first non-"loading" result, so it
  // never fights a user who has since collapsed the category by hand.
  const alertsDefaultAppliedRef = useRef(false);
  useEffect(() => {
    if (alertsDefaultAppliedRef.current) return;
    if (alerts.status === "loading") return;
    alertsDefaultAppliedRef.current = true;
    if (alerts.activeAlerts.length > 0) setAlertsSectionOpen(true);
  }, [alerts.status, alerts.activeAlerts]);

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

  /** Render `moment` on `targetSweepIndex`, routing to the SRV-specific
   * wasm call for `"SRV"` (never sending that synthetic code through
   * `selectAndRender`, which expects a real wire code) and to the ordinary
   * `selectAndRender` for every other moment -- the one place this
   * distinction is made, so every caller below can treat "which moment is
   * selected" uniformly. */
  const renderSelection = useCallback(
    (targetSweepIndex: number, moment: string) => {
      if (moment === "SRV") {
        selectAndRenderStormRelativeVelocity(targetSweepIndex, stormMotion.speedMps, stormMotion.directionDeg);
      } else {
        selectAndRender(targetSweepIndex, moment);
      }
    },
    [selectAndRender, selectAndRenderStormRelativeVelocity, stormMotion.speedMps, stormMotion.directionDeg],
  );

  /** Change the displayed elevation, keeping the same moment when
   * possible (falling back to that sweep's first available moment
   * otherwise), and re-render immediately -- no re-decode. */
  const changeSweep = useCallback(
    (newSweepIndex: number) => {
      const moments = resolveMomentForSweep(newSweepIndex);
      const newMoment = resolvePreferredMoment(momentCodeRef.current, moments);
      setSweepIndex(newSweepIndex);
      setSweepMoments(moments);
      if (newMoment) {
        setMomentCode(newMoment);
        renderSelection(newSweepIndex, newMoment);
      }
    },
    [resolveMomentForSweep, renderSelection],
  );

  const changeMoment = useCallback(
    (newMoment: string) => {
      setMomentCode(newMoment);
      if (sweepIndexRef.current !== null) renderSelection(sweepIndexRef.current, newMoment);
    },
    [renderSelection],
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
      // "SRV" is never a real wire code any sweep carries (it is derived
      // from VEL, see `defaultSweepIndexForMoment`'s doc comment) --
      // resolve a previously-selected SRV the same way its own render path
      // does, by asking for "VEL" instead.
      const momentForDefaultSweep = momentCodeRef.current === "SRV" ? "VEL" : (momentCodeRef.current ?? "REF");
      newSweepIndex = defaultSweepIndexForMoment(momentForDefaultSweep) ?? 0;
    }
    const moments = momentWireCodesForSweep(newSweepIndex);
    const newMoment = resolvePreferredMoment(momentCodeRef.current, moments);

    setVolumeMeta(meta);
    setSweepIndex(newSweepIndex);
    setSweepMoments(moments);
    if (newMoment) {
      setMomentCode(newMoment);
      renderSelection(newSweepIndex, newMoment);
    }
    // Intentionally keyed on (status, currentKey) only: sweepIndex/
    // momentCode are read via refs so changing them does not itself
    // re-decode; decodeVolume/defaultSweepIndexForMoment/
    // momentWireCodesForSweep are stable per-renderer-instance callbacks,
    // and renderSelection's own identity changes only with stormMotion
    // (irrelevant here -- whatever value is current when this effect
    // actually runs is the one used), so none of this needs to be listed
    // (this project's eslint config does not enable react-hooks/exhaustive-deps).
  }, [status, currentKey]);

  // S11 Phase 2b: while Storm-Relative Velocity is the active moment,
  // adjusting either storm-motion input re-renders immediately -- a local
  // GPU re-render (no network/decode involved), so live feedback as the
  // user drags/types is the correct UX here, matching how changing the
  // Moment picker or color table elsewhere in this app already re-renders
  // immediately rather than waiting on an explicit "apply" action.
  useEffect(() => {
    if (momentCode !== "SRV" || sweepIndex === null) return;
    selectAndRenderStormRelativeVelocity(sweepIndex, stormMotion.speedMps, stormMotion.directionDeg);
  }, [momentCode, sweepIndex, stormMotion.speedMps, stormMotion.directionDeg, selectAndRenderStormRelativeVelocity]);

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
        case "Escape":
          // S09d follow-up: a way to cancel "Pick on map" without clicking
          // the map at all, alongside re-clicking the same panel button.
          if (rainbowPointPickTarget) {
            e.preventDefault();
            setRainbowPointPickTarget(null);
          }
          break;
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [history, stepElevation, rainbowPointPickTarget]);

  // UI polish pass on S09d: Rainbow Nowcast/Forecast result tables moved
  // out of the left sidebar into this right-docked panel -- see
  // `RightPanel.tsx`'s doc comment. Built fresh each render from each
  // hook's own phase (never a separate "should the dock show" state of its
  // own to keep in sync) -- `nowcastHasContent`/`weatherHasContent` gate
  // each entry so the dock never appears before the user has pressed "Get
  // Nowcast"/"Get Forecast" at least once.
  const rightPanelSections: RightPanelSection[] = [];
  if (nowcastHasContent(rainbowNowcast) && !nowcastDismissed) {
    rightPanelSections.push({
      key: "rainbow-nowcast",
      title: "Rainbow Nowcast (Point)",
      onDismiss: () => setNowcastDismissed(true),
      children: <RainbowNowcastResults nowcast={rainbowNowcast} />,
    });
  }
  if (weatherHasContent(rainbowWeather) && !weatherDismissed) {
    rightPanelSections.push({
      key: "rainbow-weather",
      title: "Rainbow Outlook (Point)",
      onDismiss: () => setWeatherDismissed(true),
      children: <RainbowWeatherResults weather={rainbowWeather} />,
    });
  }

  const elevationDeg = sweepIndex !== null ? (volumeMeta?.elevationDegs[sweepIndex] ?? null) : null;
  // S11 Phase 2b: "SRV" is offered in the Moment picker only when "VEL" is
  // present on the current sweep (SRV is meaningless without a VEL moment
  // to derive from) -- a display-only addition to `sweepMoments`, which
  // itself stays exactly the real wire codes `radar-web` reports (used
  // as-is for default-sweep-index resolution and the favorites row above).
  const momentOptions = sweepMoments.includes("VEL") ? [...sweepMoments, "SRV"] : sweepMoments;
  // Sidebar redesign: one-line live status summaries shown next to each
  // collapsed category header -- see each `summarize*` function's doc
  // comment above. Plain derived values recomputed every render, never
  // separate state that could go stale.
  const pointForecastsSummary = summarizePointForecasts(
    rainbowNowcast.phase === "loading",
    nowcastHasContent(rainbowNowcast),
    rainbowWeather.phase === "loading",
    weatherHasContent(rainbowWeather),
  );
  const modelForecastSummary = summarizeModelForecast(forecast.providerId, forecast.phase);
  const mapLayersSummary = summarizeMapLayers(activeMapLayer);
  const alertsSummary = summarizeAlerts(alerts.activeAlerts.length);
  const settingsSummary = summarizeSettings(history.entries.length, cacheLimit);

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
        pointPickActive={rainbowPointPickTarget !== null}
        onPointPick={handleRainbowMapPointPick}
        rainbowTileUrlTemplate={rainbow.tileUrlTemplate}
        rainbowEnabled={rainbow.enabled}
        rainbowMaxZoom={rainbow.layerInfo.maxZoom}
        mrmsCanvasRef={mrmsCanvasRef}
        mrmsEnabled={mrmsEnabled}
        onMrmsViewportChange={setMrmsViewport}
        radarOpacity={radarOpacityPercent / 100}
      />

      <RightPanel sections={rightPanelSections} />

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
          {/* S10 Phase 3: a volume that downloaded fine but failed to decode
              (truncated/corrupted transfer) used to throw uncaught out of
              the decode effect below and white-screen the whole app -- see
              `useRadarRenderer`'s `decodeVolume` doc comment. Now it can't
              crash anything, and this is that failure's one visible,
              always-present home instead of only a console line. */}
          {decodeError && (
            <span
              className="top-bar-status-item top-bar-status-error"
              title="The most recently downloaded volume failed to decode -- likely a truncated/corrupted transfer. The display keeps showing the last good frame."
            >
              decode error: {decodeError}
            </span>
          )}
        </div>
      </header>

      <Sidebar id="radarpro-sidebar" open={sidebarOpen}>
        <h1>RadarPro — workstation</h1>

        {/* Sidebar redesign: "Live Radar" is pinned -- always visible, never
            collapsible -- since it (site/elevation/moment/opacity) is used
            every session, unlike everything below it. Occasional/advanced
            tools that used to sit flat in this same section (Legend &
            Probe, the Color Table Editor, SRV storm motion) are now their
            own nested, collapsed-by-default disclosures instead of 9+ flat
            widgets in the single most-used part of the sidebar. */}
        <div className="sidebar-live-radar">
          <div className="sidebar-live-radar-title">Live Radar</div>
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
              {momentOptions.map((code) => (
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

          {/* UI polish pass: opacity control for the live radar sweep --
              only has a visible effect while the radar canvas is actually
              the active layer (`MapView` forces it fully invisible whenever
              Rainbow or MRMS is on, unaffected by this slider -- see that
              component's mutual-exclusivity comment). Lets map labels/city
              names underneath show through when turned down. */}
          <label title="Live radar opacity -- has no visible effect while Rainbow or MRMS is active">
            Radar opacity{" "}
            <input
              type="range"
              min={0}
              max={100}
              value={radarOpacityPercent}
              onChange={(e) => setRadarOpacityPercent(Number(e.target.value))}
            />{" "}
            {radarOpacityPercent}%
          </label>

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

          <SidebarSection title="Legend & Probe" nested>
            <Legend activeColorTableJson={activeTableJson} momentCode={momentCode ?? "—"} />
            <ProbePanel cursor={cursor} probe={probe} />
          </SidebarSection>

          {/* S11 Phase 2b: storm-motion input, only meaningful while Storm-
              Relative Velocity is the selected moment -- storm motion is
              conceptually tied to the moment selection, not a separate
              overlay/panel, so it's nested here rather than living in its
              own top-level category. Reuses the generic (despite the name)
              `.rainbow-panel-point`/`.rainbow-panel-field` layout classes
              rather than inventing new CSS. Changing either field re-renders
              immediately (see the `useEffect` keyed on this state above) --
              a local GPU re-render, not a network call, so no separate
              "apply" button is needed. */}
          {momentCode === "SRV" && (
            <SidebarSection title="Storm Motion (SRV)" nested>
              <div className="rainbow-panel-point">
                <label className="rainbow-panel-field">
                  <span>Storm speed (m/s)</span>
                  <input
                    type="number"
                    min={0}
                    step="1"
                    value={stormMotion.speedMps}
                    onChange={(e) => stormMotion.setSpeedMps(Number(e.target.value))}
                  />
                </label>
                <label
                  className="rainbow-panel-field"
                  title="Compass bearing, degrees clockwise from true north, that the storm is moving toward -- not the direction it is coming from."
                >
                  <span>Storm direction (° clockwise from true north)</span>
                  <input
                    type="number"
                    min={0}
                    max={359.999}
                    step="1"
                    value={stormMotion.directionDeg}
                    onChange={(e) => stormMotion.setDirectionDeg(Number(e.target.value))}
                  />
                </label>
              </div>
            </SidebarSection>
          )}

          {momentCode && (
            <SidebarSection title="Color Table Editor" nested>
              <ColorTableEditor
                momentCode={momentCode}
                activeColorTableJson={activeTableJson}
                onApply={handleApplyColorTable}
                onResetToBuiltinDefault={handleResetColorTable}
              />
            </SidebarSection>
          )}
        </div>

        {/* Sidebar redesign: the three previously-independent overlay
            checkboxes (live radar sweep / Rainbow Tiles / MRMS) are now one
            "Active Map Layer" dropdown -- exclusivity is the control
            itself. `RainbowToggle`/`MrmsPanel` below render only their own
            layer's configuration controls, and only while that layer is
            the one actually selected, matching Global Contract's
            "observations/forecasts/nowcasts must stay distinguishable" by
            never showing two overlays' configs at once. */}
        <SidebarCategory title="Map Layers" summary={mapLayersSummary}>
          <label className="map-layer-select">
            <span>Active Map Layer</span>
            <select
              value={activeMapLayer}
              onChange={(e) => handleActiveMapLayerChange(e.target.value as ActiveMapLayerId)}
            >
              <option value="radar">Live Radar Sweep</option>
              <option value="rainbow" disabled={!rainbow.configured}>
                Rainbow Tiles{!rainbow.configured ? " (not configured)" : ""}
              </option>
              <option value="mrms">MRMS National Mosaic</option>
            </select>
          </label>

          {activeMapLayer === "rainbow" && <RainbowToggle overlay={rainbow} />}

          {activeMapLayer === "mrms" && (
            <MrmsPanel
              productId={mrmsProductId}
              onProductChange={setMrmsProductId}
              enabled={mrmsEnabled}
              phase={mrms.phase}
              error={mrms.error}
              snapshot={mrms.snapshot}
              grid={mrms.grid}
              onRefresh={mrms.refresh}
            />
          )}
        </SidebarCategory>

        {/* S09d Parts B/C: Rainbow's two point-based APIs, grouped under one
            category -- distinct from both the tile overlay above and from
            each other, and named distinctly per Global Contract so neither
            is ever confused with the other or with the GEFS/HRRR "Model
            Forecast" category below. */}
        <SidebarCategory title="Point Forecasts" summary={pointForecastsSummary}>
          <SidebarSection title="Rainbow Nowcast (Point)" nested>
            <RainbowNowcastPanel
              nowcast={rainbowNowcast}
              pickModeActive={rainbowPointPickTarget === "nowcast"}
              onTogglePickMode={handleToggleNowcastPick}
            />
          </SidebarSection>

          <SidebarSection title="Rainbow Outlook (Point)" nested>
            <RainbowWeatherPanel
              weather={rainbowWeather}
              pickModeActive={rainbowPointPickTarget === "weather"}
              onTogglePickMode={handleToggleWeatherPick}
            />
          </SidebarSection>
        </SidebarCategory>

        {/* S08 follow-up: GEFS/HRRR forecast provider switcher + shared
            render path -- see `ForecastPanel`'s doc comment for why this is
            a dedicated panel rather than a MapView overlay in this stage. */}
        <SidebarCategory title="Model Forecast (GEFS/HRRR)" summary={modelForecastSummary}>
          <ForecastPanel canvasRef={forecastCanvasRef} forecast={forecast} />
        </SidebarCategory>

        {/* Sidebar redesign: collapsed by default like every other
            category, EXCEPT it defaults open instead the moment there
            already are active alerts when the poller's first result
            settles -- see the `alertsDefaultAppliedRef` effect above. */}
        <SidebarCategory title="Alerts" summary={alertsSummary} open={alertsSectionOpen} onToggle={setAlertsSectionOpen}>
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
        </SidebarCategory>

        <SidebarCategory title="Settings" summary={settingsSummary}>
          <SettingsPanel
            scanPollEvent={history.pollEvent}
            alertStatus={alerts.status}
            alertError={alerts.error}
            alertLastPolledAt={alerts.lastPolledAt}
            cacheLimit={cacheLimit}
            cacheEntriesCount={history.entries.length}
            onCacheLimitChange={setCacheLimit}
          />
        </SidebarCategory>
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
          isLiveStale={history.isLiveStale}
          currentVolumeAgeMillis={history.currentVolumeAgeMillis}
        />
        <div className="keyboard-hint">
          ← / → prev/next scan · Space play/pause · ↑ / ↓ elevation · L latest
        </div>
      </footer>
    </div>
  );
}
