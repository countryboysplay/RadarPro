import { useEffect, useRef, useState } from "react";
import { discoverLatest, downloadVolume } from "../nexrad/bucket";
import type { DiscoveredVolume } from "../nexrad/keys";

/**
 * Background poll interval. WSR-88D volumes complete roughly every 4-10
 * minutes depending on the active Volume Coverage Pattern, so polling much
 * more often than every 30-60s would just spend bandwidth re-listing a
 * directory that has usually not changed; this stage does not need
 * sub-minute precision on "a new scan just landed." 45s matches
 * `crates/radar-cache/src/update_loop.rs`'s `DEFAULT_POLL_INTERVAL` and its
 * documented reasoning -- not required to match exactly, just reasonable
 * and in the same 30-60s range.
 */
export const POLL_INTERVAL_MS = 45_000;

export type PollEvent =
  | { type: "checking" }
  | { type: "downloaded"; volume: DiscoveredVolume }
  | { type: "up-to-date"; volume: DiscoveredVolume }
  | { type: "none-available" }
  | { type: "error"; message: string };

/**
 * Runs a background loop that checks the given site for a new latest scan
 * every {@link POLL_INTERVAL_MS}, downloads it when it changes, and invokes
 * `onVolumeBytes` with the raw bytes (never stored in React state here --
 * see `useRadarRenderer`'s docs on GLOBAL_CONTRACT's "large binary arrays
 * do not live in React state" rule).
 *
 * Checks immediately on mount/site-change, then on the interval.
 * Switching `icao` (or unmounting) cancels the in-flight request (via
 * `AbortController`) and clears the interval before starting a new loop --
 * the same "cancel old, start new, never run two in parallel" discipline
 * `radar-cache/src/update_loop.rs` documents, implemented here with a plain
 * `useEffect` cleanup instead of a `CancellationToken`.
 */
export function useScanPoller(
  icao: string,
  onVolumeBytes: (bytes: Uint8Array, volume: DiscoveredVolume) => void,
): PollEvent {
  const [event, setEvent] = useState<PollEvent>({ type: "checking" });
  // Ref so the effect below does not need `onVolumeBytes` in its dependency
  // array (it would otherwise restart the poll loop on every render if the
  // caller passes a fresh closure each time).
  const onVolumeBytesRef = useRef(onVolumeBytes);
  onVolumeBytesRef.current = onVolumeBytes;

  useEffect(() => {
    const controller = new AbortController();
    let cancelled = false;
    let inFlight = false;
    let lastDownloadedKey: string | null = null;

    async function checkOnce() {
      if (inFlight) return; // never overlap a slow poll with the next tick
      inFlight = true;
      setEvent({ type: "checking" });
      try {
        const latest = await discoverLatest(icao, controller.signal);
        if (cancelled) return;

        if (!latest) {
          setEvent({ type: "none-available" });
          return;
        }
        if (latest.key === lastDownloadedKey) {
          setEvent({ type: "up-to-date", volume: latest });
          return;
        }

        const bytes = await downloadVolume(latest.key, controller.signal);
        if (cancelled) return;
        lastDownloadedKey = latest.key;
        onVolumeBytesRef.current(bytes, latest);
        setEvent({ type: "downloaded", volume: latest });
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
  }, [icao]);

  return event;
}
