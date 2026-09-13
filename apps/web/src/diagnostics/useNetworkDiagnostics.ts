import { useEffect, useState } from "react";
import type { PollEvent } from "../scan/useScanPoller";
import type { AlertPollStatus } from "../alerts/useAlertPoller";

/**
 * S10 Phase 2 network diagnostics -- see
 * `Agent Context/context/stages/S10-desktop-beta.md`'s "network
 * diagnostics" item.
 *
 * Deliberately does not perform any network activity of its own (a
 * synthetic ping would just be one more thing that can fail independently
 * of what this app actually needs to work, and would need its own
 * interval/cancellation discipline to boot). Instead this derives "when did
 * the app last actually reach the network, and did it succeed" purely from
 * state `useScanPoller`/`useAlertPoller` already produce on every real poll
 * tick -- exactly the "you likely already have this" case this stage's
 * brief calls out.
 *
 * `navigator.onLine` is tracked too, but only as one input, not the whole
 * answer: it is well known to be unreliable on its own (a captive portal or
 * a DNS-only outage still reports `true`). Combining it with the pollers'
 * own real outcomes is what {@link overallNetworkStatus} is for.
 */
export interface PollOutcome {
  lastSuccessAt: number | null;
  lastSuccessDetail: string | null;
  lastFailureAt: number | null;
  lastFailureDetail: string | null;
}

const EMPTY_OUTCOME: PollOutcome = {
  lastSuccessAt: null,
  lastSuccessDetail: null,
  lastFailureAt: null,
  lastFailureDetail: null,
};

export interface NetworkDiagnostics {
  browserOnline: boolean;
  scan: PollOutcome;
  alerts: PollOutcome;
}

function describeScanEvent(event: PollEvent): string {
  switch (event.type) {
    case "downloaded":
      return `new volume: ${event.volume.key.split("/").pop()}`;
    case "up-to-date":
      return "up to date, no new volume";
    case "none-available":
      return "reached server, no volumes today yet";
    case "checking":
      return "checking…";
    case "error":
      return event.message;
  }
}

/**
 * Tracks `navigator.onLine` live via the standard `online`/`offline`
 * window events. `typeof navigator === "undefined"` guard is defensive
 * only (this hook always runs in a browser/webview context in practice).
 */
function useBrowserOnline(): boolean {
  const [online, setOnline] = useState(() => (typeof navigator === "undefined" ? true : navigator.onLine));
  useEffect(() => {
    const goOnline = () => setOnline(true);
    const goOffline = () => setOnline(false);
    window.addEventListener("online", goOnline);
    window.addEventListener("offline", goOffline);
    return () => {
      window.removeEventListener("online", goOnline);
      window.removeEventListener("offline", goOffline);
    };
  }, []);
  return online;
}

/**
 * Derives {@link NetworkDiagnostics} from the two existing pollers' live
 * state. Pass `history.pollEvent` (from `useScanHistory`, which threads
 * `useScanPoller`'s event straight through) and the relevant fields of
 * `useAlertPoller`'s return value straight from `App.tsx` -- this hook adds
 * no new subscriptions of its own to either poller.
 */
export function useNetworkDiagnostics(
  scanEvent: PollEvent,
  alertStatus: AlertPollStatus,
  alertError: string | null,
  alertLastPolledAt: number | null,
): NetworkDiagnostics {
  const browserOnline = useBrowserOnline();

  // `scanEvent` is a freshly-constructed object on every real poll tick
  // (see `useScanPoller`'s `setEvent` calls) -- including repeated
  // identical outcomes -- so this effect reliably re-fires once per tick,
  // "checking" (mid-flight, not a terminal outcome) aside.
  const [scan, setScan] = useState<PollOutcome>(EMPTY_OUTCOME);
  useEffect(() => {
    if (scanEvent.type === "checking") return;
    if (scanEvent.type === "error") {
      setScan((prev) => ({ ...prev, lastFailureAt: Date.now(), lastFailureDetail: scanEvent.message }));
    } else {
      setScan((prev) => ({ ...prev, lastSuccessAt: Date.now(), lastSuccessDetail: describeScanEvent(scanEvent) }));
    }
  }, [scanEvent]);

  // `alertLastPolledAt`/`alertError` are primitives, so back-to-back
  // *identical* failures (same message, `alertStatus` staying `"error"`)
  // will not bump `lastFailureAt` on every single tick the way the scan
  // side does -- a known, acceptable simplification for this proof of
  // concept: the displayed failure time reflects when the failure was
  // first (or most recently distinctly) observed, which is still an
  // honest answer, just not guaranteed to advance on a fully-identical
  // repeat.
  const [alerts, setAlerts] = useState<PollOutcome>(EMPTY_OUTCOME);
  useEffect(() => {
    if (alertStatus === "loading") return;
    if (alertStatus === "ok") {
      setAlerts((prev) => ({ ...prev, lastSuccessAt: alertLastPolledAt ?? Date.now(), lastSuccessDetail: "ok" }));
    } else {
      setAlerts((prev) => ({ ...prev, lastFailureAt: Date.now(), lastFailureDetail: alertError }));
    }
  }, [alertStatus, alertError, alertLastPolledAt]);

  return { browserOnline, scan, alerts };
}

export type OverallNetworkStatus = "online" | "degraded" | "offline" | "checking";

function latestOutcomeIsFailure(o: PollOutcome): boolean {
  if (o.lastFailureAt === null) return false;
  if (o.lastSuccessAt === null) return true;
  return o.lastFailureAt > o.lastSuccessAt;
}

/**
 * A single honest headline status, combining the browser's own signal with
 * the pollers' real outcomes: `navigator.onLine` reporting `true` is not
 * enough to call the app "online" if its own most recent live requests are
 * actually failing.
 */
export function overallNetworkStatus(d: NetworkDiagnostics): OverallNetworkStatus {
  if (!d.browserOnline) return "offline";
  if (latestOutcomeIsFailure(d.scan) || latestOutcomeIsFailure(d.alerts)) return "degraded";
  if (d.scan.lastSuccessAt === null && d.alerts.lastSuccessAt === null) return "checking";
  return "online";
}
