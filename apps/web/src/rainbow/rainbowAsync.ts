// S09d Parts B/C: tiny async helpers shared by `useRainbowNowcast.ts` and
// `useRainbowWeather.ts` -- NOT shared with `forecast/useForecastProvider.ts`
// (a different, GEFS/HRRR NWP concept per that module's own doc comment);
// this file only copies that hook's *shape* (a bounded per-call timeout,
// plain-string error normalization) for these two unrelated Rainbow panels,
// per this stage's explicit instruction to model on it without touching it.
//
// No `@tauri-apps/api/core` import here, even dynamic -- each hook already
// dynamically imports `invoke` itself right before calling it, same
// convention `desktopTiles.ts`/`platform/desktop.ts` establish (keeps the
// plain browser build from ever touching `window.__TAURI_INTERNALS__`, and
// keeps this file callable from a plain browser bundle with zero Tauri
// dependency at all).

/** Per-call timeout for a single Nowcast/Weather `invoke()` round trip.
 * Shorter than `useForecastProvider`'s 25s -- these are plain HTTP JSON
 * fetches (no wasm/GPU pipeline stage that can legitimately take longer),
 * so a much shorter bound still comfortably covers a real network request
 * while failing fast on a genuinely hung one. */
export const RAINBOW_API_CALL_TIMEOUT_MS = 15_000;

/** Race `promise` against a timeout, rejecting with a descriptive message
 * rather than leaving the caller waiting forever -- same shape as
 * `useForecastProvider.ts`'s `withTimeout` (see that module's doc comment
 * for the underlying rationale: this can only rescue a call stuck
 * *awaiting* something, not one that blocks the main thread synchronously,
 * which is not a concern for a plain `invoke()` HTTP call). */
export function withTimeout<T>(promise: Promise<T>, label: string, timeoutMs: number = RAINBOW_API_CALL_TIMEOUT_MS): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`${label} timed out after ${timeoutMs / 1000}s (no response)`));
    }, timeoutMs);
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

/** Normalizes any thrown/rejected value to a plain string. A Tauri
 * `invoke()` rejection for a command returning `Result<T, String>` (every
 * command in `rainbow.rs`/`rainbow_weather.rs`) surfaces as the bare
 * `String` the Rust side returned, but this also handles a real `Error`
 * (e.g. this module's own timeout above) or anything else defensively --
 * Global Contract: no uncontrolled/unhandled rejection ever reaches the UI
 * unformatted. */
export function rainbowErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

/** True when `err` (already known to have come from a Nowcast `invoke()`
 * rejection) represents the Rainbow API's HTTP 404 -- the documented
 * "no coverage here" outcome (KB §6.2/§9), distinct from every other
 * failure. Matches the exact `Err(format!("HTTP {status}"))` convention
 * `rainbow_nowcast_precip` (and every other Rainbow command) uses, so this
 * never has to guess at a differently-shaped error string. */
export function isRainbowHttp404(err: unknown): boolean {
  return rainbowErrorMessage(err) === "HTTP 404";
}
