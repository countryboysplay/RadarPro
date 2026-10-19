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
  isDesktop,
  notifyDesktop,
} from "./platform/desktop";
import { type PollEvent } from "./scan/useScanPoller";
import { MAX_HISTORY_SCANS, useScanHistory } from "./scan/useScanHistory";
import { InfoPanel } from "./ui/InfoPanel";
import { PlaybackControls } from "./ui/PlaybackControls";
import { ArchiveLoader } from "./ui/ArchiveLoader";
import { CrossSectionPanel, legLengthAndBearing, type LatLonPoint } from "./ui/CrossSectionPanel";
import { ProbePanel } from "./ui/ProbePanel";
import { Legend } from "./ui/Legend";
import { ColorTableEditor } from "./ui/ColorTableEditor";
import { AlertsPanel } from "./ui/AlertsPanel";
import { AlertDetail } from "./ui/AlertDetail";
import { useAlertPoller } from "./alerts/useAlertPoller";
import type { AlertJson } from "./alerts/types";
import { alertNearSite } from "./alerts/geometryHitTest";
import { SEVERITY_COLORS } from "./alerts/types";
import { coveringAlerts, playAlertTone, useMyLocation } from "./location/useMyLocation";
import { ForecastPanel } from "./forecast/ForecastPanel";
import { leadHourOptions, useForecastProvider, type ForecastPhase } from "./forecast/useForecastProvider";
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
import { SidebarSection } from "./ui/SidebarSection";
import { SidebarCategory } from "./ui/SidebarCategory";
import { UnifiedTimeline } from "./timeline/UnifiedTimeline";
import { Icon } from "./ui/Icon";

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
    case "receiving": {
      const time = new Date(event.volume.startTimeMillis).toLocaleTimeString(undefined, { hour12: true });
      return `live feed · scan ${time} ${event.complete ? "complete" : "arriving"}`;
    }
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
function summarizeAlerts(nearCount: number, activeCount: number, icao: string): string {
  return activeCount > 0 ? `${activeCount} US · ${nearCount} polygons near ${icao}` : "none active";
}

/** A native toast in the desktop app, a web Notification in a browser tab. */
function showNotification(title: string, body: string, tag: string, sticky: boolean, onClick: () => void) {
  if (isDesktop()) return void notifyDesktop(title, body);
  if (!("Notification" in window) || Notification.permission !== "granted") return;
  const n = new Notification(title, { body, tag, requireInteraction: sticky });
  n.onclick = () => {
    window.focus();
    onClick();
  };
}

/** One-line summary for "My location". */
function summarizeMyLocation(on: boolean, { location, error }: ReturnType<typeof useMyLocation>, coveringCount: number): string {
  if (!on) return "off";
  if (error) return error;
  if (!location) return "locating…";
  return coveringCount > 0 ? `inside ${coveringCount} alert${coveringCount === 1 ? "" : "s"}` : "no alerts here";
}

/** One-line summary for "Settings". */
function summarizeSettings(cacheEntriesCount: number, cacheLimit: number): string {
  return `cache ${cacheEntriesCount}/${cacheLimit} scans`;
}

export default function App() {
  const [icao, setIcao] = useState(DEFAULT_SITE_ICAO);
  const site = findSite(icao) ?? RADAR_SITES[0];
  const nearbySites = useMemo(
    () =>
      RADAR_SITES.filter((s) => s.icao !== site.icao)
        .map((s) => ({ s, km: legLengthAndBearing(site, s).km }))
        .sort((a, b) => a.km - b.km)
        .slice(0, 8),
    [site],
  );

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
    renderedRangeKm,
    crossSection,
    setDealias,
    setHideNonWeather,
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
    // HRRR first: its simulated reflectivity is the forecast chasers read.
    forecast.selectProvider("hrrr");
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
  // Cross-section line: two map clicks (start, then end).
  const [xsLine, setXsLine] = useState<{ a: LatLonPoint; b: LatLonPoint | null } | null>(null);
  const [xsPicking, setXsPicking] = useState(false);

  const handleRainbowMapPointPick = useCallback(
    (lat: number, lon: number) => {
      if (xsPicking) {
        if (xsLine && !xsLine.b) {
          setXsLine({ a: xsLine.a, b: { lat, lon } });
          setXsPicking(false);
        } else {
          setXsLine({ a: { lat, lon }, b: null });
        }
        return;
      }
      if (rainbowPointPickTarget === "nowcast") rainbowNowcast.setPoint(lon, lat);
      else if (rainbowPointPickTarget === "weather") rainbowWeather.setPoint(lon, lat);
      setRainbowPointPickTarget(null);
    },
    [rainbowPointPickTarget, rainbowNowcast, rainbowWeather, xsPicking, xsLine],
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

  // `alertsSectionOpen` is controlled so selecting an alert -- from a map
  // click or the list itself -- can force the Warnings section open.
  // `forecastOpen` is the time strip's forecast drawer.
  const [alertsSectionOpen, setAlertsSectionOpen] = useState(false);
  const [forecastOpen, setForecastOpen] = useState(false);
  // Which side of NOW the map shows: live radar, or forecast radar once the user picks past NOW.
  const [timeRegion, setTimeRegion] = useState<"observed" | "forecast">("observed");
  const [forecastPlaying, setForecastPlaying] = useState(false);
  const [loopLoading, setLoopLoading] = useState(false);
  const [siteQuery, setSiteQuery] = useState("");
  // All alerts by default: zone-only watches can't be placed, so a near-site filter could hide one.
  const [nearAlertsOnly, setNearAlertsOnly] = useState(false);
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

  // Your own position, and a desktop notification (plus optional tone) when
  // an alert polygon starts covering it. Off by default: it asks for location.
  const [showMyLocation, setShowMyLocation] = useState(false);
  const [notifyCovering, setNotifyCovering] = useState(false);
  const [notifySound, setNotifySound] = useState(true);
  const myLocation = useMyLocation(showMyLocation);
  const [centerOnMeRequest, setCenterOnMeRequest] = useState(0);
  const coveringMe = useMemo(
    () => (myLocation.location ? coveringAlerts(myLocation.location, alerts.activeAlerts, new Set()).covering : []),
    [myLocation.location, alerts.activeAlerts],
  );
  const notifiedKeysRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    // Cleared while off, so turning it on reports alerts already covering you.
    if (!notifyCovering) {
      notifiedKeysRef.current = new Set();
      return;
    }
    if (!myLocation.location) return;
    const { covering, fresh } = coveringAlerts(myLocation.location, alerts.activeAlerts, notifiedKeysRef.current);
    notifiedKeysRef.current = new Set(covering.map(({ key }) => key));
    if (fresh.length === 0) return;
    if (notifySound) playAlertTone();
    for (const { key, alert } of fresh) {
      showNotification(
        `${alert.event} covers your location`,
        alert.headline ?? alert.affectedAreas.join("; "),
        key,
        alert.severity === "Extreme" || alert.severity === "Severe",
        () => setSelectedAlertKey(key),
      );
    }
  }, [myLocation.location, alerts.activeAlerts, notifyCovering, notifySound]);

  // Reveal the Warnings section whenever an alert becomes selected, so a
  // map click on a polygon always shows its detail.
  useEffect(() => {
    if (selectedAlertKey) {
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
  const [dealias, setDealiasState] = useState(false);
  const [hideNonWeather, setHideNonWeatherState] = useState(false);
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
    preferredSweepRef.current = null;
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
  // The tilt the user picked. A real-time volume arrives lowest tilt first, so a new
  // volume may not have it yet; the display falls back, then returns once it lands.
  const preferredSweepRef = useRef<number | null>(null);
  const changeSweep = useCallback(
    (newSweepIndex: number) => {
      preferredSweepRef.current = newSweepIndex;
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
  const currentRevision = currentEntry?.revision ?? null;
  useEffect(() => {
    if (status !== "ready" || !currentKey) return;
    const bytes = history.currentBytes();
    if (!bytes) return;

    const meta = decodeVolume(bytes);
    if (!meta) return;

    let newSweepIndex = preferredSweepRef.current ?? sweepIndexRef.current;
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
    // Intentionally keyed on (status, currentKey, currentRevision) only -- the
    // revision changes as a real-time volume grows -- sweepIndex/
    // momentCode are read via refs so changing them does not itself
    // re-decode; decodeVolume/defaultSweepIndexForMoment/
    // momentWireCodesForSweep are stable per-renderer-instance callbacks,
    // and renderSelection's own identity changes only with stormMotion
    // (irrelevant here -- whatever value is current when this effect
    // actually runs is the one used), so none of this needs to be listed
    // (this project's eslint config does not enable react-hooks/exhaustive-deps).
  }, [status, currentKey, currentRevision]);

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

  // Forecast side of NOW: the playback controls and ← → Space step and loop forecast hours.
  const inForecast = timeRegion === "forecast";
  // Only hours already published can be shown, stepped to, or looped.
  const forecastLeads = leadHourOptions(forecast.providerId).filter(
    (h) => forecast.publishedLeads === null || forecast.publishedLeads.includes(h),
  );
  const showRadar = () => {
    setTimeRegion("observed");
    setForecastPlaying(false);
  };
  // Only one loop at a time: the radar loop pauses whenever the forecast side takes over.
  const showForecast = () => {
    setTimeRegion("forecast");
    if (history.playMode === "playing") history.togglePlay();
  };
  const stepForecast = (delta: number) => {
    setForecastPlaying(false);
    const i = forecastLeads.indexOf(forecast.leadHours);
    forecast.setLeadHours(forecastLeads[Math.max(0, Math.min(forecastLeads.length - 1, i + delta))]);
  };
  const toggleForecastPlay = () => {
    if (forecastPlaying) {
      setForecastPlaying(false);
      return;
    }
    showForecast();
    setForecastPlaying(true);
    setLoopLoading(true);
    void forecast.loadLoop(forecastLeads).finally(() => setLoopLoading(false));
  };

  // Forecast loop: advance through the hours already rendered (more join as they load).
  useEffect(() => {
    const { cachedLeads, leadHours, setLeadHours } = forecast;
    if (!forecastPlaying || !inForecast || cachedLeads.length < 2) return;
    const next = cachedLeads.find((h) => h > leadHours) ?? cachedLeads[0];
    const timer = setTimeout(() => setLeadHours(next), history.frameMs);
    return () => clearTimeout(timer);
  }, [forecastPlaying, inForecast, forecast.cachedLeads, forecast.leadHours, forecast.setLeadHours, history.frameMs]);

  // Keyboard shortcuts -- see the module doc comment for the full list.
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (isEditableTarget(e.target)) return;
      // Tilt and Latest are radar-only; ← → Space follow whichever side of NOW is shown.
      if (["ArrowUp", "ArrowDown", "l", "L"].includes(e.key)) showRadar();
      switch (e.key) {
        case "ArrowLeft":
          e.preventDefault();
          if (inForecast) stepForecast(-1);
          else history.previous();
          break;
        case "ArrowRight":
          e.preventDefault();
          if (inForecast) stepForecast(1);
          else history.next();
          break;
        case " ":
          e.preventDefault();
          if (inForecast) toggleForecastPlay();
          else history.togglePlay();
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
  }, [history, stepElevation, rainbowPointPickTarget, inForecast, stepForecast, toggleForecastPlay, showRadar]);

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
  const nearAlerts = alerts.activeAlerts.filter(({ alert }) => alertNearSite(alert.geometry, site));
  const alertsSummary = summarizeAlerts(nearAlerts.length, alerts.activeAlerts.length, site.icao);
  const settingsSummary = summarizeSettings(history.entries.length, cacheLimit);

  const activeTableParsedUnits = useMemo(() => {
    if (!activeTableJson) return null;
    try {
      return (JSON.parse(activeTableJson) as { units?: string }).units ?? null;
    } catch {
      return null;
    }
  }, [activeTableJson]);

  const favoriteAvailable = momentOptions.filter((code) => favoriteMoments.includes(code));
  const productOrder = [...favoriteAvailable, ...momentOptions.filter((code) => !favoriteMoments.includes(code))];
  const feedError = status === "error" || history.pollEvent.type === "error" || decodeError !== null;
  const forecastSummary = [modelForecastSummary, pointForecastsSummary].join(" · ");
  // Only forecast *radar* replaces the radar on the map; other model fields stay in the panel.
  const forecastOnMap =
    timeRegion === "forecast" && forecast.variable === "composite_reflectivity" && forecast.phase === "ready";
  const forecastValidMs = forecast.run ? Date.parse(forecast.run.initTime) + forecast.leadHours * 3_600_000 : null;
  // Product and tilt come from the decoded volume, which needs the GPU pipeline.
  const segmentEmpty =
    status === "error" ? "Unavailable: GPU error above" : status === "ready" ? "Waiting for a scan…" : "Starting the GPU…";
  // Tilts in elevation order; repeated angles (split cuts) get their sweep number.
  const tilts = (volumeMeta?.elevationDegs ?? [])
    .map((deg, i) => ({ deg, i }))
    .sort((x, y) => x.deg - y.deg || x.i - y.i);
  const tiltLabel = (deg: number, i: number) =>
    tilts.filter((t) => t.deg.toFixed(1) === deg.toFixed(1)).length > 1 ? `${deg.toFixed(1)}·${i + 1}` : deg.toFixed(1);

  return (
    <div className="workstation">
      <aside className="rail" aria-label="Radar controls">
        <header className="rail-head">
          <span className="wordmark">RadarPro</span>
          <span
            className={`feed-status${feedError ? " feed-status-error" : ""}`}
            title={
              status === "ready"
                ? `GPU: ${adapterName || "(adapter name withheld)"} (${backend})`
                : status === "error"
                  ? `GPU error: ${error}`
                  : "GPU: initializing…"
            }
          >
            {describePollEvent(history.pollEvent)}
          </span>
        </header>

        {(status === "error" || decodeError) && (
          <div className="rail-error" role="status">
            {status === "error" && <p>GPU error: {error}</p>}
            {/* S10 Phase 3: a volume that downloaded but failed to decode
                keeps the last good frame on screen; this is its one visible home. */}
            {decodeError && (
              <p title="The most recently downloaded volume failed to decode -- likely a truncated/corrupted transfer. The display keeps showing the last good frame.">
                Scan failed to decode: {decodeError}. Showing the last good frame.
              </p>
            )}
          </div>
        )}

        <section className="rail-block" aria-labelledby="site-heading">
          <h2 id="site-heading" className="rail-heading">
            Site
          </h2>
          <div className="site-line">
            <span className="site-code">{site.icao}</span>
            <span className="site-name">{site.name}</span>
          </div>
          {/* Type a code or city; the list matches both and a full code switches at once. */}
          <input
            type="text"
            className="site-select"
            list="radar-sites"
            aria-label="Find radar site by code or city"
            placeholder="Type a code or city"
            autoComplete="off"
            spellCheck={false}
            value={siteQuery}
            onChange={(e) => {
              const match = RADAR_SITES.find((s) => s.icao === e.target.value.trim().toUpperCase());
              setSiteQuery(match ? "" : e.target.value);
              if (match) setIcao(match.icao);
            }}
          />
          {/* One tap to the next radar over: the eight nearest sites by distance. */}
          <div className="segment site-nearby" role="group" aria-label="Nearby radar sites">
            {nearbySites.map(({ s, km }) => (
              <button key={s.icao} type="button" onClick={() => setIcao(s.icao)} title={`${s.name} · ${km.toFixed(0)} km`}>
                {s.icao}
              </button>
            ))}
          </div>
          <datalist id="radar-sites">
            {RADAR_SITES.map((s) => (
              <option key={s.icao} value={s.icao}>
                {s.name}
              </option>
            ))}
          </datalist>
        </section>

        <section className="rail-block" aria-labelledby="product-heading">
          <div className="rail-heading-row">
            <h2 id="product-heading" className="rail-heading">
              Product
            </h2>
            {momentCode && (
              <button
                type="button"
                className="favorite-toggle"
                onClick={() => toggleFavoriteMoment(momentCode)}
                aria-pressed={favoriteMoments.includes(momentCode)}
                title={
                  favoriteMoments.includes(momentCode)
                    ? `Remove ${momentCode} from favorites`
                    : `Favorite ${momentCode} (favorites list first)`
                }
              >
                <Icon name="star" size={16} filled={favoriteMoments.includes(momentCode)} />
                <span className="visually-hidden">Favorite {momentCode}</span>
              </button>
            )}
          </div>
          <div className="segment" role="group" aria-labelledby="product-heading">
            {productOrder.length === 0 && <span className="segment-empty">{segmentEmpty}</span>}
            {productOrder.map((code) => (
              <button
                key={code}
                type="button"
                aria-pressed={code === momentCode}
                onClick={() => {
                  showRadar();
                  changeMoment(code);
                }}
                title={favoriteMoments.includes(code) ? `${code} (favorite)` : code}
              >
                {code}
                {favoriteMoments.includes(code) && <span className="segment-favorite" aria-hidden="true" />}
              </button>
            ))}
          </div>
        </section>

        <section className="rail-block" aria-labelledby="tilt-heading">
          <h2 id="tilt-heading" className="rail-heading">
            Tilt <span className="rail-heading-note">↑ ↓ keys</span>
          </h2>
          <div className="segment segment-tilts" role="group" aria-labelledby="tilt-heading">
            {!volumeMeta && <span className="segment-empty">{segmentEmpty}</span>}
            {volumeMeta && tilts.map(({ deg, i }) => (
              <button
                key={i}
                type="button"
                aria-pressed={i === sweepIndex}
                onClick={() => {
                  showRadar();
                  changeSweep(i);
                }}
                title={`Sweep ${i + 1} of ${volumeMeta.sweepCount}: ${deg.toFixed(2)}°`}
              >
                {tiltLabel(deg, i)}
              </button>
            ))}
          </div>
          <InfoPanel
            elevationDeg={elevationDeg}
            sweepIndex={sweepIndex}
            sweepCount={volumeMeta?.sweepCount ?? null}
            momentCode={momentCode}
            units={activeTableParsedUnits}
          />
        </section>

        <section className="rail-block" aria-labelledby="course-heading">
          <h2 id="course-heading" className="rail-heading">
            Course
          </h2>
          <ProbePanel cursor={cursor} probe={probe} />
          <CrossSectionPanel
            line={xsLine}
            picking={xsPicking}
            onStartPick={() => {
              setXsLine(null);
              setXsPicking(true);
            }}
            onClear={() => {
              setXsLine(null);
              setXsPicking(false);
            }}
            renderKey={`${currentEntry?.key ?? ""}|${momentCode ?? ""}|${activeTableJson ?? ""}|${dealias}|${hideNonWeather}`}
            render={(a, b, cols, rows, topKm) =>
              // SRV is derived per tilt for display and isn't in the decoded volume; slice (raw or dealiased) velocity.
              momentCode ? crossSection(momentCode === "SRV" ? "VEL" : momentCode, a, b, cols, rows, topKm) : null
            }
          />
          {momentCode && <Legend activeColorTableJson={activeTableJson} momentCode={momentCode} />}
        </section>

        {/* Warnings default open the first time active alerts arrive, and
            reopen whenever one is selected (map click or list). */}
        <SidebarCategory title="Warnings" summary={alertsSummary} open={alertsSectionOpen} onToggle={setAlertsSectionOpen}>
          <label className="check-field">
            <input type="checkbox" checked={nearAlertsOnly} onChange={(e) => setNearAlertsOnly(e.target.checked)} />
            <span>Only polygon warnings within 460 km (hides zone-only watches)</span>
          </label>
          <AlertsPanel
            status={alerts.status}
            error={alerts.error}
            alerts={nearAlertsOnly ? nearAlerts : alerts.activeAlerts}
            emptyText={nearAlertsOnly ? "no polygon warnings within 460 km; zone-only watches are not placed" : undefined}
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

        <SidebarCategory title="My location" summary={summarizeMyLocation(showMyLocation, myLocation, coveringMe.length)}>
          <label className="check-field">
            <input type="checkbox" checked={showMyLocation} onChange={(e) => setShowMyLocation(e.target.checked)} />
            <span>Show my location on the map</span>
          </label>
          <button type="button" disabled={!myLocation.location} onClick={() => setCenterOnMeRequest((n) => n + 1)}>
            Center on me
          </button>
          <label className="check-field">
            <input
              type="checkbox"
              checked={notifyCovering}
              disabled={!showMyLocation}
              onChange={async (e) => {
                const on = e.target.checked;
                // Ask first, so alerts already covering you are notified, not just sounded.
                if (on && !isDesktop() && "Notification" in window && Notification.permission === "default") {
                  await Notification.requestPermission();
                }
                setNotifyCovering(on);
              }}
            />
            <span>Notify me when a warning covers my location</span>
          </label>
          <label className="check-field">
            <input type="checkbox" checked={notifySound} disabled={!notifyCovering} onChange={(e) => setNotifySound(e.target.checked)} />
            <span>Play a sound with the notification</span>
          </label>
          {notifyCovering && (
            <button
              type="button"
              onClick={() => {
                if (notifySound) playAlertTone();
                showNotification("RadarPro test notification", "Warnings that cover your location will look like this.", "radarpro-test", false, () => {});
              }}
            >
              Send a test notification
            </button>
          )}
          {notifyCovering && !isDesktop() && (!("Notification" in window) || Notification.permission === "denied") && (
            <div className="settings-field-note">Desktop notifications are blocked; only the sound will play.</div>
          )}
          {coveringMe.length > 0 && (
            <ul className="alerts-list">
              {coveringMe.map(({ key, alert }) => (
                <li key={key}>
                  <button
                    type="button"
                    className={`alert-row${key === selectedAlertKey ? " alert-row-selected" : ""}`}
                    onClick={() => handleAlertClick(key)}
                  >
                    <span className="alert-severity-dot" style={{ background: SEVERITY_COLORS[alert.severity] }} aria-hidden="true" />
                    <span className="alert-row-text">
                      <span className="alert-row-event">{alert.event}</span>
                      {alert.headline && <span className="alert-row-headline">{alert.headline}</span>}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </SidebarCategory>

        {/* One active map layer at a time: exclusivity is the control itself,
            and only the selected layer's own settings show. */}
        <SidebarCategory title="Layers" summary={mapLayersSummary}>
          <label className="map-layer-select">
            <span>Active map layer</span>
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
          {/* Only visible while the live radar is the active layer. */}
          <label className="range-field" title="Live radar opacity -- has no visible effect while Rainbow or MRMS is active">
            <span>Radar opacity</span>
            <input
              type="range"
              min={0}
              max={100}
              value={radarOpacityPercent}
              onChange={(e) => setRadarOpacityPercent(Number(e.target.value))}
            />
            <span className="num">{radarOpacityPercent}%</span>
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

        {momentCode === "REF" && (
          <SidebarCategory title="Reflectivity" summary={hideNonWeather ? "weather only" : "all echo"} defaultOpen>
            <label
              className="check-field"
              title="Hide weak echo that dual-pol marks as insects, birds or ground clutter (low correlation coefficient, or insect-like ZDR under 30 dBZ). Echo of 35 dBZ and up is always shown, so hail and tornado debris stay visible."
            >
              <input
                type="checkbox"
                checked={hideNonWeather}
                onChange={(e) => {
                  setHideNonWeather(e.target.checked);
                  setHideNonWeatherState(e.target.checked);
                  if (sweepIndex !== null && momentCode) renderSelection(sweepIndex, momentCode);
                }}
              />
              <span>Hide non-weather echo</span>
            </label>
          </SidebarCategory>
        )}

        {(momentCode === "VEL" || momentCode === "SRV") && (
          <SidebarCategory title="Velocity" summary={dealias ? "dealiased" : "raw"} defaultOpen>
            <label
              className="check-field"
              title="Unfold velocities that wrapped past the radar's Nyquist limit. Corrected values can be wrong where echo is noisy or disconnected."
            >
              <input
                type="checkbox"
                checked={dealias}
                onChange={(e) => {
                  setDealias(e.target.checked);
                  setDealiasState(e.target.checked);
                  if (sweepIndex !== null && momentCode) renderSelection(sweepIndex, momentCode);
                }}
              />
              <span>Dealias velocity</span>
            </label>
          </SidebarCategory>
        )}

        {/* Storm motion only means something while SRV is the product. */}
        {momentCode === "SRV" && (
          <SidebarCategory
            title="Storm motion"
            summary={`${stormMotion.speedMps} m/s toward ${stormMotion.directionDeg}°`}
            defaultOpen
          >
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
                <span>Moving toward (° true)</span>
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
          </SidebarCategory>
        )}

        <SidebarCategory title="Archive" summary={history.archiveStartMillis !== null ? "viewing archive" : "live"}>
          <ArchiveLoader
            archiveStartMillis={history.archiveStartMillis}
            status={history.archiveStatus}
            onLoad={history.loadArchive}
            onReturnToLive={history.returnToLive}
          />
        </SidebarCategory>

        {momentCode && (
          <SidebarCategory title="Color table" summary={momentCode}>
            <ColorTableEditor
              momentCode={momentCode}
              activeColorTableJson={activeTableJson}
              onApply={handleApplyColorTable}
              onResetToBuiltinDefault={handleResetColorTable}
            />
          </SidebarCategory>
        )}

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
      </aside>

      <main className="map-region">
        <MapView
          site={site}
          canvasRef={canvasRef}
          canvasSize={RADAR_CANVAS_SIZE}
          sweepRangeKm={renderedRangeKm}
          onSiteClick={setIcao}
          rangeRings={rangeRingsGeo}
          onCursorMove={handleCursorMove}
          onCursorLeave={handleCursorLeave}
          alerts={alerts.geojson}
          selectedAlertKey={selectedAlertKey}
          onAlertClick={handleAlertClick}
          pointPickActive={rainbowPointPickTarget !== null || xsPicking}
          onPointPick={handleRainbowMapPointPick}
          rainbowTileUrlTemplate={rainbow.tileUrlTemplate}
          rainbowEnabled={rainbow.enabled}
          rainbowMaxZoom={rainbow.layerInfo.maxZoom}
          mrmsCanvasRef={mrmsCanvasRef}
          mrmsEnabled={mrmsEnabled}
          onMrmsViewportChange={setMrmsViewport}
          radarOpacity={radarOpacityPercent / 100}
          course={xsLine}
          myLocation={myLocation.location}
          centerOnMeRequest={centerOnMeRequest}
          forecastCanvasRef={forecastCanvasRef}
          forecastOnMap={forecastOnMap}
          forecastKey={`${forecast.providerId}|${forecast.variable}|${forecast.leadHours}|${forecast.phase}`}
        />
        {forecastOnMap && forecastValidMs !== null && (
          <div className="map-forecast-badge" role="status">
            <strong>Forecast radar</strong> HRRR simulated reflectivity · valid{" "}
            {new Date(forecastValidMs).toLocaleString(undefined, { weekday: "short", hour: "numeric", minute: "2-digit", hour12: true })} (+
            {forecast.leadHours}h) · model, not observed
            {loopLoading && <span className="map-forecast-loading">loading hours {forecast.cachedLeads.length}/{forecastLeads.length}…</span>}
            <button type="button" onClick={showRadar}>
              Back to radar
            </button>
          </div>
        )}

        <RightPanel sections={rightPanelSections} />

        {/* Forecasts live with time, never in the rail: model fields and
            Rainbow point forecasts open from the time strip. Hidden rather
            than unmounted: the forecast canvas is painted once per render. */}
        <section className="forecast-drawer" aria-labelledby="forecast-heading" hidden={!forecastOpen}>
          <header className="forecast-drawer-head">
            <h2 id="forecast-heading">Forecast</h2>
            <button
              type="button"
              className="icon-button"
              aria-label="Close forecast"
              onClick={() => setForecastOpen(false)}
            >
              <Icon name="close" size={16} />
            </button>
          </header>
          <div className="forecast-drawer-body">
            <div className="forecast-drawer-model">
              <ForecastPanel canvasRef={forecastCanvasRef} forecast={forecast} />
            </div>
            <div className="forecast-drawer-points">
              <SidebarSection title="Rainbow nowcast (point)" defaultOpen>
                <RainbowNowcastPanel
                  nowcast={rainbowNowcast}
                  pickModeActive={rainbowPointPickTarget === "nowcast"}
                  onTogglePickMode={handleToggleNowcastPick}
                />
              </SidebarSection>
              <SidebarSection title="Rainbow outlook (point)">
                <RainbowWeatherPanel
                  weather={rainbowWeather}
                  pickModeActive={rainbowPointPickTarget === "weather"}
                  onTogglePickMode={handleToggleWeatherPick}
                />
              </SidebarSection>
            </div>
          </div>
        </section>
      </main>

      <footer className="time-strip" title="← / → previous/next scan or forecast hour · Space play/pause · ↑ / ↓ tilt · L latest">
        {/* Past NOW the same controls step and loop forecast hours instead of scans. */}
        <PlaybackControls
          playMode={inForecast ? (forecastPlaying ? "playing" : "paused") : history.playMode}
          currentIndex={inForecast ? forecastLeads.indexOf(forecast.leadHours) : history.currentIndex}
          entriesCount={inForecast ? forecastLeads.length : history.entries.length}
          currentTimeMillis={inForecast ? forecastValidMs : (currentEntry?.startTimeMillis ?? null)}
          frameMs={history.frameMs}
          onPrevious={() => (inForecast ? stepForecast(-1) : history.previous())}
          onNext={() => (inForecast ? stepForecast(1) : history.next())}
          onTogglePlay={() => (inForecast ? toggleForecastPlay() : history.togglePlay())}
          onJumpLatest={() => {
            showRadar();
            history.jumpToLatest();
          }}
          onFrameMsChange={history.setFrameMs}
          isLiveStale={!inForecast && history.isLiveStale}
          isArchive={history.archiveStartMillis !== null}
          currentVolumeAgeMillis={inForecast ? null : history.currentVolumeAgeMillis}
        />
        {/* S09: one real-time axis, observed scans up to NOW and forecast
            valid times beyond it; drives the same history/forecast state. */}
        <UnifiedTimeline
          history={history}
          forecast={forecast}
          onRegionChange={(region) => {
            if (region === "observed") {
              showRadar();
            } else {
              showForecast();
              setForecastPlaying(false);
            }
          }}
        />
        <button
          type="button"
          className="forecast-toggle"
          aria-expanded={forecastOpen}
          onClick={() => setForecastOpen((v) => !v)}
          title={forecastSummary}
        >
          <Icon name="forecast" />
          <span>
            Forecast
            <small>{modelForecastSummary}</small>
          </span>
        </button>
      </footer>
    </div>
  );
}
