import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { ensureWasmModuleLoaded, initGpu, rangeRingsGeoJson, type RadarWebRenderer } from "./wasmModule";

/** What the map shows, for `radar-web`'s `setView`: the radar canvas's
 * backing size in device pixels, the radar site, and the lon/lat box the
 * canvas covers. The sweep is redrawn for exactly this view, so it stays
 * sharp at any zoom. */
export interface RadarView {
  width: number;
  height: number;
  siteLat: number;
  siteLon: number;
  west: number;
  north: number;
  east: number;
  south: number;
}

export type RendererStatus = "loading" | "ready" | "error";

/** Decoded-volume metadata a UI needs to build an elevation picker and
 * display volume-level facts -- mirrors `radar-web`'s `VolumeSummary`, but
 * as a plain object (the wasm class itself is `.free()`d immediately after
 * extraction, per `wasm-bindgen`'s manual-memory-management contract). */
export interface VolumeMeta {
  siteIcao: string;
  sweepCount: number;
  /** One elevation angle (degrees) per sweep, `volume.sweeps` order. */
  elevationDegs: number[];
}

/** `GateProbeResult.state`'s three, deliberately-never-collapsed values --
 * see `crates/radar-web/src/browser.rs`'s `GateProbeResult` docs and
 * `COLOR_TABLE_FORMAT.md`'s "Missing and range-folded colors". */
export type ProbeState = "valid" | "missing" | "range_folded";

export interface ProbeResult {
  azimuthDeg: number;
  slantRangeKm: number;
  radialIndex: number;
  gateIndex: number;
  state: ProbeState;
  /** Present only when `state === "valid"`. */
  value: number | null;
  /** Present only when `state === "valid"`; from the active color table's
   * declared `units`, per `COLOR_TABLE_FORMAT.md`. */
  units: string | null;
}

export type ColorTableLoadResult =
  | { ok: true; name: string; appliedTo: string[] }
  | { ok: false; error: string };

export interface RadarRendererState {
  status: RendererStatus;
  error: string | null;
  adapterName: string | null;
  backend: string | null;
  /** The most recent `decodeVolume` failure message (a truncated/corrupted
   * download failed to parse), or `null` if the most recent decode
   * succeeded -- see `decodeVolume`'s doc comment. Distinct from `error`
   * above, which is the GPU/renderer-init failure, not a per-volume one. */
  decodeError: string | null;
}

function applyView(renderer: RadarWebRenderer, v: RadarView) {
  try {
    renderer.setView(v.width, v.height, v.siteLat, v.siteLon, v.west, v.north, v.east, v.south);
  } catch (err) {
    console.error("setView failed:", errMessage(err));
  }
}

function errMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/**
 * Owns the `radar-web` wasm renderer's lifecycle for one `<canvas>` (loads
 * the wasm module once, process-wide, and acquires a GPU device/surface for
 * the canvas on mount) and exposes its S05 decode-once/select-and-render-many
 * API -- plus color-table load, gate probing, and range-ring geometry -- as
 * stable callbacks bound to the live renderer instance.
 *
 * Per GLOBAL_CONTRACT ("Large binary arrays do not live in React state"),
 * raw volume bytes are never put into React state here or by any caller --
 * `decodeVolume` takes them as a plain function argument and hands them
 * straight to the wasm renderer, which keeps the decoded `Volume` in its
 * own (non-React) memory. Only small, already-extracted metadata becomes
 * state/return values here.
 */
export function useRadarRenderer(canvasRef: RefObject<HTMLCanvasElement>) {
  const [state, setState] = useState<RadarRendererState>({
    status: "loading",
    error: null,
    adapterName: null,
    backend: null,
    decodeError: null,
  });
  const rendererRef = useRef<RadarWebRenderer | null>(null);
  // The map usually reports its view before the GPU is ready; kept to apply then.
  const viewRef = useRef<RadarView | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    let cancelled = false;
    setState((s) => ({ ...s, status: "loading", error: null }));

    (async () => {
      try {
        await ensureWasmModuleLoaded();
        const renderer = (await initGpu(canvas, Math.max(1, canvas.width), Math.max(1, canvas.height))) as RadarWebRenderer;
        if (cancelled) {
          renderer.free();
          return;
        }
        rendererRef.current = renderer;
        if (viewRef.current) applyView(renderer, viewRef.current);
        setState((s) => ({
          ...s,
          status: "ready",
          adapterName: renderer.adapterName,
          backend: renderer.backend,
        }));
        // Diagnostic breadcrumb (not user-facing UI, which already shows
        // this in the top bar) -- on desktop this is forwarded into the
        // native log file via `attachDesktopLogging`, giving S10's
        // "GPU/backend reporting" requirement a real on-disk record, not
        // just a transient status line. Harmless in a browser tab.
        console.info(`[radar] GPU ready: adapter="${renderer.adapterName}" backend="${renderer.backend}"`);
      } catch (err) {
        if (cancelled) return;
        setState((s) => ({ ...s, status: "error", error: errMessage(err) }));
      }
    })();

    return () => {
      cancelled = true;
      rendererRef.current?.free();
      rendererRef.current = null;
    };
    // Runs once per mounted canvas (empty deps -- `canvasRef` is a stable
    // ref object); size changes go through `setView`, not re-init.
  }, []);

  /**
   * Decode `bytes` (a full raw Archive II volume) once and keep it in the
   * wasm renderer's own memory. Does not render anything -- call
   * `selectAndRender` afterward. Returns `null` if the renderer is not
   * ready yet, *or* if `bytes` fails to decode (a truncated/corrupted
   * download -- see S10 Phase 3's "partial downloads" reliability pass).
   *
   * # S10 Phase 3: this used to be the one renderer call in this file with
   * no try/catch around it
   *
   * `radar-web`'s `decodeVolume` is a `Result<VolumeSummary, JsValue>` on
   * the Rust side, which `wasm-bindgen` turns into a JS function that
   * *throws* on `Err` -- exactly like every other fallible method on this
   * renderer (`selectAndRender`, `probeGate`, etc.), all of which this file
   * already wraps in try/catch. This one call was not, and live-verifying
   * against a real truncated NEXRAD download (a fetch response body sliced
   * short) turned up the real consequence: the thrown error propagated,
   * uncaught, out of the `useEffect` in `App.tsx` that calls this, and
   * React -- with no error boundary anywhere in this app -- unmounted the
   * *entire* tree to a blank white screen. A single bad download from an
   * otherwise-healthy feed was enough to white-screen the app. Now this
   * matches every sibling method: caught, logged, and reported through
   * `decodeError` below instead of crashing.
   */
  const decodeVolume = useCallback((bytes: Uint8Array): VolumeMeta | null => {
    const renderer = rendererRef.current;
    if (!renderer) return null;
    try {
      const summary = renderer.decodeVolume(bytes);
      try {
        const meta = {
          siteIcao: summary.siteIcao,
          sweepCount: summary.sweepCount,
          elevationDegs: Array.from(summary.elevationDegs()),
        };
        setState((s) => (s.decodeError === null ? s : { ...s, decodeError: null }));
        return meta;
      } finally {
        // `VolumeSummary` is a wasm-bindgen class backed by linear-memory
        // allocations the JS garbage collector does not know about.
        summary.free();
      }
    } catch (err) {
      const message = errMessage(err);
      console.error("decodeVolume failed (likely a truncated/corrupted download):", message);
      setState((s) => ({ ...s, decodeError: message }));
      return null;
    }
  }, []);

  /** Every moment wire code (`"REF"`, `"VEL"`, ...) present on sweep
   * `sweepIndex` of the currently-decoded volume. Empty if the renderer
   * isn't ready or nothing has been decoded yet. */
  const momentWireCodesForSweep = useCallback((sweepIndex: number): string[] => {
    const renderer = rendererRef.current;
    if (!renderer) return [];
    try {
      return renderer.momentWireCodesForSweep(sweepIndex);
    } catch (err) {
      console.error("momentWireCodesForSweep failed:", errMessage(err));
      return [];
    }
  }, []);

  /** The lowest-elevation sweep index carrying `momentCode`, or `null`. */
  const defaultSweepIndexForMoment = useCallback((momentCode: string): number | null => {
    const renderer = rendererRef.current;
    if (!renderer) return null;
    try {
      const idx = renderer.defaultSweepIndexForMoment(momentCode);
      return idx === undefined ? null : idx;
    } catch (err) {
      console.error("defaultSweepIndexForMoment failed:", errMessage(err));
      return null;
    }
  }, []);

  /**
   * Re-render the given `(sweepIndex, momentCode)` selection from the
   * already-decoded volume -- cheap, no re-decode. Returns `false` on
   * failure (renderer not ready, or the sweep doesn't carry that moment)
   * without throwing.
   */
  /** Frame (and redraw) the sweep for what the map now shows -- see `RadarView`. */
  const setView = useCallback((view: RadarView): void => {
    viewRef.current = view;
    const renderer = rendererRef.current;
    if (renderer) applyView(renderer, view);
  }, []);

  const selectAndRender = useCallback((sweepIndex: number, momentCode: string): boolean => {
    const renderer = rendererRef.current;
    if (!renderer) return false;
    try {
      renderer.selectAndRender(sweepIndex, momentCode);
      return true;
    } catch (err) {
      console.error("selectAndRender failed:", errMessage(err));
      return false;
    }
  }, []);

  /**
   * Re-render Storm-Relative Velocity (S11 Phase 2b) for `sweepIndex` from
   * that sweep's VEL moment, applying a uniform storm-motion vector
   * (`stormSpeedMps` in meters per second, `stormDirectionDeg` the compass
   * bearing in degrees clockwise from true north the storm is moving
   * *toward*). Same shape as `selectAndRender` above: returns `false` on
   * failure (renderer not ready, or the sweep doesn't carry VEL to derive
   * SRV from) without throwing. Cheap to call repeatedly as the storm-motion
   * inputs change -- see `radar-web`'s `selectAndRenderStormRelativeVelocity`
   * doc comment for why a changed motion vector alone still triggers a real
   * re-render rather than reusing a stale GPU upload.
   */
  const selectAndRenderStormRelativeVelocity = useCallback(
    (sweepIndex: number, stormSpeedMps: number, stormDirectionDeg: number): boolean => {
      const renderer = rendererRef.current;
      if (!renderer) return false;
      try {
        renderer.selectAndRenderStormRelativeVelocity(sweepIndex, stormSpeedMps, stormDirectionDeg);
          return true;
      } catch (err) {
        console.error("selectAndRenderStormRelativeVelocity failed:", errMessage(err));
        return false;
      }
    },
    [],
  );

  /**
   * Parse/validate/apply a user- or preset-supplied color table. Never
   * throws -- a malformed table (structurally invalid JSON, or JSON that
   * fails `ColorTable` validation) comes back as `{ ok: false, error }`
   * with the exact message the Rust side produced, and the previously
   * active table for every affected moment is left untouched.
   */
  const loadColorTable = useCallback((json: string): ColorTableLoadResult => {
    const renderer = rendererRef.current;
    if (!renderer) return { ok: false, error: "renderer not ready yet" };
    try {
      const result = renderer.loadColorTable(json);
      try {
        return { ok: true, name: result.name, appliedTo: result.appliedTo() };
      } finally {
        result.free();
      }
    } catch (err) {
      return { ok: false, error: errMessage(err) };
    }
  }, []);

  /** Revert `momentCode` to `radar-web`'s built-in default palette. */
  const resetColorTable = useCallback((momentCode: string): void => {
    try {
      rendererRef.current?.resetColorTable(momentCode);
    } catch (err) {
      console.error("resetColorTable failed:", errMessage(err));
    }
  }, []);

  /** Show dealiased (`true`) or raw (`false`) velocity in later VEL/SRV
   * renders, probes and cross-sections. Re-render the selection afterward. */
  const setDealias = useCallback((enabled: boolean): void => {
    try {
      rendererRef.current?.setDealias(enabled);
    } catch (err) {
      console.error("setDealias failed:", errMessage(err));
    }
  }, []);

  /** Hide (`true`) or show (`false`) non-weather reflectivity (low-CC
   * insects/birds/clutter) in later REF renders, probes and cross-sections.
   * Re-render the selection afterward. */
  const setHideNonWeather = useCallback((enabled: boolean): void => {
    try {
      rendererRef.current?.setHideNonWeather(enabled);
    } catch (err) {
      console.error("setHideNonWeather failed:", errMessage(err));
    }
  }, []);

  /** The active color table for `momentCode`, as its original JSON text --
   * for the color-table editor to display/edit. `null` if unavailable. */
  const activeColorTableJson = useCallback((momentCode: string): string | null => {
    const renderer = rendererRef.current;
    if (!renderer) return null;
    try {
      return renderer.activeColorTableJson(momentCode);
    } catch (err) {
      console.error("activeColorTableJson failed:", errMessage(err));
      return null;
    }
  }, []);

  /**
   * Resolve a map cursor position to a gate on the given sweep/moment.
   * Returns `null` both when the renderer isn't ready/nothing is decoded
   * *and* when the cursor legitimately falls outside the sweep's coverage
   * (radar-web's own `Ok(None)` case) -- callers that need to tell those
   * apart for UI purposes can check `status`/`momentWireCodesForSweep`
   * first; this hook does not conflate them into a thrown error either way.
   */
  const probeGate = useCallback(
    (
      siteLatDeg: number,
      siteLonDeg: number,
      sweepIndex: number,
      momentCode: string,
      cursorLatDeg: number,
      cursorLonDeg: number,
    ): ProbeResult | null => {
      const renderer = rendererRef.current;
      if (!renderer) return null;
      try {
        const result = renderer.probeGate(
          siteLatDeg,
          siteLonDeg,
          sweepIndex,
          momentCode,
          cursorLatDeg,
          cursorLonDeg,
        );
        if (!result) return null;
        try {
          const rawState = result.state;
          if (rawState !== "valid" && rawState !== "missing" && rawState !== "range_folded") {
            console.error(`probeGate returned an unrecognized state: ${rawState}`);
            return null;
          }
          return {
            azimuthDeg: result.azimuthDeg,
            slantRangeKm: result.slantRangeKm,
            radialIndex: result.radialIndex,
            gateIndex: result.gateIndex,
            state: rawState,
            value: result.value ?? null,
            units: result.units ?? null,
          };
        } finally {
          result.free();
        }
      } catch (err) {
        console.error("probeGate failed:", errMessage(err));
        return null;
      }
    },
    [],
  );

  /** Range-ring geometry around a site, as a nested `[[ [lon, lat], ... ],
   * ...]` array (one ring per radius) -- directly usable as a MapLibre
   * `MultiLineString`'s `coordinates`. `null` before the wasm module has
   * loaded (this needs no decoded volume/GPU state, only the module). */
  const rangeRings = useCallback(
    (siteLatDeg: number, siteLonDeg: number, radiiKm: number[], numPoints: number): number[][][] | null => {
      if (state.status === "loading") return null;
      try {
        return rangeRingsGeoJson(
          siteLatDeg,
          siteLonDeg,
          new Float64Array(radiiKm),
          numPoints,
        ) as number[][][];
      } catch (err) {
        console.error("rangeRingsGeoJson failed:", errMessage(err));
        return null;
      }
    },
    [state.status],
  );

  /** RGBA bytes of a vertical slice through the decoded volume (see
   * `radar-web`'s `crossSection`), or `null` if unavailable. */
  const crossSection = useCallback(
    (
      momentCode: string,
      a: { lat: number; lon: number },
      b: { lat: number; lon: number },
      cols: number,
      rows: number,
      topKm: number,
    ): Uint8Array | null => {
      const renderer = rendererRef.current;
      if (!renderer) return null;
      try {
        return renderer.crossSection(momentCode, a.lat, a.lon, b.lat, b.lon, cols, rows, topKm);
      } catch (err) {
        console.error("crossSection failed:", errMessage(err));
        return null;
      }
    },
    [],
  );

  return {
    ...state,
    setView,
    crossSection,
    decodeVolume,
    momentWireCodesForSweep,
    defaultSweepIndexForMoment,
    selectAndRender,
    selectAndRenderStormRelativeVelocity,
    loadColorTable,
    resetColorTable,
    setDealias,
    setHideNonWeather,
    activeColorTableJson,
    probeGate,
    rangeRings,
  };
}
