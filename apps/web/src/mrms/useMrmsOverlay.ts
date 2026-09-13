import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { ensureMrmsWasmModuleLoaded, MrmsHandle } from "./wasmModule";
import {
  signedBoundsFromGeometry,
  type MrmsGridMetadata,
  type MrmsProductId,
  type MrmsSignedBounds,
  type MrmsSnapshotMetadata,
  type MrmsViewport,
} from "./types";

export type { MrmsViewport };

/** Debounce applied to viewport (pan/zoom) changes before a new render is
 * even requested -- matches how a map typically waits for movement to
 * settle (an `idle`/`moveend`-style event) rather than reacting to every
 * intermediate `move` frame. This is a hard requirement (Global Contract:
 * "an unthrottled GPU render + wasm call on every raw map `move` event
 * would be a real performance problem for a national-scale grid this
 * large"), not a nice-to-have -- doubly so here since, unlike GEFS/HRRR's
 * comparatively tiny grids, `renderCurrentGrid` re-uploads MRMS's full
 * 7000x3500 (24.5 million cell) value texture to the GPU on *every* call
 * (`mrms::palette::render_mrms_grid` builds a fresh `display_values` buffer
 * and hands it to `forecast_core::gpu::render_forecast_grid` each time --
 * there is no cross-call texture cache on the Rust side). 300ms (the upper
 * end of the stage brief's suggested 250-300ms range) is chosen specifically
 * to minimize how often that upload happens during a drag/zoom gesture. */
const VIEWPORT_DEBOUNCE_MS = 300;

/** MRMS publishes a new snapshot roughly every ~2 minutes into the current
 * UTC calendar day's key prefix (see `crates/mrms/src/client.rs`'s module
 * doc) -- unlike GEFS/HRRR's discrete once/twice-daily runs, there is no
 * need for a multi-day lookback; today plus one fallback day (covering the
 * few minutes right after UTC midnight, before today's prefix has any
 * objects yet) is generous. */
const DISCOVER_LOOKBACK_DAYS = 2;

/** Per-call timeout (ms) for every `MrmsHandle` wasm call -- same rationale
 * `useForecastProvider`'s own `CALL_TIMEOUT_MS` documents in detail
 * (Global Contract: "no silent/uncontrolled failures on untrusted network
 * data", and a real observed hang inside `forecast_core::gpu::
 * render_forecast_grid`'s GPU-adapter acquisition on this exact shared
 * render path). Higher than forecast's 25s: MRMS's per-call GPU texture
 * upload is ~20x larger (24.5M cells vs. GEFS/HRRR's ~1.2M), so a slow but
 * genuinely-progressing call needs more headroom before this gives up on
 * it.
 */
const CALL_TIMEOUT_MS = 35_000;

/** Floor applied to a requested half-extent before it reaches
 * `renderCurrentGrid` -- guards against a degenerate zero-size camera
 * (e.g. a not-yet-laid-out map reporting a zero-height viewport) that would
 * make the orthographic `clip_to_world` projection undefined. Small enough
 * to never visibly affect any real viewport. */
const MIN_HALF_EXTENT_DEG = 1e-4;

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

function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export type MrmsPhase =
  | "idle" // disabled -- nothing discovered, nothing on the canvas.
  | "loading-module"
  | "discovering"
  | "fetching"
  | "rendering"
  | "ready"
  | "no-coverage-in-view"
  | "error";

export interface MrmsOverlayState {
  phase: MrmsPhase;
  /** Non-null only when `phase === "error"`. */
  error: string | null;
  snapshot: MrmsSnapshotMetadata | null;
  grid: MrmsGridMetadata | null;
}

/** True whenever `viewport`'s bounding box has any overlap at all with
 * `dataBounds` (MRMS's real CONUS coverage). Used purely to decide whether
 * a render call is worth making at all -- *not* to reshape the camera
 * handed to `renderCurrentGrid` (see this module's other doc comment for
 * why not). */
function viewportOverlapsData(viewport: MrmsViewport, dataBounds: MrmsSignedBounds): boolean {
  const viewWest = viewport.centerLon - viewport.halfExtentLon;
  const viewEast = viewport.centerLon + viewport.halfExtentLon;
  const viewSouth = viewport.centerLat - viewport.halfExtentLat;
  const viewNorth = viewport.centerLat + viewport.halfExtentLat;
  return (
    viewEast > dataBounds.west &&
    viewWest < dataBounds.east &&
    viewNorth > dataBounds.south &&
    viewSouth < dataBounds.north
  );
}

/**
 * Owns one `mrms-web` `MrmsHandle` at a time (see
 * `crates/mrms-web/src/wasm_api.rs`) for whichever product
 * (`"reflectivity"` | `"precip_rate"`) is currently selected, and keeps a
 * `<canvas>` painted with a render of the most-recently-fetched snapshot
 * framed to the live map's current viewport.
 *
 * # Why this never clamps the camera it sends to `renderCurrentGrid`
 *
 * The stage brief calls for "clamp[ing] any requested camera to (intersect
 * with) these real bounds rather than requesting/rendering area with no
 * data". That intersection already happens, per-pixel, one layer down:
 * `forecast_grid.wgsl`'s fragment shader (the exact shared shader this
 * render path uses, unmodified) resolves each output pixel's world
 * position to a grid cell and returns fully-transparent
 * (`NO_DATA_COLOR`) for any cell outside `[0, grid_width) x [0,
 * grid_height)` -- i.e. any world position outside MRMS's real CONUS
 * footprint is already "clamped" to transparent with no JS-side help.
 * Reshaping the *requested camera itself* to a smaller intersected
 * rectangle would additionally require repositioning/resizing the overlay
 * canvas to match that smaller rectangle's on-screen projection (rather
 * than the simple "cover the whole map viewport, matching `MapView`'s
 * existing rings/alerts overlay canvases 1:1" approach used here) --
 * meaningfully more machinery for no correctness benefit, since the
 * shader-level clamp already guarantees the visual result is identical
 * either way. This hook instead uses `viewportOverlapsData` purely to
 * *skip* a render call entirely when the current viewport has zero overlap
 * with MRMS's coverage at all (e.g. panned to Europe) -- avoiding a wholly
 * wasted GPU texture upload for a result that would just be a blank
 * transparent canvas -- and to drive the `"no-coverage-in-view"` status so
 * that blank canvas is never presented as a silent/ambiguous state.
 *
 * # Debounce + generation counter (Global Contract hard requirement)
 *
 * Every viewport change is debounced by `VIEWPORT_DEBOUNCE_MS` before a
 * render is even requested (see that constant's doc comment for why this
 * matters especially for MRMS's much larger per-call GPU upload). On top of
 * that, a monotonically increasing generation counter -- the same pattern
 * `useForecastProvider` established -- is bumped on every new render
 * request (debounced-viewport-settled, product switch, or
 * enable/disable), and every async continuation checks it before touching
 * React state or the canvas: a still-in-flight `renderCurrentGrid` call
 * superseded by a newer one (the user kept panning before the first
 * resolved) can never paint a stale frame over a newer one, and can never
 * invalidate a *newer*, unrelated pipeline's own generation (each pipeline
 * switch clears any pending debounce timer synchronously before starting,
 * so a stale timer can never fire into a newer generation's territory).
 *
 * # No uncontrolled failures
 *
 * Every wasm call is wrapped in try/catch (via `withTimeout`); a thrown/
 * rejected error always becomes this hook's `error`/`phase: "error"` state,
 * never an unhandled rejection (Global Contract).
 *
 * # Large binary output kept out of React state
 *
 * `renderCurrentGrid`'s `Uint8Array` (`canvas.width * canvas.height * 4`
 * bytes -- potentially several megapixels) is painted straight onto the
 * caller's `<canvas>` via `putImageData` and never stored in React state,
 * same discipline `useForecastProvider` already established.
 */
export function useMrmsOverlay(
  canvasRef: RefObject<HTMLCanvasElement>,
  productId: MrmsProductId,
  enabled: boolean,
  viewport: MrmsViewport | null,
) {
  const [state, setState] = useState<MrmsOverlayState>({
    phase: "idle",
    error: null,
    snapshot: null,
    grid: null,
  });

  const handleRef = useRef<MrmsHandle | null>(null);
  const generationRef = useRef(0);
  const boundsRef = useRef<MrmsSignedBounds | null>(null);
  const viewportRef = useRef<MrmsViewport | null>(viewport);
  viewportRef.current = viewport;
  const debounceTimerRef = useRef<number | null>(null);

  const clearDebounceTimer = useCallback(() => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
  }, []);

  const freeHandle = useCallback(() => {
    try {
      handleRef.current?.free();
    } catch {
      // Nothing a caller could do about a failed free() of wasm-bindgen
      // memory -- never let this become an uncontrolled throw.
    }
    handleRef.current = null;
  }, []);

  const clearCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    ctx?.clearRect(0, 0, canvas.width, canvas.height);
  }, [canvasRef]);

  const paint = useCallback(
    (bytes: Uint8Array, width: number, height: number) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      // `MapView` may have resized the canvas's backing buffer again while
      // this call was in flight (a further pan/zoom, or a viewport resize)
      // -- skip rather than paint a now-mismatched-size buffer into it.
      if (canvas.width !== width || canvas.height !== height) return;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      // Copies into a fresh `Uint8ClampedArray` rather than viewing
      // `bytes.buffer` directly -- see `useForecastProvider.paint`'s
      // identical comment (`ImageData` rejects `ArrayBufferLike`).
      const clamped = new Uint8ClampedArray(bytes);
      const imageData = new ImageData(clamped, width, height);
      ctx.putImageData(imageData, 0, 0);
    },
    [canvasRef],
  );

  /** Render `vp` against the currently-fetched grid, guarded by
   * `generation`. Silently discards its own result once superseded -- see
   * this hook's own doc comment. */
  const renderAt = useCallback(
    async (handle: MrmsHandle, vp: MrmsViewport, generation: number) => {
      const canvas = canvasRef.current;
      const bounds = boundsRef.current;
      if (!canvas || !bounds) return;

      if (!viewportOverlapsData(vp, bounds)) {
        clearCanvas();
        setState((s) => (generation === generationRef.current ? { ...s, phase: "no-coverage-in-view", error: null } : s));
        return;
      }

      const width = canvas.width;
      const height = canvas.height;
      if (width === 0 || height === 0) return; // canvas not laid out yet.

      setState((s) => (generation === generationRef.current ? { ...s, phase: "rendering", error: null } : s));
      try {
        const pixels = (await withTimeout(
          handle.renderCurrentGrid(
            vp.centerLon,
            vp.centerLat,
            Math.max(vp.halfExtentLon, MIN_HALF_EXTENT_DEG),
            Math.max(vp.halfExtentLat, MIN_HALF_EXTENT_DEG),
            width,
            height,
          ),
          "renderCurrentGrid",
        )) as Uint8Array;
        if (generation !== generationRef.current) return;
        paint(pixels, width, height);
        setState((s) => (generation === generationRef.current ? { ...s, phase: "ready" } : s));
      } catch (err) {
        if (generation !== generationRef.current) return;
        setState((s) => ({ ...s, phase: "error", error: errMessage(err) }));
      }
    },
    [canvasRef, clearCanvas, paint],
  );

  /** Debounce a viewport change: only the last call within
   * `VIEWPORT_DEBOUNCE_MS` actually fires a render request. */
  const scheduleRender = useCallback(
    (vp: MrmsViewport) => {
      clearDebounceTimer();
      debounceTimerRef.current = window.setTimeout(() => {
        debounceTimerRef.current = null;
        const handle = handleRef.current;
        if (!handle || !boundsRef.current) return;
        const generation = ++generationRef.current;
        void renderAt(handle, vp, generation);
      }, VIEWPORT_DEBOUNCE_MS);
    },
    [clearDebounceTimer, renderAt],
  );

  /** Discover -> fetch the latest snapshot for `product`, then render it
   * immediately at whatever viewport is currently known (not debounced --
   * there is nothing to debounce against for a first render). Frees any
   * previous handle first, same "fresh handle per selection" shape
   * `useForecastProvider.selectProvider` uses. */
  const runPipeline = useCallback(
    (product: MrmsProductId) => {
      const generation = ++generationRef.current;
      clearDebounceTimer();
      freeHandle();
      boundsRef.current = null;
      clearCanvas();
      setState({ phase: "loading-module", error: null, snapshot: null, grid: null });

      void (async () => {
        try {
          await ensureMrmsWasmModuleLoaded();
          if (generation !== generationRef.current) return;

          const handle = new MrmsHandle(product);
          handleRef.current = handle;
          setState((s) => (generation === generationRef.current ? { ...s, phase: "discovering" } : s));

          const snapshotJson = (await withTimeout(
            handle.discoverLatestSnapshot(DISCOVER_LOOKBACK_DAYS),
            "discoverLatestSnapshot",
          )) as string;
          if (generation !== generationRef.current) return;
          const snapshot = JSON.parse(snapshotJson) as MrmsSnapshotMetadata;
          setState((s) => (generation === generationRef.current ? { ...s, snapshot, phase: "fetching" } : s));

          const gridJson = (await withTimeout(handle.fetchSnapshot(), "fetchSnapshot")) as string;
          if (generation !== generationRef.current) return;
          const grid = JSON.parse(gridJson) as MrmsGridMetadata;
          boundsRef.current = signedBoundsFromGeometry(grid.geometry);

          const vp = viewportRef.current;
          setState((s) => (generation === generationRef.current ? { ...s, grid, phase: vp ? "rendering" : "ready" } : s));
          if (vp) {
            await renderAt(handle, vp, generation);
          }
        } catch (err) {
          if (generation !== generationRef.current) return;
          setState((s) => ({ ...s, phase: "error", error: errMessage(err) }));
        }
      })();
    },
    [clearDebounceTimer, freeHandle, clearCanvas, renderAt],
  );

  // Discover -> fetch -> render whenever enabled or the selected product
  // changes; tear everything down (free the handle, clear the canvas, drop
  // back to "idle") when disabled. Mirrors `useRainbowOverlay`'s
  // enabled-gating shape: nothing is fetched/rendered/held in GPU memory
  // while the user has this overlay turned off.
  useEffect(() => {
    if (!enabled) {
      generationRef.current++; // invalidate anything still in flight.
      clearDebounceTimer();
      freeHandle();
      boundsRef.current = null;
      clearCanvas();
      setState({ phase: "idle", error: null, snapshot: null, grid: null });
      return;
    }
    runPipeline(productId);
    // Intentionally omits `runPipeline` from deps beyond what it already
    // closes over stably (this project's eslint config does not enable
    // react-hooks/exhaustive-deps) -- re-running on `enabled`/`productId`
    // alone is exactly the desired "discover on mount/product-change"
    // behavior the stage brief asks for.
  }, [enabled, productId]);

  // React to viewport (pan/zoom) changes once a grid is actually loaded --
  // debounced via `scheduleRender`. The very first render after a fetch is
  // triggered directly inside `runPipeline` above instead (nothing to
  // debounce against yet); this effect fires either for a *subsequent*
  // viewport change, or -- if the map had no viewport to give yet when the
  // fetch completed -- the first time `viewport` itself goes from `null` to
  // a real value. Deliberately keyed on `viewport` alone (not `state.grid`
  // too): re-running this just because `state.grid` changed would fire a
  // second, redundant debounced render on top of `runPipeline`'s own
  // immediate one every time a fetch completes with a viewport already
  // known -- the common case.
  useEffect(() => {
    if (!enabled || !viewport) return;
    if (!handleRef.current || !boundsRef.current) return;
    scheduleRender(viewport);
  }, [viewport, enabled, scheduleRender]);

  useEffect(() => {
    return () => {
      generationRef.current++;
      clearDebounceTimer();
      freeHandle();
    };
  }, [clearDebounceTimer, freeHandle]);

  const refresh = useCallback(() => {
    if (enabled) runPipeline(productId);
  }, [enabled, productId, runPipeline]);

  return { ...state, refresh };
}
