import { useEffect, useMemo, useState } from "react";
import type { PlacefileLayer } from "../placefiles/usePlacefiles";
import { fetchMds, fetchOutlook, fetchWatches, type MesoscaleDiscussion, type OutlookKind, type SpcOutlookKind, type WpcKind } from "./spc";
import { fetchWpc } from "./wpc";
import { fetchLsrs } from "./lsr";

/** Watches and MDs come out every few minutes on an active day. */
const PRODUCT_POLL_MS = 2 * 60_000;
/** SPC outlooks are reissued a few times a day. */
const OUTLOOK_POLL_MS = 10 * 60_000;
/** WPC reissues every ~6 h, and its QPF is up to 1.2 MB: spare a hotspot. */
const WPC_POLL_MS = 60 * 60_000;

/** Polls `load` while `enabled`, keeping the last good result when a refresh fails. */
export function usePolled<T>(load: ((signal: AbortSignal) => Promise<T>) | null, key: string, pollMs: number | null) {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    // Switched off: clear. A new archive step keeps the old outlines until its own arrive.
    if (!load) {
      setValue(null);
      setError(null);
      return;
    }
    const controller = new AbortController();
    const poll = () =>
      load(controller.signal).then(
        (v) => {
          if (controller.signal.aborted) return;
          setValue(v);
          setError(null);
        },
        (err) => {
          if (!controller.signal.aborted) setError(err instanceof Error ? err.message : String(err));
        },
      );
    poll();
    const timer = pollMs === null ? undefined : setInterval(poll, pollMs);
    return () => {
      controller.abort();
      clearInterval(timer);
    };
    // `key` stands for `load`'s inputs, so a new closure each render doesn't refetch.
  }, [key]);
  return { value, error };
}

/**
 * SPC layers, bottom to top: an SPC or WPC outlook, watches, mesoscale
 * discussions; plus NWS storm reports. With `archiveTimeMs` set, the watches and MDs in effect then
 * (fetched once) and no outlook (today's would mislead on a past scan).
 */
export function useSpc(outlook: OutlookKind, showWatches: boolean, showMds: boolean, showReports: boolean, archiveTimeMs: number | null) {
  const live = archiveTimeMs === null;
  // Products change slowly; ten-minute steps keep loop playback from refetching every frame.
  const at = live ? null : Math.floor(archiveTimeMs / 600_000) * 600_000;
  const outlookOn = outlook !== "off" && live;
  const isWpc = outlook.startsWith("ero") || outlook.startsWith("qpf");
  const o = usePolled(outlookOn ? (s) => (isWpc ? fetchWpc(outlook as WpcKind, s) : fetchOutlook(outlook as SpcOutlookKind, s)) : null, outlookOn ? outlook : "", isWpc ? WPC_POLL_MS : OUTLOOK_POLL_MS);
  const w = usePolled(showWatches ? (s) => fetchWatches(at, s) : null, showWatches ? `w${at}` : "", live ? PRODUCT_POLL_MS : null);
  const m = usePolled(showMds ? (s) => fetchMds(at, s) : null, showMds ? `m${at}` : "", live ? PRODUCT_POLL_MS : null);
  const r = usePolled(showReports ? (s) => fetchLsrs(at, s) : null, showReports ? `r${at}` : "", live ? PRODUCT_POLL_MS : null);

  const layers = useMemo(() => [o.value, w.value, m.value?.layer].filter((l): l is PlacefileLayer => !!l), [o.value, w.value, m.value]);
  const mds: MesoscaleDiscussion[] = m.value?.mds ?? [];
  const watchCount = w.value ? w.value.file.items.filter((i) => i.kind === "polygon").length : 0;
  const reports = r.value;
  return {
    layers,
    mds,
    watchCount,
    /** Storm reports: points, drawn above warnings with the placefiles (not under them like `layers`). */
    reports,
    reportCount: reports?.file.items.length ?? 0,
    reportsError: r.error,
    error: o.error ?? w.error ?? m.error,
    outlookHiddenInArchive: outlook !== "off" && !live,
  };
}
