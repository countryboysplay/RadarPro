import { useEffect, useMemo, useState } from "react";
import { fetchActiveAlerts } from "./nwsApi";
import { AlertStoreHandle, ensureAlertsWasmModuleLoaded } from "./wasmModule";
import { severityRank, type AlertChangeJson, type AlertFeatureCollection, type AlertJson, type HeldAlert } from "./types";

/**
 * Network poll interval against `/alerts/active`. NWS's feed updates as new
 * products are issued, not on any fixed cadence (there is no "correct"
 * interval to derive this from) -- 60s sits in the same 30-90s range as
 * `useScanPoller`'s `POLL_INTERVAL_MS` (45s) and is explicitly the cadence
 * `docs/adr/0010-nws-alert-lifecycle-reconciliation.md` assumed
 * (`AlertStore`'s absence-based safety net was reasoned about at "~60-120s
 * poll cadence, well under 10 minutes" -- see that ADR's Decision section).
 * Going dramatically faster/slower than this would need re-reading that
 * ADR's absence-limit reasoning, per this task's own instructions.
 */
export const ALERT_POLL_INTERVAL_MS = 60_000;

/**
 * A cheap, network-free local tick that calls `AlertStoreHandle.expireStale`
 * between network polls, so an alert crossing its own `expires` timestamp
 * disappears from the active set (and the change stream) promptly rather
 * than lingering until the next network round trip up to
 * `ALERT_POLL_INTERVAL_MS` later -- `AlertStore::expire_stale`'s own docs
 * explicitly invite calling it this way ("safe and cheap to call frequently
 * between polls"). 10s is frequent enough that expiry feels immediate on a
 * human timescale without doing meaningful work every frame.
 */
const EXPIRE_STALE_TICK_MS = 10_000;

export type AlertPollStatus = "loading" | "ok" | "error";

export interface AlertPollerState {
  /** `"loading"` until the wasm module is ready and the first poll has
   * resolved; `"ok"`/`"error"` reflect only the most recent network poll
   * (a transient error does not clear previously-known alerts -- the store
   * itself decides removal, this hook never second-guesses it). */
  status: AlertPollStatus;
  error: string | null;
  /** Every alert `weather-alerts`'s store currently considers active,
   * keyed by its stable `AlertKey` -- for the list/details UI. Sorted by
   * severity (most severe first) then soonest-issued. */
  activeAlerts: HeldAlert[];
  /** The same active set as a GeoJSON `FeatureCollection`, straight from
   * `AlertStoreHandle.activeAlertsGeoJson` (id = stable key), for the map
   * layer. */
  geojson: AlertFeatureCollection | null;
  /** `AlertStore::held_count` -- includes alerts not yet past their own
   * `expires` at the moment of the last mutating call, purely a debug/HUD
   * figure. */
  heldCount: number;
  /** Wall-clock time (epoch ms) of the last successful network poll. */
  lastPolledAt: number | null;
}

const EMPTY_GEOJSON: AlertFeatureCollection = { type: "FeatureCollection", features: [] };

function parseChanges(json: string): AlertChangeJson[] {
  return JSON.parse(json) as AlertChangeJson[];
}

function parseGeoJson(json: string): AlertFeatureCollection {
  return JSON.parse(json) as AlertFeatureCollection;
}

/** Apply a batch of `AlertChange` events to a held-alert map, mutating
 * nothing about *when* an alert is added/updated/removed beyond what the
 * store itself already decided -- see `GLOBAL_CONTRACT.md`: never
 * reimplement or second-guess `AlertStore`'s lifecycle decisions here. */
function applyChanges(held: Map<string, AlertJson>, changes: AlertChangeJson[]): Map<string, AlertJson> {
  if (changes.length === 0) return held;
  const next = new Map(held);
  for (const change of changes) {
    switch (change.type) {
      case "New":
      case "Updated":
        next.set(change.key, change.alert);
        break;
      case "Cancelled":
      case "Expired":
        next.delete(change.key);
        break;
    }
  }
  return next;
}

/**
 * Background poll loop for NWS `/alerts/active`, feeding each poll into a
 * `weather-alerts` `AlertStoreHandle` (wasm) and exposing its current
 * active set / GeoJSON for the map + a details-panel/list UI. Analogous to
 * `useScanPoller` (same `AbortController` cancellation, cleanup-on-unmount
 * discipline) but not built on it directly -- this polls a wholly
 * different provider (`api.weather.gov`, not the NEXRAD S3 bucket) with a
 * wholly different response shape and lifecycle model.
 *
 * # Poll scope: nationwide, not area/point-filtered
 *
 * This hook fetches the plain `/alerts/active` endpoint with no
 * `?area=`/`?point=` filter -- every currently-active alert in the US,
 * regardless of the currently-selected radar site. Tradeoff, documented
 * per this task's brief:
 *   - **Nationwide (chosen)**: simpler (one query shape, no dependency on
 *     the selected site or map viewport), and correctly shows every alert
 *     visible on the map as the user pans/zooms/changes sites -- an alert
 *     near a site that is not currently selected still renders the moment
 *     the map is panned there, with no re-fetch needed.
 *   - **Area/point-filtered** (`?area=<state>` or `?point=<lat>,<lon>`
 *     near the selected site): smaller payload (this feed is a few hundred
 *     KB nationwide, not huge, but a filtered query is smaller still), at
 *     the cost that panning the map away from the queried area/site could
 *     show a region with no alerts even though the *visible* map area
 *     genuinely has active ones -- the filter would need to be re-derived
 *     from the map viewport, not just the selected radar site, to avoid
 *     that gap.
 * Nationwide is a reasonable default for this stage; a future refinement
 * could switch to a viewport-derived `?point=`/bounding-query as the
 * overlay's real use grows to care about payload size.
 *
 * # Never second-guessing `AlertStore`
 *
 * This hook's only jobs are: fetch, hand the raw body to
 * `AlertStoreHandle.ingestPoll`/`expireStale`, and apply the
 * `AlertChange` events it returns to plain React state via `applyChanges`
 * (add-or-replace on New/Updated, remove on Cancelled/Expired) -- exactly
 * the store's own decision, on exactly the store's own timing. No
 * additional "if it's been missing a while, hide it anyway" logic is
 * layered on top here; that would risk reintroducing the poll-race bug
 * `docs/adr/0010-...` was written to prevent.
 */
export function useAlertPoller(): AlertPollerState {
  const [status, setStatus] = useState<AlertPollStatus>("loading");
  const [error, setError] = useState<string | null>(null);
  const [held, setHeld] = useState<Map<string, AlertJson>>(new Map());
  const [geojson, setGeojson] = useState<AlertFeatureCollection | null>(null);
  const [heldCount, setHeldCount] = useState(0);
  const [lastPolledAt, setLastPolledAt] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    let inFlight = false;
    let store: AlertStoreHandle | null = null;

    function refreshGeojson() {
      if (!store) return;
      setGeojson(parseGeoJson(store.activeAlertsGeoJson(Date.now())));
      setHeldCount(store.heldCount());
    }

    async function pollOnce() {
      if (inFlight || !store) return;
      inFlight = true;
      try {
        const body = await fetchActiveAlerts(controller.signal);
        if (cancelled || !store) return;
        const changesJson = store.ingestPoll(body, Date.now());
        const changes = parseChanges(changesJson);
        setHeld((prev) => applyChanges(prev, changes));
        refreshGeojson();
        setStatus("ok");
        setError(null);
        setLastPolledAt(Date.now());
      } catch (err) {
        if (cancelled) return;
        if (err instanceof DOMException && err.name === "AbortError") return;
        setStatus("error");
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        inFlight = false;
      }
    }

    function expireStaleTick() {
      if (!store) return;
      const changesJson = store.expireStale(Date.now());
      const changes = parseChanges(changesJson);
      if (changes.length > 0) {
        setHeld((prev) => applyChanges(prev, changes));
        refreshGeojson();
      }
    }

    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let expireTimer: ReturnType<typeof setInterval> | null = null;

    ensureAlertsWasmModuleLoaded()
      .then(() => {
        if (cancelled) return;
        store = new AlertStoreHandle();
        refreshGeojson();
        void pollOnce();
        pollTimer = setInterval(() => void pollOnce(), ALERT_POLL_INTERVAL_MS);
        expireTimer = setInterval(expireStaleTick, EXPIRE_STALE_TICK_MS);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setStatus("error");
        setError(err instanceof Error ? err.message : String(err));
      });

    return () => {
      cancelled = true;
      controller.abort();
      if (pollTimer) clearInterval(pollTimer);
      if (expireTimer) clearInterval(expireTimer);
      store?.free();
    };
  }, []);

  const activeAlerts = useMemo<HeldAlert[]>(() => {
    const list = Array.from(held.entries()).map(([key, alert]) => ({ key, alert }));
    list.sort((a, b) => {
      const rankDiff = severityRank(a.alert.severity) - severityRank(b.alert.severity);
      if (rankDiff !== 0) return rankDiff;
      return b.alert.issued - a.alert.issued;
    });
    return list;
  }, [held]);

  return {
    status,
    error,
    activeAlerts,
    geojson: geojson ?? EMPTY_GEOJSON,
    heldCount,
    lastPolledAt,
  };
}
