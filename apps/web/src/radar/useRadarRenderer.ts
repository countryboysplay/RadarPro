import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { ensureWasmModuleLoaded, initGpu, type RadarWebRenderer } from "./wasmModule";

/** Fixed backing-buffer size for the radar `<canvas>` (device pixels, no
 * `devicePixelRatio` scaling -- same choice `radar-web`'s own `www/` test
 * page makes, see its README "Canvas sizing"). The canvas is then scaled
 * on-screen via CSS by `MapView` to approximate the sweep's real-world
 * extent at the map's current zoom -- see that component's docs. Using a
 * fixed backing buffer (rather than resizing the GPU surface to match
 * on-screen pixels) means pan/zoom/resize never need to re-touch the GPU
 * surface at all -- only CSS -- which is why this stage did not need to
 * add resize support to `radar-web`'s exported API. */
export const RADAR_CANVAS_SIZE = 900;

export type RendererStatus = "loading" | "ready" | "error";

export interface SweepMeta {
  siteIcao: string;
  sweepCount: number;
  elevationDeg: number;
  radialCount: number;
}

export interface RadarRendererState {
  status: RendererStatus;
  error: string | null;
  adapterName: string | null;
  backend: string | null;
  sweepInfo: SweepMeta | null;
}

/**
 * Owns the `radar-web` wasm renderer's lifecycle for one `<canvas>`: loads
 * the wasm module (once, process-wide), acquires a GPU device/surface for
 * the canvas on mount, and exposes an imperative `renderVolume` for
 * decoding + rendering newly downloaded Archive II bytes.
 *
 * Per GLOBAL_CONTRACT ("Large binary arrays do not live in React state"),
 * the raw volume bytes are never put into React state here or by any
 * caller -- `renderVolume` takes them as a plain function argument and
 * hands them straight to the wasm renderer. Only small, already-extracted
 * metadata (`SweepMeta`) becomes state, for display.
 */
export function useRadarRenderer(canvasRef: RefObject<HTMLCanvasElement>) {
  const [state, setState] = useState<RadarRendererState>({
    status: "loading",
    error: null,
    adapterName: null,
    backend: null,
    sweepInfo: null,
  });
  const rendererRef = useRef<RadarWebRenderer | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    canvas.width = RADAR_CANVAS_SIZE;
    canvas.height = RADAR_CANVAS_SIZE;

    let cancelled = false;
    setState((s) => ({ ...s, status: "loading", error: null }));

    (async () => {
      try {
        await ensureWasmModuleLoaded();
        const renderer = (await initGpu(
          canvas,
          RADAR_CANVAS_SIZE,
          RADAR_CANVAS_SIZE,
        )) as RadarWebRenderer;
        if (cancelled) {
          renderer.free();
          return;
        }
        rendererRef.current = renderer;
        setState((s) => ({
          ...s,
          status: "ready",
          adapterName: renderer.adapterName,
          backend: renderer.backend,
        }));
      } catch (err) {
        if (cancelled) return;
        setState((s) => ({
          ...s,
          status: "error",
          error: err instanceof Error ? err.message : String(err),
        }));
      }
    })();

    return () => {
      cancelled = true;
      rendererRef.current?.free();
      rendererRef.current = null;
    };
    // Runs once per mounted canvas (empty deps -- `canvasRef` is a stable
    // ref object) -- `radar-web` has no resize/re-init API (see
    // RADAR_CANVAS_SIZE doc comment), so there is nothing else to react to.
  }, []);

  /**
   * Decode + render one volume's bytes immediately (synchronously, aside
   * from the underlying GPU calls). Returns the decoded sweep's metadata,
   * or `null` if the renderer is not ready yet (e.g. GPU init still in
   * flight) -- the caller is expected to hold onto the bytes and retry
   * once `status` becomes `"ready"`; see `App.tsx`.
   */
  const renderVolume = useCallback((bytes: Uint8Array): SweepMeta | null => {
    const renderer = rendererRef.current;
    if (!renderer) return null;

    const info = renderer.decodeSweep(bytes);
    try {
      const meta: SweepMeta = {
        siteIcao: info.siteIcao,
        sweepCount: info.sweepCount,
        elevationDeg: info.elevationDeg,
        radialCount: info.radialCount,
      };
      renderer.renderFrame();
      setState((s) => ({ ...s, sweepInfo: meta }));
      return meta;
    } finally {
      // `SweepInfo` is a wasm-bindgen class backed by linear-memory
      // allocations the JS garbage collector does not know about --
      // without an explicit `free()`, every poll (every ~45s, indefinitely,
      // for as long as the page stays open) would leak.
      info.free();
    }
  }, []);

  return { ...state, renderVolume };
}
