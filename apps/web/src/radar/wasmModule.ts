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
import init, { initGpu, RadarWebRenderer, SweepInfo } from "../wasm/radar_web.js";

export type { RadarWebRenderer, SweepInfo };
export { initGpu };

let modulePromise: Promise<void> | null = null;

/** Ensure the wasm module is instantiated. Safe to call more than once. */
export function ensureWasmModuleLoaded(): Promise<void> {
  if (!modulePromise) {
    modulePromise = init().then(() => undefined);
  }
  return modulePromise;
}
