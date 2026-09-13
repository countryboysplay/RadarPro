// S09d Part B: sidebar panel for the Rainbow Nowcast API (KB §6) -- a
// point-based, minute-by-minute precipitation forecast for the next 4
// hours. Distinct from both this app's live NEXRAD radar and its GEFS/HRRR
// "Forecast" panel -- labeled "Rainbow Nowcast" throughout (Global
// Contract: these three must stay visually distinguishable).
//
// Always rendered, even when unavailable (no key / not on desktop) --
// same "visible but disabled, with a clear reason" convention
// `RainbowToggle` established, reusing its `.rainbow-toggle-status*`
// classes for the status line so all Rainbow panels read as one family.
import { useState } from "react";
import type { RainbowNowcastState } from "../rainbow/useRainbowNowcast";

export interface RainbowNowcastPanelProps {
  nowcast: RainbowNowcastState;
  /** Whether this panel's "Pick on map" mode is the one currently armed
   * (S09d follow-up: "click the map to set the point") -- lifted to
   * `App.tsx` since at most one of {nowcast, weather} can be armed at a
   * time, and only `App.tsx` sees the actual map click (via `MapView`'s
   * `onPointPick`). This panel never reaches into MapLibre itself. */
  pickModeActive: boolean;
  /** Arm/disarm this panel's pick mode -- toggling while already armed
   * cancels it, giving the user a way out without clicking the map. */
  onTogglePickMode: () => void;
}

/** Selectable "how far in the past should the nowcast start" offsets (KB
 * §6.1: up to 30 minutes back, 1-minute aligned). Computing the actual
 * epoch timestamp from a fixed offset at request time (rather than letting
 * the user type an arbitrary epoch value) makes an invalid combination
 * structurally impossible, per this stage's explicit UI decision to keep
 * every picker in this feature a plain dropdown. */
const START_OFFSET_MINUTES = [0, 5, 10, 15, 20, 25, 30] as const;

function offsetLabel(minutes: number): string {
  return minutes === 0 ? "Now" : `-${minutes} min`;
}

/** `epoch seconds -> minute-aligned start_timestamp` for a given
 * minutes-ago offset, or `null` for "now" (which omits the param entirely,
 * KB §6.1's documented default). */
function startTimestampForOffset(minutesAgo: number): number | null {
  if (minutesAgo === 0) return null;
  const nowSeconds = Math.floor(Date.now() / 1000);
  const aligned = nowSeconds - (nowSeconds % 60);
  return aligned - minutesAgo * 60;
}

function formatClockTime(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function describeNowcastStatus(nowcast: RainbowNowcastState): { text: string; kind: "unconfigured" | "error" | "ready" | "" } | null {
  if (!nowcast.desktopAvailable) {
    return { text: "desktop app required -- api.rainbow.ai blocks direct browser requests (CORS)", kind: "unconfigured" };
  }
  if (!nowcast.configured) {
    return { text: "not configured -- add your API key in Settings to enable", kind: "unconfigured" };
  }
  if (nowcast.phase === "loading") return { text: "fetching nowcast…", kind: "" };
  if (nowcast.phase === "error") return { text: `unavailable: ${nowcast.error ?? "unknown error"}`, kind: "error" };
  if (nowcast.phase === "ready") {
    return {
      text: nowcast.usedGlobalFallback
        ? "showing global-coverage nowcast (no regional data at this point)"
        : "showing regional nowcast",
      kind: "ready",
    };
  }
  return null;
}

export function RainbowNowcastPanel({ nowcast, pickModeActive, onTogglePickMode }: RainbowNowcastPanelProps) {
  const { point, setLon, setLat } = nowcast;
  const [startOffsetMinutes, setStartOffsetMinutes] = useState(0);

  const available = nowcast.desktopAvailable && nowcast.configured;
  const status = describeNowcastStatus(nowcast);

  const handleGetNowcast = () => {
    nowcast.fetchNowcast(point, startTimestampForOffset(startOffsetMinutes));
  };

  return (
    <div className="rainbow-panel">
      <div className="rainbow-panel-point">
        <label className="rainbow-panel-field">
          <span>Lon</span>
          <input
            type="number"
            step="0.0001"
            value={point.lon}
            disabled={!available}
            onChange={(e) => setLon(Number(e.target.value))}
          />
        </label>
        <label className="rainbow-panel-field">
          <span>Lat</span>
          <input
            type="number"
            step="0.0001"
            value={point.lat}
            disabled={!available}
            onChange={(e) => setLat(Number(e.target.value))}
          />
        </label>
      </div>

      <button
        type="button"
        className="rainbow-panel-button rainbow-panel-button-secondary"
        disabled={!available}
        aria-pressed={pickModeActive}
        onClick={onTogglePickMode}
      >
        {pickModeActive ? "Click the map… (cancel)" : "Pick on map"}
      </button>

      <label className="rainbow-panel-field">
        <span>Start</span>
        <select value={startOffsetMinutes} disabled={!available} onChange={(e) => setStartOffsetMinutes(Number(e.target.value))}>
          {START_OFFSET_MINUTES.map((minutes) => (
            <option key={minutes} value={minutes}>
              {offsetLabel(minutes)}
            </option>
          ))}
        </select>
      </label>

      <button type="button" className="rainbow-panel-button" disabled={!available || nowcast.phase === "loading"} onClick={handleGetNowcast}>
        Get Nowcast
      </button>

      {status && <div className={`rainbow-toggle-status${status.kind ? ` rainbow-toggle-status-${status.kind}` : ""}`}>{status.text}</div>}
    </div>
  );
}

/** UI polish pass: whether {@link RainbowNowcastResults} has anything to
 * show for `nowcast` right now -- a pending/ready/error result, never the
 * initial pre-fetch `"idle"` phase. Drives whether this panel's entry
 * exists in the right-dock (`RightPanel`) at all -- that dock must never
 * reserve empty screen space before the user has ever pressed "Get
 * Nowcast". */
export function nowcastHasContent(nowcast: RainbowNowcastState): boolean {
  return nowcast.phase !== "idle";
}

/**
 * The RESULTS half of the Rainbow Nowcast panel -- split out from
 * {@link RainbowNowcastPanel} (which keeps only the inputs/button/status
 * line in the left sidebar) so the table itself renders in the right-docked
 * `RightPanel` instead, where it has room to breathe. See this task's
 * report for the "cramped in the narrow sidebar column" user-testing
 * feedback this responds to.
 */
export function RainbowNowcastResults({ nowcast }: { nowcast: RainbowNowcastState }) {
  if (nowcast.phase === "loading") {
    return <div className="rainbow-panel-summary">Fetching nowcast…</div>;
  }

  if (nowcast.phase === "error") {
    return <div className="rainbow-toggle-status rainbow-toggle-status-error">{nowcast.error ?? "unknown error"}</div>;
  }

  if (nowcast.phase !== "ready" || !nowcast.result) return null;

  return (
    <div className="rainbow-panel-results">
      <div className="rainbow-panel-summary">Max intensity: {nowcast.result.summary.intensity}</div>
      <div className="rainbow-panel-table-scroll">
        <table className="rainbow-panel-table">
          <thead>
            <tr>
              <th>Time</th>
              <th>Rate (mm/h)</th>
              <th>Type</th>
            </tr>
          </thead>
          <tbody>
            {nowcast.result.forecast.map((item) => (
              <tr key={item.timestampBegin}>
                <td>
                  {formatClockTime(item.timestampBegin)}–{formatClockTime(item.timestampEnd)}
                </td>
                <td>{item.precipRate.toFixed(2)}</td>
                <td>{item.precipType}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
