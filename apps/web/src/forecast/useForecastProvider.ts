import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { ensureForecastWasmModuleLoaded, ProviderHandle } from "./wasmModule";
import type {
  ForecastEnsembleSpec,
  ForecastGridMetadata,
  ForecastModelMetadata,
  ForecastProviderId,
  ForecastRunMetadata,
} from "./types";

/** Fixed backing-buffer size for the forecast render canvas -- see
 * `ForecastPanel`'s doc comment for why this is a dedicated panel, not a
 * map overlay, in this stage. */
// ~0.04° per pixel over the fixed extent below: storm-scale for HRRR's 3 km grid.
export const FORECAST_RENDER_WIDTH = 1440;
export const FORECAST_RENDER_HEIGHT = 960;

/**
 * Fixed orthographic-camera framing (decimal degrees) handed to
 * `renderCurrentGrid` -- a round, documented approximation of the
 * continental US (both GEFS's global grid and HRRR's CONUS grid cover this
 * comfortably), not derived from the live `MapView`'s current viewport.
 * Exact geographic alignment with the map is out of scope for this stage --
 * see `ForecastPanel`'s doc comment (mirrors the same "reasonable,
 * documented approximation" choice `MapView`'s own `APPROX_SWEEP_RADIUS_KM`
 * already makes for the radar sweep overlay).
 */
export const FORECAST_CENTER_LON = -97;
export const FORECAST_CENTER_LAT = 38;
export const FORECAST_HALF_EXTENT_LON = 30;
export const FORECAST_HALF_EXTENT_LAT = 20;

/** Lookback window for `discoverLatestRun` -- generous enough to survive a
 * provider's typical publish latency without a UI-configurable knob (a
 * proof-of-concept concern, per this stage's brief). */
const DISCOVER_LOOKBACK_DAYS = 5;

/** Forecast variables (canonical names, see `forecast-core`'s
 * `ForecastVariable::canonical_name`). `models`: which providers publish it
 * (absent: GEFS and HRRR). `onMap`: drawn on the map in place of the radar
 * (clear where it is low); GEFS's coarse global grid stays in the panel.
 * `opaque`: has no clear areas, so it is drawn see-through to keep the map's labels readable. */
export const FORECAST_VARIABLES = [
  { id: "composite_reflectivity", label: "Simulated reflectivity", models: ["hrrr"], onMap: true },
  { id: "stp", label: "Significant tornado parameter (STP)", models: ["hrrr"], onMap: true },
  { id: "sbcape", label: "CAPE, surface-based", models: ["hrrr"], onMap: true },
  { id: "mlcape", label: "CAPE, mixed-layer", models: ["hrrr"], onMap: true },
  { id: "srh_0_1km", label: "Helicity 0–1 km", models: ["hrrr"], onMap: true },
  { id: "srh_0_3km", label: "Helicity 0–3 km", models: ["hrrr"], onMap: true },
  { id: "bulk_shear_0_6km", label: "Bulk shear 0–6 km", models: ["hrrr"], onMap: true },
  { id: "lcl_height", label: "LCL height (cloud base)", models: ["hrrr"], onMap: true },
  { id: "temperature_2m", label: "Temperature (2 m)", models: ["gefs", "hrrr", "rtma"], onMap: true, opaque: true },
  { id: "dewpoint_2m", label: "Dewpoint (2 m)", models: ["gefs", "hrrr", "rtma"], onMap: true, opaque: true },
  { id: "wind_speed_10m", label: "Wind speed (10 m)", models: ["hrrr", "rtma"], onMap: true },
  { id: "wind_gust", label: "Wind gust", models: ["gefs", "hrrr", "rtma"], onMap: true },
  { id: "relative_humidity", label: "Relative humidity" },
  { id: "wind_u_10m", label: "Wind, east–west (10 m)", models: ["gefs", "hrrr", "rtma"] },
  { id: "wind_v_10m", label: "Wind, north–south (10 m)", models: ["gefs", "hrrr", "rtma"] },
  { id: "mslp", label: "Sea-level pressure" },
  { id: "cloud_cover", label: "Cloud cover", models: ["gefs", "hrrr", "rtma"] },
] as const;

/** Whether `providerId` publishes variable `id`. */
export function variableAvailable(id: string, providerId: ForecastProviderId | null): boolean {
  const v = FORECAST_VARIABLES.find((x) => x.id === id);
  return !!v && providerId !== null && ("models" in v ? (v.models as readonly string[]).includes(providerId) : providerId !== "rtma");
}

/** Whether `id` is drawn on the map for `providerId` (never GEFS: its 0.5° global grid is too coarse). */
export function variableOnMap(id: string, providerId: ForecastProviderId | null): boolean {
  const v = FORECAST_VARIABLES.find((x) => x.id === id);
  return !!v && "onMap" in v && providerId !== "gefs";
}

const DEFAULT_VARIABLE = "composite_reflectivity";

const GEFS_LEAD_HOUR_OPTIONS = [0, 6, 12, 24, 48];
// Every HRRR run reaches 18 h (only the 00/06/12/18Z runs go to 48 h), hourly.
const HRRR_LEAD_HOUR_OPTIONS = Array.from({ length: 19 }, (_, h) => h);

/** Selectable forecast lead times (hours) for a provider. */
export function leadHourOptions(providerId: ForecastProviderId | null): number[] {
  // RTMA is an analysis: one valid time, no forecast hours.
  if (providerId === "rtma") return [0];
  return providerId === "hrrr" ? HRRR_LEAD_HOUR_OPTIONS : GEFS_LEAD_HOUR_OPTIONS;
}

const DEFAULT_LEAD_HOURS = 6;
const PUBLISHED_RECHECK_MS = 120_000;

/**
 * Per-call timeout (ms) for every `ProviderHandle` wasm call in this
 * pipeline -- 25s comfortably covers a real discovery walk (up to
 * `4 * (DISCOVER_LOOKBACK_DAYS + 1)` sequential `ListObjectsV2` requests
 * against S3) plus one field fetch over a slow connection.
 *
 * # Why this exists, and its one known limitation
 *
 * `provider-gefs`/`provider-hrrr`'s own `client.rs` already documents a
 * real gap: "`ClientBuilder::timeout` does not exist on reqwest's wasm32
 * (browser `fetch`) backend ... a hung fetch is left to the browser's own
 * defaults on that target." Global Contract ("no silent/uncontrolled
 * failures on untrusted network data") requires this not be allowed to
 * hang the UI forever, and the only layer that *can* bound a stuck
 * `fetch()` is this JS-side `Promise.race` -- `forecast-web`'s wasm API has
 * no cancellation primitive of its own to reach into instead.
 *
 * This genuinely rescues a call stuck *awaiting* something (a pending
 * `fetch()`, a `discoverLatestRun` that never dispatches a request at
 * all -- both observed during this stage's own browser verification). It
 * cannot rescue a call that blocks the JS main thread *synchronously* --
 * `setTimeout`'s callback cannot fire until the event loop is free, so a
 * long-running synchronous Rust computation (or a GPU call that never
 * yields back to the executor) starves this same timer along with
 * everything else on the page. This stage's own verification hit exactly
 * that on `renderCurrentGrid`: both GEFS and HRRR reliably progressed all
 * the way through `discoverLatestRun`/`fetchField` (real S3 data, correct
 * metadata) and then hung *inside* `renderCurrentGrid` long enough to wedge
 * the whole tab (unresponsive to any script injection, this timeout
 * included) -- strongly suggesting a `forecast_core::gpu::
 * render_forecast_grid` GPU-adapter/device request that never resolves,
 * quite possibly because `radar-web`'s own renderer already holds a
 * WebGPU device on the same page (untested: whether a second, independent
 * `wgpu::Instance::request_adapter()` from a second wasm module can
 * coexist with that in this browser/environment). This timeout still
 * belongs here (it correctly bounds the failure modes it *can* bound, and
 * costs nothing when a call completes normally) but a future stage must
 * root-cause the render-side hang directly in `forecast-core`/
 * `radar-render`'s GPU acquisition path -- seeing this doc comment without
 * also fixing that would be treating a symptom, not the cause.
 */
const CALL_TIMEOUT_MS = 25_000;

/** Race `promise` against a timeout, rejecting with a descriptive message
 * (never leaving the caller waiting forever) if it does not settle within
 * `CALL_TIMEOUT_MS` -- see that constant's doc comment. */
function withTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`${label} timed out after ${CALL_TIMEOUT_MS / 1000}s (no response)`));
    }, CALL_TIMEOUT_MS);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (err: unknown) => {
        clearTimeout(timer);
        reject(err);
      },
    );
  });
}

export type ForecastPhase =
  | "idle"
  | "loading-module"
  | "discovering"
  | "fetching"
  | "rendering"
  | "ready"
  | "error";

export interface ForecastProviderState {
  providerId: ForecastProviderId | null;
  phase: ForecastPhase;
  /** Non-null only when `phase === "error"` -- the plain error string
   * `ProviderHandle`'s rejected promise carried (Global Contract: never an
   * uncontrolled/unhandled rejection reaching the console silently). */
  error: string | null;
  metadata: ForecastModelMetadata | null;
  run: ForecastRunMetadata | null;
  grid: ForecastGridMetadata | null;
  leadHours: number;
  /** Canonical variable name, one of `FORECAST_VARIABLES`' ids. */
  variable: string;
  /** Only meaningful (and only ever non-null) when `metadata.isEnsemble`. */
  ensemble: ForecastEnsembleSpec | null;
  /** Lead hours already rendered for the current variable/ensemble, ascending -- the forecast loop's frames. */
  cachedLeads: number[];
  /** Lead hours of the run published so far, or `null` when the provider can't tell (treat all as available). */
  publishedLeads: number[] | null;
}

const frameKey = (variable: string, ensemble: ForecastEnsembleSpec | null, leadHours: number) =>
  `${variable}|${JSON.stringify(ensemble)}|${leadHours}`;

function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * Owns one `forecast-web` `ProviderHandle` at a time (see
 * `crates/forecast-web/src/wasm_api.rs`) and drives the
 * discover-latest-run -> fetch-field -> render-to-canvas pipeline for
 * whichever provider (`"gefs"` | `"hrrr"`) is currently selected.
 *
 * # Switching providers
 *
 * There is no "switch provider" method on the Rust side -- `selectProvider`
 * frees the previous handle (if any) and constructs a brand new
 * `ProviderHandle`, then re-runs the whole discover/fetch/render pipeline
 * from scratch. Each handle owns its own provider state entirely (see
 * `wasm_api.rs`'s "Provider independence" doc comment), so a GEFS failure
 * can never affect a subsequent HRRR attempt or vice versa -- this hook
 * adds a generation counter on top purely to discard a superseded in-flight
 * pipeline's result (e.g. the user switches away before a slow network call
 * resolves), never to paper over an actual provider-level failure.
 *
 * # No uncontrolled failures
 *
 * Every wasm call in the pipeline is wrapped in try/catch; a thrown/
 * rejected error always becomes this hook's `error`/`phase: "error"` state,
 * never an unhandled rejection or an app crash (Global Contract). A stale
 * in-flight pipeline (superseded by a newer `selectProvider`/
 * `setLeadHours`/`setEnsemble` call before it resolved) is detected via
 * that generation counter and silently discarded -- it never overwrites
 * state for the *current* selection, and never touches the canvas.
 *
 * # Large binary output kept out of React state
 *
 * `renderCurrentGrid`'s `Uint8Array` (`FORECAST_RENDER_WIDTH *
 * FORECAST_RENDER_HEIGHT * 4` bytes) is painted straight onto the caller's
 * `<canvas>` via `putImageData` and never stored in React state -- only the
 * small derived metadata objects above become state (Global Contract:
 * large binary arrays do not live in React state / churn re-renders).
 */
export function useForecastProvider(canvasRef: RefObject<HTMLCanvasElement>) {
  const [state, setState] = useState<ForecastProviderState>({
    providerId: null,
    phase: "idle",
    error: null,
    metadata: null,
    run: null,
    grid: null,
    leadHours: DEFAULT_LEAD_HOURS,
    variable: DEFAULT_VARIABLE,
    ensemble: null,
    cachedLeads: [],
    publishedLeads: null,
  });

  const handleRef = useRef<ProviderHandle | null>(null);
  // Read inside selectProvider's async pipeline without making it a dependency.
  const variableRef = useRef(state.variable);
  variableRef.current = state.variable;
  const generationRef = useRef(0);
  // Rendered frames for the current run, so revisiting an hour (and the loop) never refetches.
  // Cleared on provider/variable/ensemble change: 19 HRRR frames are ~105 MB.
  const framesRef = useRef(new Map<string, ImageData>());
  // The handle holds one "current grid", so fetch+render pairs must never interleave.
  const queueRef = useRef<Promise<unknown>>(Promise.resolve());
  const cachedLeadsFor = (variable: string, ensemble: ForecastEnsembleSpec | null) =>
    [...framesRef.current.keys()]
      .filter((k) => k.startsWith(`${variable}|${JSON.stringify(ensemble)}|`))
      .map((k) => Number(k.slice(k.lastIndexOf("|") + 1)))
      .sort((a, b) => a - b);

  const freeHandle = useCallback(() => {
    try {
      handleRef.current?.free();
    } catch {
      // Nothing a caller could do about a failed free() of wasm-bindgen
      // memory -- never let this become an uncontrolled throw.
    }
    handleRef.current = null;
  }, []);

  useEffect(() => {
    return () => {
      generationRef.current++; // discard any pipeline still in flight
      freeHandle();
    };
  }, [freeHandle]);

  /** Paint a rendered frame onto the owned canvas. No-ops if the canvas is
   * not mounted (e.g. the panel unmounted mid-render). */
  const paint = useCallback(
    (imageData: ImageData) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      canvas.width = FORECAST_RENDER_WIDTH;
      canvas.height = FORECAST_RENDER_HEIGHT;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.putImageData(imageData, 0, 0);
    },
    [canvasRef],
  );

  /** Fetch `variable`/`leadHours`/`ensemble` on `handle` and
   * render it, updating state at each step. Assumes a run has already been
   * discovered on this handle. Silently discards its own result (never
   * calls `setState`/`paint`) once `generation` is no longer the current
   * one -- see this hook's own doc comment. */
  /** Fetch and render one frame on `handle`, queued so it never interleaves
   * with another fetch/render on the same handle. Caches the frame. */
  const renderFrame = useCallback(
    (handle: ProviderHandle, variable: string, leadHours: number, ensemble: ForecastEnsembleSpec | null) => {
      const job = queueRef.current.then(async () => {
        const gridJson = (await withTimeout(
          handle.fetchField(variable, leadHours, ensemble),
          "fetchField",
        )) as string;
        const pixels = (await withTimeout(
          handle.renderCurrentGrid(
            FORECAST_CENTER_LON,
            FORECAST_CENTER_LAT,
            FORECAST_HALF_EXTENT_LON,
            FORECAST_HALF_EXTENT_LAT,
            FORECAST_RENDER_WIDTH,
            FORECAST_RENDER_HEIGHT,
          ),
          "renderCurrentGrid",
        )) as Uint8Array;
        // Copy: `Uint8Array.buffer` may be a SharedArrayBuffer, which ImageData rejects.
        const imageData = new ImageData(new Uint8ClampedArray(pixels), FORECAST_RENDER_WIDTH, FORECAST_RENDER_HEIGHT);
        framesRef.current.set(frameKey(variable, ensemble, leadHours), imageData);
        return { grid: JSON.parse(gridJson) as ForecastGridMetadata, imageData };
      });
      queueRef.current = job.catch(() => undefined);
      return job;
    },
    [],
  );

  /** Show `variable`/`leadHours`/`ensemble`: from the frame cache when
   * already rendered, else fetched and rendered on `handle`. Assumes a run
   * has already been discovered on this handle. Silently discards its own
   * result (never calls `setState`/`paint`) once `generation` is no longer
   * the current one -- see this hook's own doc comment. */
  const fetchAndRender = useCallback(
    async (
      handle: ProviderHandle,
      variable: string,
      leadHours: number,
      ensemble: ForecastEnsembleSpec | null,
      generation: number,
    ) => {
      const cached = framesRef.current.get(frameKey(variable, ensemble, leadHours));
      if (cached) {
        paint(cached);
        setState((s) => (generation === generationRef.current ? { ...s, phase: "ready", error: null } : s));
        return;
      }
      try {
        setState((s) => (generation === generationRef.current ? { ...s, phase: "fetching", error: null } : s));
        const { grid, imageData } = await renderFrame(handle, variable, leadHours, ensemble);
        if (generation !== generationRef.current) return;
        paint(imageData);
        setState((s) =>
          generation === generationRef.current
            ? { ...s, grid, phase: "ready", cachedLeads: cachedLeadsFor(variable, ensemble) }
            : s,
        );
      } catch (err) {
        if (generation !== generationRef.current) return;
        setState((s) => ({ ...s, phase: "error", error: errMessage(err) }));
      }
    },
    [paint, renderFrame],
  );

  /** Render every not-yet-cached hour in `leads`, in order, in the
   * background, for the forecast loop. Stops at the first hour that fails
   * (a run still being published has no later hours yet). */
  const loadLoop = useCallback(
    async (leads: number[]) => {
      const handle = handleRef.current;
      if (!handle || state.run === null) return;
      const { variable, ensemble } = state;
      const generation = generationRef.current;
      for (const lead of leads) {
        if (framesRef.current.has(frameKey(variable, ensemble, lead))) continue;
        try {
          await renderFrame(handle, variable, lead, ensemble);
        } catch (err) {
          console.warn(`[forecast] loop stopped at +${lead}h: ${errMessage(err)}`);
          return;
        }
        if (generation !== generationRef.current) return;
        setState((s) => ({ ...s, cachedLeads: cachedLeadsFor(variable, ensemble) }));
      }
    },
    [renderFrame, state],
  );

  /** Select (or re-select) a provider: frees any previous handle,
   * constructs a fresh one, and runs discover -> fetch -> render from
   * scratch. Resets `leadHours`/`ensemble` to this hook's defaults (`mean`
   * for an ensemble provider, `null` otherwise) -- a prior provider's
   * ensemble selection has no meaning for a different provider. */
  const selectProvider = useCallback(
    (providerId: ForecastProviderId) => {
      const generation = ++generationRef.current;
      freeHandle();
      framesRef.current.clear();
      // Discovery only guarantees +0h is out for HRRR's newest run; later hours publish over the next hour.
      const firstLead = providerId === "gefs" ? DEFAULT_LEAD_HOURS : 0;
      setState((s) => ({
        providerId,
        phase: "loading-module",
        error: null,
        metadata: null,
        run: null,
        grid: null,
        leadHours: firstLead,
        // E.g. GEFS and RTMA have no simulated reflectivity; fall back to temperature there.
        variable: variableAvailable(s.variable, providerId) ? s.variable : "temperature_2m",
        ensemble: null,
        cachedLeads: [],
        publishedLeads: null,
      }));

      void (async () => {
        try {
          await ensureForecastWasmModuleLoaded();
          if (generation !== generationRef.current) return;

          const handle = new ProviderHandle(providerId);
          handleRef.current = handle;
          const metadata = JSON.parse(handle.metadata()) as ForecastModelMetadata;
          const ensemble: ForecastEnsembleSpec | null = metadata.isEnsemble ? "mean" : null;
          setState((s) =>
            generation === generationRef.current ? { ...s, metadata, ensemble, phase: "discovering" } : s,
          );

          const runJson = (await withTimeout(
            handle.discoverLatestRun(DISCOVER_LOOKBACK_DAYS),
            "discoverLatestRun",
          )) as string;
          if (generation !== generationRef.current) return;
          const run = JSON.parse(runJson) as ForecastRunMetadata;
          setState((s) => (generation === generationRef.current ? { ...s, run } : s));

          await fetchAndRender(handle, variableRef.current, firstLead, ensemble, generation);
        } catch (err) {
          if (generation !== generationRef.current) return;
          setState((s) => ({ ...s, phase: "error", error: errMessage(err) }));
        }
      })();
    },
    [fetchAndRender, freeHandle],
  );

  /** Re-fetch and re-render at a new forecast lead time, reusing the
   * currently-discovered run (no re-discovery). Just records the new value
   * if no run has been discovered yet on the current handle. */
  const setLeadHours = useCallback(
    (leadHours: number) => {
      const handle = handleRef.current;
      if (!handle || state.run === null) {
        setState((s) => ({ ...s, leadHours }));
        return;
      }
      const generation = generationRef.current;
      setState((s) => ({ ...s, leadHours }));
      void fetchAndRender(handle, state.variable, leadHours, state.ensemble, generation);
    },
    [fetchAndRender, state.ensemble, state.run, state.variable],
  );

  /** Re-fetch and re-render with a different ensemble statistic (GEFS
   * only) -- see `metadata.isEnsemble`. No-op for a deterministic provider
   * or before a run has been discovered. */
  const setEnsemble = useCallback(
    (ensemble: ForecastEnsembleSpec) => {
      const handle = handleRef.current;
      if (!handle || state.run === null || !state.metadata?.isEnsemble) return;
      const generation = ++generationRef.current;
      framesRef.current.clear();
      setState((s) => ({ ...s, ensemble, cachedLeads: [] }));
      void fetchAndRender(handle, state.variable, state.leadHours, ensemble, generation);
    },
    [fetchAndRender, state.leadHours, state.metadata, state.run, state.variable],
  );

  /** Re-fetch and re-render a different variable on the current run. Just
   * records the choice if no run has been discovered yet. */
  const setVariable = useCallback(
    (variable: string) => {
      const handle = handleRef.current;
      const generation = ++generationRef.current;
      framesRef.current.clear();
      setState((s) => ({ ...s, variable, cachedLeads: [] }));
      if (!handle || state.run === null) return;
      void fetchAndRender(handle, variable, state.leadHours, state.ensemble, generation);
    },
    [fetchAndRender, state.ensemble, state.leadHours, state.run],
  );

  // A run publishes hour by hour (HRRR: over about an hour), so re-check what's out.
  useEffect(() => {
    const handle = handleRef.current;
    if (!handle || state.run === null) return;
    const generation = generationRef.current;
    const initTime = state.run.initTime;
    // RTMA is "now": pick up each new hourly analysis in place, without dropping back to radar.
    const refreshAnalysis = async () => {
      const job = queueRef.current.then(() => withTimeout(handle.discoverLatestRun(DISCOVER_LOOKBACK_DAYS), "discoverLatestRun"));
      queueRef.current = job.catch(() => undefined);
      const runJson = (await job) as string;
      const run = JSON.parse(runJson) as ForecastRunMetadata;
      if (generation !== generationRef.current || run.initTime === initTime) return;
      framesRef.current.clear();
      const { grid, imageData } = await renderFrame(handle, variableRef.current, 0, null);
      if (generation !== generationRef.current) return;
      paint(imageData);
      setState((s) => ({ ...s, run, grid, cachedLeads: cachedLeadsFor(variableRef.current, null) }));
    };
    const check = () =>
      (state.providerId === "rtma"
        ? refreshAnalysis()
        : queueRef.current
            .then(() => handle.publishedLeadHours() as Promise<string>)
            .then((json) => {
              if (generation === generationRef.current) {
                setState((s) => ({ ...s, publishedLeads: JSON.parse(json) as number[] | null }));
              }
            })
      ).catch((err) => console.warn(`[forecast] recheck failed: ${errMessage(err)}`));
    if (state.providerId !== "rtma") void check();
    const timer = setInterval(check, PUBLISHED_RECHECK_MS);
    return () => clearInterval(timer);
  }, [state.run, state.providerId, paint, renderFrame]);

  /** Retry the current provider's pipeline from scratch after an error
   * (e.g. a transient discovery/network failure) -- equivalent to
   * re-selecting the same provider. No-op if none has ever been selected. */
  const retry = useCallback(() => {
    if (state.providerId) selectProvider(state.providerId);
  }, [selectProvider, state.providerId]);

  return {
    ...state,
    selectProvider,
    setLeadHours,
    setEnsemble,
    setVariable,
    loadLoop,
    retry,
  };
}
