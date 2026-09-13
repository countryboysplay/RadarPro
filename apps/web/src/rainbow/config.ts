// S09b: Rainbow Weather (doc.rainbow.ai) tile overlay -- config.
//
// Per GLOBAL_CONTRACT's provider rules, Rainbow is an optional/keyed
// provider (not core like GEFS/HRRR) and "must fail independently -- the
// app must build and run fully without it" and "credentials never ship in
// source". This module is the single place that reads the env var, so
// every other Rainbow module treats "configured or not" as one boolean
// rather than each re-checking `import.meta.env` itself.
//
// `import.meta.env.VITE_RAINBOW_API_KEY` is empty/undefined in any build
// that has no `.env.local` (see apps/web/.env.example) -- Vite inlines env
// vars at build time, so this is never a runtime fetch/network call and can
// never throw.
const RAW_KEY = import.meta.env.VITE_RAINBOW_API_KEY ?? "";

/** The user-supplied key, or `""` if unset. Never log this value. */
export const RAINBOW_API_KEY: string = RAW_KEY.trim();

/** Whether the Rainbow overlay has a key to work with at all. Every
 * Rainbow-facing UI element must gate on this rather than assuming the key
 * is present -- see `RainbowToggle`. */
export function isRainbowConfigured(): boolean {
  return RAINBOW_API_KEY.length > 0;
}

/** doc.rainbow.ai's API base -- verified 2026-09-13 (see
 * `Agent Context/context/stages/S09b-rainbow-tiles.md`). */
export const RAINBOW_API_BASE = "https://api.rainbow.ai";
