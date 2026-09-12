// Lazy, process-wide-singleton loader for `radar-web`'s compiled wasm
// module. `wasm-bindgen --target web`'s default export instantiates the
// module (fetching `radar_web_bg.wasm` relative to this file via
// `import.meta.url` -- see that generated file); it must run exactly once
// per page, before any of the module's other exports are called. Every
// caller (there is normally only one -- see `useRadarRenderer`) awaits the
// same cached promise instead of re-instantiating.
//
// This import path (`../wasm/radar_web.js`) is generated build output --
// see `apps/web/README.md` "Wasm build pipeline" and
// `apps/web/scripts/build-wasm.mjs`. It does not exist in a fresh checkout
// until `npm run build:wasm` (or `dev`/`build`/`typecheck`, which all run
// it automatically via npm's `pre*` script hooks) has been run at least
// once.
//
// S05: `radar-web`'s API was generalized from a fixed decode+render-once
// shape (`decodeSweep`/`renderFrame`/`SweepInfo`) to decode-once /
// select-and-render-many (`decodeVolume`/`selectAndRender`/
// `VolumeSummary`), plus color-table load/probe/range-ring exports -- see
// `crates/radar-web/src/browser.rs` and its README for the full contract.
// This module just re-exports the new surface; all of the actual calling
// logic lives in `useRadarRenderer.ts`.
import init, {
  initGpu,
  rangeRingsGeoJson,
  type ColorTableApplyResult,
  type GateProbeResult,
  type RadarWebRenderer,
  type VolumeSummary,
} from "../wasm/radar_web.js";

export type { RadarWebRenderer, VolumeSummary, ColorTableApplyResult, GateProbeResult };
export { initGpu, rangeRingsGeoJson };

let modulePromise: Promise<void> | null = null;

/** Ensure the wasm module is instantiated. Safe to call more than once. */
export function ensureWasmModuleLoaded(): Promise<void> {
  if (!modulePromise) {
    modulePromise = init().then(() => undefined);
  }
  return modulePromise;
}
