// S09b: Rainbow Weather (doc.rainbow.ai) tile overlay -- config.
//
// Per GLOBAL_CONTRACT's provider rules, Rainbow is an optional/keyed
// provider (not core like GEFS/HRRR) and "must fail independently -- the
// app must build and run fully without it" and "credentials never ship in
// source". This module names the two places a key can come from -- the
// build-time env var, and the localStorage key S09c's Settings section
// reads/writes -- but does not itself decide which one wins: see
// `useRainbowApiKey`, the single reactive source of truth every other
// Rainbow module (and the Settings UI) reads instead of each re-checking
// `import.meta.env`/`localStorage` itself.
//
// `import.meta.env.VITE_RAINBOW_API_KEY` is empty/undefined in any build
// that has no `.env.local` (see apps/web/.env.example) -- Vite inlines env
// vars at build time, so this is never a runtime fetch/network call and can
// never throw.
const RAW_ENV_KEY = import.meta.env.VITE_RAINBOW_API_KEY ?? "";

/** The build-time env var's key, or `""` if unset. A fallback only -- a key
 * saved in Settings (localStorage, see `RAINBOW_SETTINGS_STORAGE_KEY`)
 * takes priority whenever both are present. Never log this value. */
export const RAINBOW_ENV_API_KEY: string = RAW_ENV_KEY.trim();

/** localStorage key backing the Settings section's Rainbow API key field --
 * see `useRainbowApiKey`. Never sent anywhere; read/written only in this
 * browser. */
export const RAINBOW_SETTINGS_STORAGE_KEY = "radarpro.rainbowApiKey";

/** doc.rainbow.ai's API base -- verified 2026-09-13 (see
 * `Agent Context/context/stages/S09b-rainbow-tiles.md`). */
export const RAINBOW_API_BASE = "https://api.rainbow.ai";
