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
export const FORECAST_RENDER_WIDTH = 480;
export const FORECAST_RENDER_HEIGHT = 320;

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

/** Only variable wired up in this stage -- S08's exit criteria is "the same
 * forecast UI and grid renderer switch between GEFS and HRRR", not a
 * variable picker, and both providers publish this one. */
export const FORECAST_VARIABLE = "temperature_2m";

/** Selectable forecast lead times (hours) for this proof-of-concept UI. */
export const FORECAST_LEAD_HOUR_OPTIONS = [0, 6, 12, 24, 48];

const DEFAULT_LEAD_HOURS = 6;

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
  /** Only meaningful (and only ever non-null) when `metadata.isEnsemble`. */
  ensemble: ForecastEnsembleSpec | null;
}

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
    ensemble: null,
  });

  const handleRef = useRef<ProviderHandle | null>(null);
  const generationRef = useRef(0);

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

  /** Paint `bytes` (RGBA8, `FORECAST_RENDER_WIDTH * FORECAST_RENDER_HEIGHT
   * * 4` long) onto the owned canvas. No-ops if the canvas is not mounted
   * (e.g. the panel unmounted mid-render). */
  const paint = useCallback(
    (bytes: Uint8Array) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      canvas.width = FORECAST_RENDER_WIDTH;
      canvas.height = FORECAST_RENDER_HEIGHT;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      // Copies into a fresh `Uint8ClampedArray` rather than viewing
      // `bytes.buffer` directly -- `Uint8Array.buffer` types as
      // `ArrayBufferLike` (could be a `SharedArrayBuffer`), which
      // `ImageData`'s constructor does not accept; the copy is cheap
      // relative to the GPU render/network fetch that produced `bytes`.
      const clamped = new Uint8ClampedArray(bytes);
      const imageData = new ImageData(clamped, FORECAST_RENDER_WIDTH, FORECAST_RENDER_HEIGHT);
      ctx.putImageData(imageData, 0, 0);
    },
    [canvasRef],
  );

  /** Fetch `FORECAST_VARIABLE`/`leadHours`/`ensemble` on `handle` and
   * render it, updating state at each step. Assumes a run has already been
   * discovered on this handle. Silently discards its own result (never
   * calls `setState`/`paint`) once `generation` is no longer the current
   * one -- see this hook's own doc comment. */
  const fetchAndRender = useCallback(
    async (handle: ProviderHandle, leadHours: number, ensemble: ForecastEnsembleSpec | null, generation: number) => {
      try {
        setState((s) => (generation === generationRef.current ? { ...s, phase: "fetching", error: null } : s));
        const gridJson = (await withTimeout(
          handle.fetchField(FORECAST_VARIABLE, leadHours, ensemble),
          "fetchField",
        )) as string;
        if (generation !== generationRef.current) return;
        const grid = JSON.parse(gridJson) as ForecastGridMetadata;
        setState((s) => (generation === generationRef.current ? { ...s, grid, phase: "rendering" } : s));

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
        if (generation !== generationRef.current) return;
        paint(pixels);
        setState((s) => (generation === generationRef.current ? { ...s, phase: "ready" } : s));
      } catch (err) {
        if (generation !== generationRef.current) return;
        setState((s) => ({ ...s, phase: "error", error: errMessage(err) }));
      }
    },
    [paint],
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
      setState({
        providerId,
        phase: "loading-module",
        error: null,
        metadata: null,
        run: null,
        grid: null,
        leadHours: DEFAULT_LEAD_HOURS,
        ensemble: null,
      });

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

          await fetchAndRender(handle, DEFAULT_LEAD_HOURS, ensemble, generation);
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
      void fetchAndRender(handle, leadHours, state.ensemble, generation);
    },
    [fetchAndRender, state.ensemble, state.run],
  );

  /** Re-fetch and re-render with a different ensemble statistic (GEFS
   * only) -- see `metadata.isEnsemble`. No-op for a deterministic provider
   * or before a run has been discovered. */
  const setEnsemble = useCallback(
    (ensemble: ForecastEnsembleSpec) => {
      const handle = handleRef.current;
      if (!handle || state.run === null || !state.metadata?.isEnsemble) return;
      const generation = generationRef.current;
      setState((s) => ({ ...s, ensemble }));
      void fetchAndRender(handle, state.leadHours, ensemble, generation);
    },
    [fetchAndRender, state.leadHours, state.metadata, state.run],
  );

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
    retry,
  };
}
