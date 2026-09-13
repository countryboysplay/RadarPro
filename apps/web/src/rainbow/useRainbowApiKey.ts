// S09c: reactive Rainbow API key -- Settings addendum.
//
// A key typed into the sidebar's Settings section is saved to
// `localStorage` and takes priority over the build-time
// `VITE_RAINBOW_API_KEY` env var when both are present; the env var remains
// a valid fallback for a self-hosted/Docker setup that prefers env-based
// config over per-browser Settings. This module is the single reactive
// source of truth for that resolution -- every consumer (the Settings
// input itself, and `useRainbowOverlay`'s "configured" gate) reads it via
// `useRainbowApiKey()` rather than each re-checking `localStorage`/
// `import.meta.env` on its own, so a key saved in Settings is reflected
// everywhere immediately, with no page reload.
//
// Built on a tiny external store (module-level listener set) rather than
// each hook instance polling localStorage on an interval: `setSettingsKey`
// notifies every subscriber synchronously, and `useSyncExternalStore` is
// the React-blessed way to subscribe a component to state that lives
// outside React itself (here, the browser's localStorage).
import { useCallback, useSyncExternalStore } from "react";
import { RAINBOW_ENV_API_KEY, RAINBOW_SETTINGS_STORAGE_KEY } from "./config";

type Listener = () => void;
const listeners = new Set<Listener>();

function readSettingsKeyRaw(): string {
  try {
    return localStorage.getItem(RAINBOW_SETTINGS_STORAGE_KEY)?.trim() ?? "";
  } catch {
    // localStorage can throw (private browsing with storage disabled,
    // locked-down browser settings, etc.) -- never let reading a config
    // value crash the app (Global Contract: fail independently).
    return "";
  }
}

function writeSettingsKeyRaw(key: string): void {
  try {
    if (key) {
      localStorage.setItem(RAINBOW_SETTINGS_STORAGE_KEY, key);
    } else {
      localStorage.removeItem(RAINBOW_SETTINGS_STORAGE_KEY);
    }
  } catch {
    // Best-effort persistence only -- in-memory subscribers in this tab
    // still update below even if the browser refuses to persist it.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  // Also react to another tab/window changing the same key (native
  // `storage` events never fire in the tab that made the change, which is
  // exactly why `writeSettingsKeyRaw` notifies `listeners` itself above).
  const onStorage = (e: StorageEvent) => {
    if (e.key === RAINBOW_SETTINGS_STORAGE_KEY || e.key === null) listener();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", onStorage);
  };
}

export type RainbowApiKeySource = "settings" | "env" | "none";

export interface RainbowApiKeyState {
  /** The key `useRainbowOverlay` should actually use: the Settings
   * override if present, else the build-time env var, else `""`. Never log
   * this value. */
  effectiveKey: string;
  /** Whether `effectiveKey` is non-empty. */
  configured: boolean;
  /** Which source `effectiveKey` came from -- lets the Settings UI say
   * "env var active as a fallback" vs. "using the key saved here". */
  source: RainbowApiKeySource;
  /** The raw value saved in Settings/localStorage (possibly `""`) -- the
   * Settings input's controlled value. Distinct from `effectiveKey`, which
   * falls back to the env var when this is empty. */
  settingsKey: string;
  /** Whether a build-time env var is present at all, regardless of whether
   * it's currently the active source. */
  envKeyPresent: boolean;
  /** Save (or, given `""`, clear) the Settings override. Written only to
   * this browser's `localStorage` -- never sent anywhere. */
  setSettingsKey: (key: string) => void;
}

/**
 * Non-hook accessor for the same resolution `useRainbowApiKey` performs
 * (Settings override, else the build-time env var, else `""`) -- for
 * callers that are not React components and so cannot use
 * `useSyncExternalStore`. Currently only `apps/web/src/rainbow/
 * desktopTiles.ts`'s `maplibregl.addProtocol` handler, which needs the
 * *current* key at the moment MapLibre requests a tile, not a value
 * captured once when the protocol was registered. Reads the same
 * `localStorage`/env-var sources `useRainbowApiKey` does -- no second copy
 * of the priority rule. Never log this value.
 */
export function getEffectiveRainbowApiKey(): string {
  return readSettingsKeyRaw() || RAINBOW_ENV_API_KEY;
}

export function useRainbowApiKey(): RainbowApiKeyState {
  const settingsKey = useSyncExternalStore(subscribe, readSettingsKeyRaw);

  const setSettingsKey = useCallback((key: string) => {
    writeSettingsKeyRaw(key.trim());
  }, []);

  const envKeyPresent = RAINBOW_ENV_API_KEY.length > 0;
  const effectiveKey = settingsKey || RAINBOW_ENV_API_KEY;
  const source: RainbowApiKeySource = settingsKey ? "settings" : envKeyPresent ? "env" : "none";

  return {
    effectiveKey,
    configured: effectiveKey.length > 0,
    source,
    settingsKey,
    envKeyPresent,
    setSettingsKey,
  };
}
