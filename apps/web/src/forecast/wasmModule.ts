// Lazy, process-wide-singleton loader for `forecast-web`'s compiled wasm
// module -- same pattern as `src/alerts/wasmModule.ts` (for `weather-alerts`)
// and `src/radar/wasmModule.ts` (for `radar-web`). `wasm-bindgen --target
// web`'s default export instantiates the module (fetching
// `forecast_web_bg.wasm` relative to this file via `import.meta.url`); it
// must run exactly once per page, before `ProviderHandle` is constructed.
// Every caller (normally only `useForecastProvider`) awaits the same cached
// promise instead of re-instantiating.
//
// This import path (`../wasm/forecast_web.js`) is generated build output --
// see `apps/web/README.md` "Wasm build pipeline" and
// `apps/web/scripts/build-wasm.mjs`. It does not exist in a fresh checkout
// until `npm run build:wasm` (or `dev`/`build`/`typecheck`, which all run
// it automatically via npm's `pre*` script hooks) has been run at least
// once.
import init, { ProviderHandle } from "../wasm/forecast_web.js";

export { ProviderHandle };

let modulePromise: Promise<void> | null = null;

/** Ensure the `forecast-web` wasm module is instantiated. Safe to call more than once. */
export function ensureForecastWasmModuleLoaded(): Promise<void> {
  if (!modulePromise) {
    modulePromise = init().then(() => undefined);
  }
  return modulePromise;
}
