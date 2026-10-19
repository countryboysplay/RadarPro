import { useEffect, useRef, useState } from "react";
import { discoverLatest, downloadVolume } from "../nexrad/bucket";
import { ChunkedVolume, discoverNewestVolume, nextVolumeNumber, previousVolumeNumber } from "../nexrad/chunks";
import type { DiscoveredVolume } from "../nexrad/keys";

/**
 * Poll interval. The real-time feed adds a chunk about every 12 s, so 8 s
 * keeps new radials at most one poll behind without hammering the bucket.
 */
export const POLL_INTERVAL_MS = 8_000;

/** A volume with no new chunk for this long has stalled (radar down, feed gap): re-discover. */
const STALLED_AFTER_MS = 3 * 60_000;
/** A newest real-time volume older than this means the site isn't on the feed: use the archive. */
const FEED_STALE_AFTER_MS = 30 * 60_000;

export type PollEvent =
  | { type: "checking" }
  /** Real-time feed: `volume` is being received chunk by chunk (or just finished). */
  | { type: "receiving"; volume: DiscoveredVolume; complete: boolean }
  | { type: "downloaded"; volume: DiscoveredVolume }
  | { type: "up-to-date"; volume: DiscoveredVolume }
  | { type: "none-available" }
  | { type: "error"; message: string };

/**
 * Follows the given site live. Primary source is the real-time chunk feed
 * (`nexrad/chunks.ts`): the newest volume is re-assembled and handed to
 * `onVolumeBytes` each time new chunks land, so the lowest tilt shows
 * within seconds of the radar sending it, minutes before the archive has
 * the finished volume. Sites with no recent real-time data fall back to
 * the archive's latest finished volume.
 *
 * `onVolumeBytes` may be called repeatedly for the same volume key with
 * longer bytes; callers treat that as an in-place update. Raw bytes never
 * enter React state (GLOBAL_CONTRACT: large binary arrays do not live in
 * React state). Switching `icao` (or unmounting) cancels everything in
 * flight before a new loop starts.
 */
export function useScanPoller(
  icao: string,
  onVolumeBytes: (bytes: Uint8Array, volume: DiscoveredVolume) => void,
  /** `false` pauses polling entirely (archive playback). */
  enabled = true,
): PollEvent {
  const [event, setEvent] = useState<PollEvent>({ type: "checking" });
  // Ref so the effect below does not need `onVolumeBytes` in its dependency
  // array (it would otherwise restart the poll loop on every render if the
  // caller passes a fresh closure each time).
  const onVolumeBytesRef = useRef(onVolumeBytes);
  onVolumeBytesRef.current = onVolumeBytes;

  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    const { signal } = controller;
    let cancelled = false;
    let inFlight = false;
    let current: ChunkedVolume | null = null;
    let lastProgressAt = 0;
    let lastArchiveKey: string | null = null;

    const emit = (bytes: Uint8Array, volume: ChunkedVolume) => {
      if (cancelled) return; // a fetch that finished after a site switch belongs to the old site.
      const described = volume.describe();
      if (!described) return;
      onVolumeBytesRef.current(bytes, described);
      setEvent({ type: "receiving", volume: described, complete: volume.complete });
    };

    /** Start following the feed's newest volume; false if the site has no recent real-time data. */
    async function startFeed(): Promise<boolean> {
      const newest = await discoverNewestVolume(icao, signal);
      if (newest === null) return false;
      const volume = new ChunkedVolume(icao, newest);
      const bytes = await volume.poll(signal);
      const described = volume.describe();
      if (!bytes || !described || Date.now() - described.startTimeMillis > FEED_STALE_AFTER_MS) return false;
      // The volume before it has just finished and may not be in the archive yet: keep the loop gap-free.
      const previous = new ChunkedVolume(icao, previousVolumeNumber(newest));
      const previousBytes = await previous.poll(signal).catch(() => null);
      const previousStart = previous.describe()?.startTimeMillis ?? 0;
      if (previousBytes && previous.complete && previousStart < described.startTimeMillis) emit(previousBytes, previous);
      emit(bytes, volume);
      current = volume;
      lastProgressAt = Date.now();
      return true;
    }

    async function followFeed(volume: ChunkedVolume) {
      const bytes = await volume.poll(signal);
      if (bytes) {
        emit(bytes, volume);
        lastProgressAt = Date.now();
      }
      if (volume.complete) {
        // Folders are reused every 999 volumes, so the next number may still hold a days-old volume.
        const next = new ChunkedVolume(icao, nextVolumeNumber(volume.volume));
        const nextBytes = await next.poll(signal);
        const nextStart = next.describe()?.startTimeMillis ?? 0;
        if (nextBytes && nextStart > (volume.describe()?.startTimeMillis ?? 0)) {
          emit(nextBytes, next);
          current = next;
          lastProgressAt = Date.now();
        }
      }
      if (Date.now() - lastProgressAt > STALLED_AFTER_MS) current = null;
    }

    async function checkArchive() {
      const latest = await discoverLatest(icao, signal);
      if (cancelled) return;
      if (!latest) {
        setEvent({ type: "none-available" });
        return;
      }
      if (latest.key === lastArchiveKey) {
        setEvent({ type: "up-to-date", volume: latest });
        return;
      }
      const bytes = await downloadVolume(latest.key, signal);
      if (cancelled) return;
      lastArchiveKey = latest.key;
      onVolumeBytesRef.current(bytes, latest);
      setEvent({ type: "downloaded", volume: latest });
    }

    async function checkOnce() {
      if (inFlight) return; // never overlap a slow poll with the next tick
      inFlight = true;
      try {
        if (current) await followFeed(current);
        else if (!(await startFeed())) await checkArchive();
      } catch (err) {
        if (cancelled) return;
        if (err instanceof DOMException && err.name === "AbortError") return;
        setEvent({ type: "error", message: err instanceof Error ? err.message : String(err) });
      } finally {
        inFlight = false;
      }
    }

    void checkOnce();
    const timer = setInterval(() => void checkOnce(), POLL_INTERVAL_MS);

    return () => {
      cancelled = true;
      controller.abort();
      clearInterval(timer);
    };
  }, [icao, enabled]);

  return event;
}
