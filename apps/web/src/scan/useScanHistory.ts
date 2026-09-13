import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import type { DiscoveredVolume } from "../nexrad/keys";
import { type PollEvent, useScanPoller } from "./useScanPoller";

/**
 * Hard cap on how many recently-downloaded scans are held in memory at
 * once (per GLOBAL_CONTRACT / S05: "Keep RAM bounded during long loops").
 * 15 scans of a typical WSR-88D volume (a handful of MB each, variable with
 * VCP/precip coverage) is comfortably bounded (tens of MB, not hundreds)
 * while still giving a useful ~1-2 hours of animation history at the
 * typical 4-10 minute WSR-88D volume cadence. Oldest scan is evicted first
 * (FIFO) once a new one would exceed this cap -- see `historyReducer`'s
 * `SCAN_ADDED` case.
 */
export const MAX_HISTORY_SCANS = 15;

/**
 * S10 Phase 2: sane bounds for the user-adjustable cache size setting
 * (Settings sidebar section, backed by `platform/desktop.ts`'s
 * `loadPersistedCacheLimit`/`persistCacheLimit`). `MIN_HISTORY_SCANS` keeps
 * enough held frames for previous/next/loop to still mean something;
 * `MAX_HISTORY_SCANS_LIMIT` (4x the {@link MAX_HISTORY_SCANS} default) keeps
 * worst-case memory bounded -- the same "a handful of MB each" volumes
 * `MAX_HISTORY_SCANS`'s own doc comment reasons about, at 4x the cap, stay
 * in the "well under a GB" range rather than becoming unbounded.
 */
export const MIN_HISTORY_SCANS = 5;
export const MAX_HISTORY_SCANS_LIMIT = 60;

/**
 * S10 Phase 3 reliability pass: how old the currently-*live* frame's own
 * volume start time must get before the UI stops calling it "live" without
 * qualification. `useScanPoller`'s own doc comment already establishes that
 * WSR-88D volumes normally complete every 4-10 minutes -- so "now minus this
 * volume's start time" naturally drifts up toward that 10-minute figure even
 * under completely healthy polling, right before the next volume lands.
 * Twice that slow-end figure gives a full extra cycle of buffer (covers one
 * missed/delayed poll) before flagging anything, so this must never
 * false-positive during normal operation, while still catching a real stuck
 * feed (site down, or the poller silently never finding a newer key) well
 * inside the "several minutes" a person would actually notice something is
 * wrong -- see this module's `useScanHistory` doc comment for the bug this
 * fixes (a site's feed going stale while the badge kept reading "live").
 */
export const STALE_LIVE_AFTER_MS = 20 * 60_000;

/** How often the "is the live frame stale" clock re-checks itself. Cheap
 * (a plain comparison, no network/decode work) and only needs
 * minute-grained freshness for a human-readable "no new scan in Xm" label,
 * so a coarse interval is deliberate here, unlike `ADVANCE_FRAME`'s
 * animation timer above. */
const STALE_CHECK_INTERVAL_MS = 15_000;

/** Clamp any candidate cache-size value (e.g. read back from a
 * hand-editable `settings.json`, or typed into the Settings field) into
 * `[MIN_HISTORY_SCANS, MAX_HISTORY_SCANS_LIMIT]`, defensively falling back
 * to the default for a non-finite input. Never trust a raw number from
 * disk or a text input directly against the eviction loop below. */
export function clampHistoryLimit(candidate: number): number {
  if (!Number.isFinite(candidate)) return MAX_HISTORY_SCANS;
  return Math.max(MIN_HISTORY_SCANS, Math.min(MAX_HISTORY_SCANS_LIMIT, Math.round(candidate)));
}

/**
 * - `"live"`: always shows the most recently downloaded scan; a new scan
 *   landing in the background immediately becomes the displayed frame.
 * - `"paused"`: holds whatever frame is currently displayed; new scans
 *   still download into the bounded cache in the background but the view
 *   does not change until the user pauses/resumes/jumps to latest.
 * - `"playing"`: animates forward through the held history at `frameMs`
 *   per frame, looping back to the oldest held frame after the newest
 *   (chosen over ping-pong or stop-at-end as the simpler, more
 *   "traditional radar loop" default) without ever re-downloading a frame
 *   already in the cache.
 */
export type PlayMode = "live" | "paused" | "playing";

export interface HistoryEntryMeta {
  /** The S3 object key -- also the cache/eviction key. */
  key: string;
  startTimeMillis: number;
}

interface HistoryState {
  entries: HistoryEntryMeta[];
  currentIndex: number;
  playMode: PlayMode;
  frameMs: number;
  /** Current cache cap (see `MIN_HISTORY_SCANS`/`MAX_HISTORY_SCANS_LIMIT`
   * above) -- carried in state, not read from the module-level
   * `MAX_HISTORY_SCANS` constant, so the Settings panel's live value drives
   * eviction directly. */
  maxEntries: number;
}

type HistoryAction =
  | { type: "RESET" }
  | { type: "SCAN_ADDED"; key: string; startTimeMillis: number }
  | { type: "STEP"; delta: -1 | 1 }
  | { type: "TOGGLE_PLAY" }
  | { type: "ADVANCE_FRAME" }
  | { type: "SET_FRAME_MS"; ms: number }
  | { type: "JUMP_LATEST" }
  | { type: "JUMP_TO_TIME"; millis: number }
  | { type: "SET_MAX_ENTRIES"; maxEntries: number };

const MIN_FRAME_MS = 100;
const MAX_FRAME_MS = 4000;
export const DEFAULT_FRAME_MS = 700;

function initialState(maxEntries: number = MAX_HISTORY_SCANS): HistoryState {
  return { entries: [], currentIndex: -1, playMode: "live", frameMs: DEFAULT_FRAME_MS, maxEntries: clampHistoryLimit(maxEntries) };
}

/** Shared FIFO-eviction trim used by both `SCAN_ADDED` (a new scan pushed
 * the count over the cap) and `SET_MAX_ENTRIES` (the cap itself shrank
 * below the currently-held count) -- same "evict oldest first, shift
 * `currentIndex` down to keep pointing at the same logical frame" rule
 * either way. */
function trimToCap(entries: HistoryEntryMeta[], currentIndex: number, cap: number): { entries: HistoryEntryMeta[]; currentIndex: number } {
  while (entries.length > cap) {
    entries = entries.slice(1);
    currentIndex -= 1;
  }
  return { entries, currentIndex: clampIndex(currentIndex, entries.length) };
}

function clampIndex(index: number, length: number): number {
  if (length === 0) return -1;
  return Math.max(0, Math.min(index, length - 1));
}

function historyReducer(state: HistoryState, action: HistoryAction): HistoryState {
  switch (action.type) {
    case "RESET":
      return { ...initialState(state.maxEntries), frameMs: state.frameMs };

    case "SCAN_ADDED": {
      const grown = [...state.entries, { key: action.key, startTimeMillis: action.startTimeMillis }];
      // Evict oldest first (FIFO) once over the cap -- shift every held
      // index down to match, so "previous/next" and "playing" keep
      // pointing at the same logical frame across an eviction.
      const trimmed = trimToCap(grown, state.currentIndex, state.maxEntries);
      const entries = trimmed.entries;
      let currentIndex = trimmed.currentIndex;
      // "Live" mode always tracks the newest held frame -- this is the
      // *only* path that auto-jumps the displayed frame forward; paused/
      // playing modes leave `currentIndex` untouched so a new background
      // download never yanks the view away from what's being reviewed.
      if (state.playMode === "live") {
        currentIndex = entries.length - 1;
      }
      return { ...state, entries, currentIndex };
    }

    case "STEP": {
      if (state.entries.length === 0) return state;
      const currentIndex = clampIndex(state.currentIndex + action.delta, state.entries.length);
      // Manually stepping is always a deliberate "look at history" action.
      return { ...state, currentIndex, playMode: "paused" };
    }

    case "TOGGLE_PLAY":
      if (state.entries.length === 0) return state;
      return { ...state, playMode: state.playMode === "playing" ? "paused" : "playing" };

    case "ADVANCE_FRAME": {
      if (state.playMode !== "playing" || state.entries.length === 0) return state;
      const currentIndex = (state.currentIndex + 1) % state.entries.length;
      return { ...state, currentIndex };
    }

    case "SET_FRAME_MS":
      return { ...state, frameMs: Math.max(MIN_FRAME_MS, Math.min(MAX_FRAME_MS, action.ms)) };

    case "SET_MAX_ENTRIES": {
      const maxEntries = clampHistoryLimit(action.maxEntries);
      if (maxEntries === state.maxEntries) return state;
      // Shrinking the cap below the currently-held count evicts the
      // oldest entries immediately (not lazily on the next scan) -- the
      // Settings field's own note promises this.
      const { entries, currentIndex } = trimToCap(state.entries, state.currentIndex, maxEntries);
      return { ...state, entries, currentIndex, maxEntries };
    }

    case "JUMP_LATEST": {
      if (state.entries.length === 0) return { ...state, playMode: "live" };
      return { ...state, currentIndex: state.entries.length - 1, playMode: "live" };
    }

    case "JUMP_TO_TIME": {
      if (state.entries.length === 0) return state;
      // Nearest-by-timestamp, not nearest-by-index -- lets a caller (the
      // S09 unified timeline) drive selection from a real clicked/dragged
      // time position without knowing anything about indices itself.
      let bestIndex = 0;
      let bestDiffMillis = Infinity;
      for (let i = 0; i < state.entries.length; i++) {
        const diff = Math.abs(state.entries[i].startTimeMillis - action.millis);
        if (diff < bestDiffMillis) {
          bestDiffMillis = diff;
          bestIndex = i;
        }
      }
      // Same "deliberate look at history" treatment as STEP -- jumping to a
      // specific time is never mistaken for "live".
      return { ...state, currentIndex: bestIndex, playMode: "paused" };
    }
  }
}

export interface ScanHistory {
  entries: HistoryEntryMeta[];
  currentIndex: number;
  playMode: PlayMode;
  frameMs: number;
  pollEvent: PollEvent;
  /** The currently-selected history entry's raw bytes, or `undefined` if
   * history is empty or (should not normally happen within the cap) the
   * bytes were already evicted. A plain ref-backed lookup, never React
   * state -- see GLOBAL_CONTRACT's "large binary arrays do not live in
   * React state". */
  currentBytes: () => Uint8Array | undefined;
  previous: () => void;
  next: () => void;
  togglePlay: () => void;
  jumpToLatest: () => void;
  /** Select whichever held entry's `startTimeMillis` is closest to
   * `millis` (a real timestamp, not an index) -- added for the S09 unified
   * timeline so it can drive playback from a clicked/dragged real-time
   * position while reusing all of this hook's existing index-based state.
   * No-op if history is empty. Same "paused" treatment as `previous`/
   * `next`: a deliberate jump is never mistaken for "live". */
  jumpToNearestByTimestamp: (millis: number) => void;
  setFrameMs: (ms: number) => void;
  /** How old the currently-displayed frame's own volume start time is,
   * right now -- ticks live via a coarse internal timer (see
   * `STALE_CHECK_INTERVAL_MS`) so it keeps advancing even when nothing else
   * about the selection changes. `null` when nothing is selected. */
  currentVolumeAgeMillis: number | null;
  /** `true` only in `"live"` mode, once {@link currentVolumeAgeMillis}
   * exceeds {@link STALE_LIVE_AFTER_MS} -- i.e. exactly the case a "LIVE"
   * badge must stop reading as fresh. Always `false` in `"paused"`/
   * `"playing"` mode: a deliberate history review is never mislabeled as a
   * stuck live feed. */
  isLiveStale: boolean;
}

/**
 * Wraps `useScanPoller`'s live-download loop with a small, capped,
 * in-memory history of recently-downloaded scans (see `MAX_HISTORY_SCANS`)
 * and previous/next/play/pause/latest playback over it.
 *
 * Raw volume bytes are kept in a plain `Map` ref (`bytesRef`), never React
 * state -- only small per-scan metadata (`HistoryEntryMeta`: a key and a
 * timestamp) becomes state, driving the reducer above. A separate effect
 * prunes `bytesRef` to match whatever the reducer's `entries` currently
 * holds, so eviction is "delete the byte array," never "re-download it."
 *
 * Live polling never stops in the background regardless of `playMode`
 * (`useScanPoller` runs unconditionally below) -- only whether a newly
 * downloaded scan *changes what's displayed* depends on `playMode` (see
 * `historyReducer`'s `SCAN_ADDED` case).
 *
 * # S10 Phase 3: staleness, not just poll errors
 *
 * `useScanPoller`'s `pollEvent` already surfaces a *transient* poll failure
 * (a top-bar "error: ..." line), but live-verifying against a real blocked
 * feed (the NEXRAD bucket host unreachable while everything else keeps
 * working) turned up a real gap: the playback status's "live" badge kept
 * reading exactly as it does when everything is healthy, with no signal at
 * all that the displayed volume was actually minutes old -- and a feed that
 * silently stops finding a newer key (no HTTP error at all, just
 * `"up-to-date"` forever) would not even get the transient error text. This
 * hook now separately tracks how old the *displayed* live volume itself is
 * ({@link STALE_LIVE_AFTER_MS}) so the UI can flag that directly, regardless
 * of whether the underlying poll is erroring or just never finding anything
 * new -- see `currentVolumeAgeMillis`/`isLiveStale` below and
 * `PlaybackControls`'s use of them.
 */
export function useScanHistory(icao: string, maxEntries: number = MAX_HISTORY_SCANS): ScanHistory {
  const [state, dispatch] = useReducer(historyReducer, maxEntries, initialState);
  const bytesRef = useRef<Map<string, Uint8Array>>(new Map());

  // Reset history (metadata + cached bytes) whenever the selected site
  // changes -- an old site's scans are meaningless once switched away.
  useEffect(() => {
    dispatch({ type: "RESET" });
    bytesRef.current.clear();
  }, [icao]);

  // S10 Phase 2: live-adjust the cache cap when the Settings panel's value
  // changes, without needing a full site reset -- trims from the oldest
  // end immediately if the new cap is smaller (see `historyReducer`'s
  // `SET_MAX_ENTRIES` case / `trimToCap`). A no-op dispatch (same value)
  // short-circuits inside the reducer, so this is cheap on every render
  // where `maxEntries` hasn't actually changed.
  useEffect(() => {
    dispatch({ type: "SET_MAX_ENTRIES", maxEntries });
  }, [maxEntries]);

  const onVolumeBytes = useCallback((bytes: Uint8Array, volume: DiscoveredVolume) => {
    bytesRef.current.set(volume.key, bytes);
    dispatch({ type: "SCAN_ADDED", key: volume.key, startTimeMillis: volume.startTimeMillis });
  }, []);

  const pollEvent = useScanPoller(icao, onVolumeBytes);

  // Prune `bytesRef` to match `entries` after every metadata change --
  // catches both site-reset (entries briefly empty) and per-scan FIFO
  // eviction (the oldest key drops out of `entries`).
  useEffect(() => {
    const liveKeys = new Set(state.entries.map((e) => e.key));
    for (const key of bytesRef.current.keys()) {
      if (!liveKeys.has(key)) bytesRef.current.delete(key);
    }
  }, [state.entries]);

  // Animation timer: only runs while `playMode === "playing"`, ticking
  // every `frameMs`. Advancing never touches the network -- it only moves
  // `currentIndex` through the already-held `entries`/`bytesRef`.
  useEffect(() => {
    if (state.playMode !== "playing") return;
    const timer = setInterval(() => dispatch({ type: "ADVANCE_FRAME" }), state.frameMs);
    return () => clearInterval(timer);
  }, [state.playMode, state.frameMs]);

  // Live-ticking clock backing `currentVolumeAgeMillis`/`isLiveStale` --
  // deliberately a plain `Date.now()` sample on a coarse interval, not
  // derived from any poll event, so staleness is caught whether the
  // underlying poll is erroring, or just never finding a newer key.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), STALE_CHECK_INTERVAL_MS);
    return () => clearInterval(timer);
  }, []);

  const currentEntryForAge = state.entries[state.currentIndex];
  const currentVolumeAgeMillis = currentEntryForAge ? now - currentEntryForAge.startTimeMillis : null;
  const isLiveStale =
    state.playMode === "live" && currentVolumeAgeMillis !== null && currentVolumeAgeMillis > STALE_LIVE_AFTER_MS;

  const currentBytes = useCallback(() => {
    const entry = state.entries[state.currentIndex];
    return entry ? bytesRef.current.get(entry.key) : undefined;
    // Deliberately not memoized on `state` beyond closing over it via the
    // render this callback was created in -- callers invoke this
    // imperatively right after reading `currentIndex`, never store it.
  }, [state.entries, state.currentIndex]);

  const previous = useCallback(() => dispatch({ type: "STEP", delta: -1 }), []);
  const next = useCallback(() => dispatch({ type: "STEP", delta: 1 }), []);
  const togglePlay = useCallback(() => dispatch({ type: "TOGGLE_PLAY" }), []);
  const jumpToLatest = useCallback(() => dispatch({ type: "JUMP_LATEST" }), []);
  const jumpToNearestByTimestamp = useCallback(
    (millis: number) => dispatch({ type: "JUMP_TO_TIME", millis }),
    [],
  );
  const setFrameMs = useCallback((ms: number) => dispatch({ type: "SET_FRAME_MS", ms }), []);

  return {
    entries: state.entries,
    currentIndex: state.currentIndex,
    playMode: state.playMode,
    frameMs: state.frameMs,
    pollEvent,
    currentBytes,
    previous,
    next,
    togglePlay,
    jumpToLatest,
    jumpToNearestByTimestamp,
    setFrameMs,
    currentVolumeAgeMillis,
    isLiveStale,
  };
}
