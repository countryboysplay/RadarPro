// Lazy, process-wide-singleton loader for `mrms-web`'s compiled wasm module
// -- same pattern as `src/forecast/wasmModule.ts` (for `forecast-web`),
// `src/alerts/wasmModule.ts` (for `weather-alerts`), and
// `src/radar/wasmModule.ts` (for `radar-web`). `wasm-bindgen --target
// web`'s default export instantiates the module (fetching `mrms_web_bg.wasm`
// relative to this file via `import.meta.url`); it must run exactly once
// per page, before `MrmsHandle` is constructed. Every caller (normally only
// `useMrmsOverlay`) awaits the same cached promise instead of
// re-instantiating.
//
// This import path (`../wasm/mrms_web.js`) is generated build output -- see
// `apps/web/README.md` "Wasm build pipeline" and
// `apps/web/scripts/build-wasm.mjs`. It does not exist in a fresh checkout
// until `npm run build:wasm` (or `dev`/`build`/`typecheck`, which all run it
// automatically via npm's `pre*` script hooks) has been run at least once.
import init, { MrmsHandle } from "../wasm/mrms_web.js";

export { MrmsHandle };

let modulePromise: Promise<void> | null = null;

/** Ensure the `mrms-web` wasm module is instantiated. Safe to call more than once. */
export function ensureMrmsWasmModuleLoaded(): Promise<void> {
  if (!modulePromise) {
    modulePromise = init().then(() => undefined);
  }
  return modulePromise;
}
