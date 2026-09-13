// RadarPro desktop-shell integration -- Stage S10 (Desktop Beta), Phase 1.
// See `Agent Context/context/stages/S10-desktop-beta.md` and
// `apps/desktop/src-tauri/src/lib.rs`.
//
// This module is the ONLY place `apps/web` talks to Tauri. It must stay
// additive: every export here is a no-op (resolves to `null`/does nothing)
// when the app is running as a plain browser page, so `npm run dev`/
// `npm run build`/`npm run typecheck` for the standalone web app are
// completely unaffected -- see GLOBAL_CONTRACT "the desktop shell is a new
// host for the exact same `apps/web` frontend, not a reimplementation".
//
// Deliberately narrow (per this project's "avoid premature abstraction"
// rule): one persisted setting (the default radar site) and one logging
// bridge, not a speculative generic settings/IPC framework.
import { isTauri } from "@tauri-apps/api/core";
import { LazyStore } from "@tauri-apps/plugin-store";
// `useScanHistory` is this setting's real domain owner (it defines the
// bounds the cache-limit setting must respect and the eviction logic that
// consumes it) -- reusing its exported bound/clamp here instead of
// redefining a second copy of the same magic numbers, per this stage's
// "don't invent a second settings mechanism" instruction extended to "don't
// invent a second copy of the same validation" too.
import { clampHistoryLimit } from "../scan/useScanHistory";

/** True only when this bundle is actually running inside the Tauri
 * desktop shell's webview -- always false in a plain browser tab, checked
 * at call time (not module-load time) so the browser build never touches
 * `window.__TAURI_INTERNALS__`. */
export function isDesktop(): boolean {
  return isTauri();
}

const SETTINGS_STORE_PATH = "settings.json";
const DEFAULT_SITE_KEY = "defaultSiteIcao";

// `LazyStore` only records its path at construction time -- it never
// invokes the backend until a method is actually called -- so creating
// this eagerly at module scope is safe even in a browser bundle that will
// never call `isDesktop()` truthily.
const settingsStore = new LazyStore(SETTINGS_STORE_PATH);

/**
 * Load the persisted default radar site ICAO, if any. Resolves to `null`
 * in a browser tab, on first run, or if the stored value is not a
 * well-formed 4-letter ICAO (defensively -- a hand-edited or corrupted
 * `settings.json` must never crash the app or select a bogus site).
 */
export async function loadPersistedDefaultSite(): Promise<string | null> {
  if (!isDesktop()) return null;
  try {
    const value = await settingsStore.get<string>(DEFAULT_SITE_KEY);
    if (typeof value === "string" && /^[A-Za-z0-9]{3,4}$/.test(value)) {
      return value.toUpperCase();
    }
    return null;
  } catch (err) {
    console.error("[desktop] failed to load persisted default site:", err);
    return null;
  }
}

/** Persist `icao` as the default radar site for next launch. No-op in a
 * browser tab. Fire-and-forget from the caller's perspective, but errors
 * are logged rather than silently swallowed. */
export function persistDefaultSite(icao: string): void {
  if (!isDesktop()) return;
  settingsStore
    .set(DEFAULT_SITE_KEY, icao)
    .then(() => settingsStore.save())
    .catch((err: unknown) => {
      console.error("[desktop] failed to persist default site:", err);
    });
}

const FAVORITE_MOMENTS_KEY = "favoriteMoments";
const CACHE_LIMIT_KEY = "cacheLimitScans";

/** Loose but real validation for a wire moment code (e.g. `"REF"`,
 * `"VEL"`, `"ZDR"`, `"SW"`, `"CC"`, `"KDP"`, `"PHI"`) -- short
 * alphanumeric/underscore tokens, matching what `radar-web`'s
 * `momentWireCodesForSweep` actually hands back. A hand-edited or
 * corrupted `settings.json` must never inject something weird into
 * `<option>`/button labels or `changeMoment` calls. */
const MOMENT_CODE_RE = /^[A-Za-z0-9_]{1,8}$/;
/** Arbitrary but generous cap on persisted favorites -- there are only a
 * handful of real WSR-88D moments per sweep, so this is purely a defensive
 * bound against a corrupted settings file, not a meaningful product limit. */
const MAX_FAVORITE_MOMENTS = 20;

function sanitizeFavoriteMoments(candidate: unknown): string[] {
  if (!Array.isArray(candidate)) return [];
  return candidate.filter((v): v is string => typeof v === "string" && MOMENT_CODE_RE.test(v)).slice(0, MAX_FAVORITE_MOMENTS);
}

/**
 * Load the persisted set of favorited/starred moment codes (e.g.
 * `["REF", "VEL"]`), if any. Resolves to `null` in a browser tab or on
 * first run -- callers should fall back to an empty list, exactly like
 * {@link loadPersistedDefaultSite}'s `null` convention.
 */
export async function loadPersistedFavoriteMoments(): Promise<string[] | null> {
  if (!isDesktop()) return null;
  try {
    const value = await settingsStore.get<unknown>(FAVORITE_MOMENTS_KEY);
    if (value === undefined) return null;
    return sanitizeFavoriteMoments(value);
  } catch (err) {
    console.error("[desktop] failed to load persisted favorite moments:", err);
    return null;
  }
}

/** Persist the given favorited moment codes for next launch. No-op in a
 * browser tab. Same fire-and-forget-but-logged shape as
 * {@link persistDefaultSite}. */
export function persistFavoriteMoments(moments: string[]): void {
  if (!isDesktop()) return;
  settingsStore
    .set(FAVORITE_MOMENTS_KEY, sanitizeFavoriteMoments(moments))
    .then(() => settingsStore.save())
    .catch((err: unknown) => {
      console.error("[desktop] failed to persist favorite moments:", err);
    });
}

/**
 * Load the persisted scan-history cache-size limit (see
 * `useScanHistory`'s `MIN_HISTORY_SCANS`/`MAX_HISTORY_SCANS_LIMIT`), if
 * any. Resolves to `null` in a browser tab, on first run, or for a
 * malformed stored value -- callers fall back to `MAX_HISTORY_SCANS`
 * (the built-in default), exactly like the default-site convention.
 */
export async function loadPersistedCacheLimit(): Promise<number | null> {
  if (!isDesktop()) return null;
  try {
    const value = await settingsStore.get<unknown>(CACHE_LIMIT_KEY);
    if (typeof value !== "number" || !Number.isFinite(value)) return null;
    return clampHistoryLimit(value);
  } catch (err) {
    console.error("[desktop] failed to load persisted cache limit:", err);
    return null;
  }
}

/** Persist `limit` (clamped into bounds) as the scan-history cache size
 * for next launch. No-op in a browser tab. */
export function persistCacheLimit(limit: number): void {
  if (!isDesktop()) return;
  settingsStore
    .set(CACHE_LIMIT_KEY, clampHistoryLimit(limit))
    .then(() => settingsStore.save())
    .catch((err: unknown) => {
      console.error("[desktop] failed to persist cache limit:", err);
    });
}

const RADAR_OPACITY_KEY = "radarOpacityPercent";

/** `0`..`100` integer -- this setting's own domain (unlike the cache-limit
 * bound above, there's no other module that already owns "radar opacity
 * percent", so the clamp lives here rather than being imported). */
function clampRadarOpacityPercent(value: number): number {
  return Math.round(Math.min(100, Math.max(0, value)));
}

/**
 * Load the persisted live-radar-canvas opacity (`0`..`100`, a percentage --
 * see `MapView`'s `radarOpacity` prop, which this feeds as `/100`), if any.
 * Resolves to `null` in a browser tab, on first run, or for a malformed
 * stored value -- callers fall back to `100` (fully opaque, this feature's
 * documented "unchanged until the user touches it" default), same
 * hydrate-then-persist convention as {@link loadPersistedCacheLimit}.
 */
export async function loadPersistedRadarOpacity(): Promise<number | null> {
  if (!isDesktop()) return null;
  try {
    const value = await settingsStore.get<unknown>(RADAR_OPACITY_KEY);
    if (typeof value !== "number" || !Number.isFinite(value)) return null;
    return clampRadarOpacityPercent(value);
  } catch (err) {
    console.error("[desktop] failed to load persisted radar opacity:", err);
    return null;
  }
}

/** Persist `percent` (clamped to `0`..`100`) as the live-radar opacity for
 * next launch. No-op in a browser tab. */
export function persistRadarOpacity(percent: number): void {
  if (!isDesktop()) return;
  settingsStore
    .set(RADAR_OPACITY_KEY, clampRadarOpacityPercent(percent))
    .then(() => settingsStore.save())
    .catch((err: unknown) => {
      console.error("[desktop] failed to persist radar opacity:", err);
    });
}

function formatConsoleArg(arg: unknown): string {
  if (typeof arg === "string") return arg;
  if (arg instanceof Error) return `${arg.name}: ${arg.message}`;
  try {
    return JSON.stringify(arg);
  } catch {
    return String(arg);
  }
}

/**
 * Forward this webview's `console.log`/`info`/`warn`/`error` calls into the
 * native shell's structured log file (the `LogDir` target configured in
 * `apps/desktop/src-tauri/src/lib.rs`) so GPU/backend init messages, decode
 * errors, etc. end up in one real on-disk log next to the native side's own
 * log lines -- this stage's "structured logging" requirement.
 *
 * NOTE: `@tauri-apps/plugin-log`'s own `attachConsole()` does the opposite
 * of what its name suggests -- it prints native Rust-side `log::` output
 * *into* the webview devtools console, not the other way around (confirmed
 * against its source during this stage's own verification). Forwarding
 * this webview's `console.*` calls *to* the Rust log file, which is what
 * this stage actually needs, means monkey-patching `console.*` to also
 * call the plugin's `trace`/`info`/`warn`/`error` functions -- there is no
 * built-in helper for it.
 *
 * No-op in a browser tab. Safe to call once at app startup.
 */
export async function attachDesktopLogging(): Promise<void> {
  if (!isDesktop()) return;
  try {
    const logPlugin = await import("@tauri-apps/plugin-log");
    const wrap = (
      original: (...args: unknown[]) => void,
      forwardTo: (message: string) => Promise<void>,
    ) => {
      return (...args: unknown[]) => {
        original(...args);
        forwardTo(args.map(formatConsoleArg).join(" ")).catch(() => {
          // Never let a logging failure cascade into a console-logging loop.
        });
      };
    };
    console.log = wrap(console.log.bind(console), logPlugin.trace);
    console.info = wrap(console.info.bind(console), logPlugin.info);
    console.warn = wrap(console.warn.bind(console), logPlugin.warn);
    console.error = wrap(console.error.bind(console), logPlugin.error);
  } catch (err) {
    // Never let logging setup break app startup.
    console.error("[desktop] failed to attach console logging:", err);
  }
}

/** Native-shell diagnostics (app/Tauri version, OS/arch, log file
 * location) -- see `get_diagnostics` in `apps/desktop/src-tauri/src/lib.rs`.
 * Resolves to `null` in a browser tab. */
export interface DesktopDiagnostics {
  appVersion: string;
  tauriVersion: string;
  os: string;
  arch: string;
  logDir: string | null;
}

/**
 * S10 Phase 4 update strategy -- see `apps/desktop/README.md`'s "Releases
 * and updates" section. Backed by `@tauri-apps/plugin-updater`, which
 * checks the GitHub Releases endpoint configured in
 * `apps/desktop/src-tauri/tauri.conf.json` (`plugins.updater.endpoints`)
 * and verifies the downloaded artifact against that config's `pubkey` -- a
 * self-generated Ed25519/minisign keypair. This is a completely separate,
 * free, no-CA mechanism from the Windows code-signing this project has
 * explicitly decided against (see `Agent Context/context/stages/
 * S10-desktop-beta.md`'s "Code signing" section) -- it verifies the update
 * *package*, not the app binary's publisher identity.
 */
export type UpdateCheckResult =
  | { status: "up-to-date" }
  | { status: "available"; version: string; notes: string | null }
  | { status: "error"; error: string };

/** Holds the `Update` handle returned by a successful `check()` call so a
 * following {@link installPendingUpdate} can act on it without checking
 * again. Desktop-only; never populated in a browser tab. Cleared once
 * consumed (or once a fresh check supersedes it). */
let pendingUpdate: Awaited<ReturnType<typeof import("@tauri-apps/plugin-updater").check>> | null = null;

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Check the configured update endpoint for a newer release. No-op (never
 * called) outside the desktop shell; callers should gate on {@link isDesktop}
 * the same as every other function in this module. Never throws -- a
 * network failure or malformed manifest comes back as `{ status: "error" }`,
 * not an exception, so a flaky connection can't crash the Settings panel. */
export async function checkForUpdate(): Promise<UpdateCheckResult> {
  if (!isDesktop()) return { status: "error", error: "not running in the desktop shell" };
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) {
      pendingUpdate = null;
      return { status: "up-to-date" };
    }
    pendingUpdate = update;
    return { status: "available", version: update.version, notes: update.body ?? null };
  } catch (err) {
    console.error("[desktop] update check failed:", err);
    pendingUpdate = null;
    return { status: "error", error: errorMessage(err) };
  }
}

/** Download progress for {@link installPendingUpdate}'s optional callback.
 * `totalBytes` is `null` until the download's `Started` event reports a
 * content length (some servers omit it). */
export interface UpdateDownloadProgress {
  downloadedBytes: number;
  totalBytes: number | null;
}

/**
 * Download and install the update found by the most recent
 * {@link checkForUpdate} call, then relaunch into the new version. Returns
 * `{ ok: false }` (never throws) if there is no pending update or the
 * download/install fails -- e.g. a connection drop partway through. A
 * successful install normally ends the process via `relaunch()` before
 * this promise resolves, so callers should treat "no response" as success,
 * not a hang.
 */
export async function installPendingUpdate(
  onProgress?: (progress: UpdateDownloadProgress) => void,
): Promise<{ ok: boolean; error?: string }> {
  if (!isDesktop()) return { ok: false, error: "not running in the desktop shell" };
  const update = pendingUpdate;
  if (!update) return { ok: false, error: "no update available -- check for updates first" };
  try {
    let downloadedBytes = 0;
    let totalBytes: number | null = null;
    await update.downloadAndInstall((event) => {
      if (event.event === "Started") {
        totalBytes = event.data.contentLength ?? null;
      } else if (event.event === "Progress") {
        downloadedBytes += event.data.chunkLength;
      }
      onProgress?.({ downloadedBytes, totalBytes });
    });
    pendingUpdate = null;
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
    return { ok: true };
  } catch (err) {
    console.error("[desktop] update install failed:", err);
    return { ok: false, error: errorMessage(err) };
  }
}

export async function getDesktopDiagnostics(): Promise<DesktopDiagnostics | null> {
  if (!isDesktop()) return null;
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const raw = await invoke<{
      app_version: string;
      tauri_version: string;
      os: string;
      arch: string;
      log_dir: string | null;
    }>("get_diagnostics");
    return {
      appVersion: raw.app_version,
      tauriVersion: raw.tauri_version,
      os: raw.os,
      arch: raw.arch,
      logDir: raw.log_dir,
    };
  } catch (err) {
    console.error("[desktop] failed to fetch diagnostics:", err);
    return null;
  }
}
