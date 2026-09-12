// Lazy, process-wide-singleton loader for `weather-alerts`'s compiled wasm
// module -- same pattern as `src/radar/wasmModule.ts` for `radar-web`.
// `wasm-bindgen --target web`'s default export instantiates the module
// (fetching `weather_alerts_bg.wasm` relative to this file via
// `import.meta.url`); it must run exactly once per page, before
// `AlertStoreHandle` or `parseAlertsJson` are used. Every caller (normally
// only `useAlertPoller`) awaits the same cached promise instead of
// re-instantiating.
//
// This import path (`../wasm/weather_alerts.js`) is generated build
// output -- see `apps/web/README.md` "Wasm build pipeline" and
// `apps/web/scripts/build-wasm.mjs`. It does not exist in a fresh checkout
// until `npm run build:wasm` (or `dev`/`build`/`typecheck`, which all run
// it automatically via npm's `pre*` script hooks) has been run at least
// once.
import init, { AlertStoreHandle, parseAlertsJson } from "../wasm/weather_alerts.js";

export { AlertStoreHandle, parseAlertsJson };

let modulePromise: Promise<void> | null = null;

/** Ensure the `weather-alerts` wasm module is instantiated. Safe to call more than once. */
export function ensureAlertsWasmModuleLoaded(): Promise<void> {
  if (!modulePromise) {
    modulePromise = init().then(() => undefined);
  }
  return modulePromise;
}
