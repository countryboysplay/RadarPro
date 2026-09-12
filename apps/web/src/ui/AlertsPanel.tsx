import type { AlertPollStatus } from "../alerts/useAlertPoller";
import type { HeldAlert } from "../alerts/types";
import { SEVERITY_COLORS } from "../alerts/types";

export interface AlertsPanelProps {
  status: AlertPollStatus;
  error: string | null;
  alerts: HeldAlert[];
  selectedKey: string | null;
  onSelect: (key: string) => void;
  lastPolledAt: number | null;
  heldCount: number;
}

function describeStatus(status: AlertPollStatus, error: string | null, lastPolledAt: number | null): string {
  if (status === "loading") return "loading NWS alerts…";
  if (status === "error") return `poll error: ${error ?? "unknown"} (showing last-known alerts)`;
  if (lastPolledAt) return `updated ${new Date(lastPolledAt).toLocaleTimeString()}`;
  return "up to date";
}

/**
 * S06: live list of currently-active NWS alerts (`weather-alerts`'
 * `AlertStore` active set, via `useAlertPoller`). Clicking a row selects it
 * for `AlertDetail`; the row set itself updates live as `AlertChange`
 * events arrive (new/updated/cancelled/expired) -- this component has no
 * lifecycle logic of its own, it only ever renders `useAlertPoller`'s
 * current `alerts` array.
 */
export function AlertsPanel({
  status,
  error,
  alerts,
  selectedKey,
  onSelect,
  lastPolledAt,
  heldCount,
}: AlertsPanelProps) {
  return (
    <div className="alerts-panel">
      <div className="alerts-panel-title">NWS Alerts</div>
      <div className={`alerts-panel-status alerts-panel-status-${status}`}>
        {describeStatus(status, error, lastPolledAt)}
      </div>

      {alerts.length === 0 ? (
        <div className="alerts-panel-empty">
          {status === "loading" ? "checking for active alerts…" : "no active alerts"}
        </div>
      ) : (
        <ul className="alerts-list">
          {alerts.map(({ key, alert }) => (
            <li key={key}>
              <button
                type="button"
                className={`alert-row${key === selectedKey ? " alert-row-selected" : ""}`}
                onClick={() => onSelect(key)}
              >
                <span
                  className="alert-severity-dot"
                  style={{ background: SEVERITY_COLORS[alert.severity] }}
                  aria-hidden="true"
                />
                <span className="alert-row-text">
                  <span className="alert-row-event">{alert.event}</span>
                  {alert.headline && <span className="alert-row-headline">{alert.headline}</span>}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="alerts-panel-footer">
        {heldCount} held · Source: National Weather Service (api.weather.gov)
      </div>
    </div>
  );
}
