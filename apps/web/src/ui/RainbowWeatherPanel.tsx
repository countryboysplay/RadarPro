// S09d Part C: sidebar panel for the Rainbow Weather (Forecast) API (KB
// §7) -- Rainbow's own blended hourly/daily point forecast. This is a
// DIFFERENT product from this app's existing GEFS/HRRR "Forecast" panel
// (`forecast/ForecastPanel.tsx`, NWP model output) -- Global Contract
// requires these stay distinguishable, so every label here says "Rainbow
// forecast", never bare "Forecast".
//
// Same "always visible, disabled + labeled when unavailable" convention as
// `RainbowToggle`/`RainbowNowcastPanel`, reusing the same
// `.rainbow-toggle-status*` classes.
import { useState } from "react";
import type { RainbowWeatherState } from "../rainbow/useRainbowWeather";
import { formatFahrenheit } from "../rainbow/units";

export interface RainbowWeatherPanelProps {
  weather: RainbowWeatherState;
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

/** `null` = "don't request this timeline" (KB §7.1: "leaving one unset
 * returns `null` for it, that's fine, don't force both"). Both dropdowns
 * default to a request-something value so the common case is one click,
 * while still letting the user opt either one out entirely. */
const FORECAST_HOURS_OPTIONS = [null, 6, 12, 24, 48] as const;
const FORECAST_DAYS_OPTIONS = [null, 3, 5, 7] as const;

function hoursLabel(hours: number | null): string {
  return hours === null ? "Off" : `${hours} h`;
}
function daysLabel(days: number | null): string {
  return days === null ? "Off" : `${days} d`;
}

function formatClockTime(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
function formatDayLabel(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
}

function describeWeatherStatus(weather: RainbowWeatherState): { text: string; kind: "unconfigured" | "error" | "ready" | "" } | null {
  if (!weather.desktopAvailable) {
    return { text: "desktop app required -- api.rainbow.ai blocks direct browser requests (CORS)", kind: "unconfigured" };
  }
  if (!weather.configured) {
    return { text: "not configured -- add your API key in Settings to enable", kind: "unconfigured" };
  }
  if (weather.phase === "loading") return { text: "fetching Rainbow forecast…", kind: "" };
  if (weather.phase === "error") return { text: `unavailable: ${weather.error ?? "unknown error"}`, kind: "error" };
  if (weather.phase === "ready") return { text: "showing Rainbow forecast", kind: "ready" };
  return null;
}

export function RainbowWeatherPanel({ weather, pickModeActive, onTogglePickMode }: RainbowWeatherPanelProps) {
  const { point, setLon, setLat } = weather;
  const [forecastHours, setForecastHours] = useState<number | null>(24);
  const [forecastDays, setForecastDays] = useState<number | null>(null);
  const [dayStartHour, setDayStartHour] = useState(6);

  const available = weather.desktopAvailable && weather.configured;
  const status = describeWeatherStatus(weather);
  const nothingSelected = forecastHours === null && forecastDays === null;

  const handleGetForecast = () => {
    weather.fetchWeather(point, { forecastHours, forecastDays, dayStartHour });
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
        <span>Hourly</span>
        <select value={forecastHours ?? "off"} disabled={!available} onChange={(e) => setForecastHours(e.target.value === "off" ? null : Number(e.target.value))}>
          {FORECAST_HOURS_OPTIONS.map((hours) => (
            <option key={hours ?? "off"} value={hours ?? "off"}>
              {hoursLabel(hours)}
            </option>
          ))}
        </select>
      </label>

      <label className="rainbow-panel-field">
        <span>Daily</span>
        <select value={forecastDays ?? "off"} disabled={!available} onChange={(e) => setForecastDays(e.target.value === "off" ? null : Number(e.target.value))}>
          {FORECAST_DAYS_OPTIONS.map((days) => (
            <option key={days ?? "off"} value={days ?? "off"}>
              {daysLabel(days)}
            </option>
          ))}
        </select>
      </label>

      <label className="rainbow-panel-field">
        <span>Day starts at</span>
        <input
          type="number"
          min={0}
          max={23}
          value={dayStartHour}
          disabled={!available}
          onChange={(e) => setDayStartHour(Math.min(23, Math.max(0, Number(e.target.value))))}
        />
      </label>

      <button type="button" className="rainbow-panel-button" disabled={!available || nothingSelected || weather.phase === "loading"} onClick={handleGetForecast}>
        Get Forecast
      </button>

      {status && <div className={`rainbow-toggle-status${status.kind ? ` rainbow-toggle-status-${status.kind}` : ""}`}>{status.text}</div>}
    </div>
  );
}

/** UI polish pass: whether {@link RainbowWeatherResults} has anything to
 * show for `weather` right now -- a pending/ready/error result, never the
 * initial pre-fetch `"idle"` phase. Drives whether this panel's entry
 * exists in the right-dock (`RightPanel`) at all. */
export function weatherHasContent(weather: RainbowWeatherState): boolean {
  return weather.phase !== "idle";
}

/**
 * The RESULTS half of the Rainbow Forecast panel -- split out from
 * {@link RainbowWeatherPanel} (which keeps only the inputs/button/status
 * line in the left sidebar) so the hourly/daily tables render in the
 * right-docked `RightPanel` instead, where they have room to breathe.
 *
 * Temperatures are converted Celsius -> Fahrenheit for display
 * (`../rainbow/units.ts`'s `formatFahrenheit`) -- the Rainbow Weather API
 * is always metric (KB §7: "No unit-selection parameter is documented;
 * convert client-side"), so every `temperature`/`feelsLikeTemperature`/
 * `temperatureDewPoint`/`temperatureMin`/`temperatureMax` field is shown as
 * "°F", never the response's own `units.temperature` ("celsius") string.
 * This is temperature-only: precip amount, precip chance, wind, UV, etc.
 * all keep using their own reported `units.*` label, unconverted.
 */
export function RainbowWeatherResults({ weather }: { weather: RainbowWeatherState }) {
  if (weather.phase === "loading") {
    return <div className="rainbow-panel-summary">Fetching Rainbow forecast…</div>;
  }

  if (weather.phase === "error") {
    return <div className="rainbow-toggle-status rainbow-toggle-status-error">{weather.error ?? "unknown error"}</div>;
  }

  if (weather.phase !== "ready" || !weather.result) return null;

  const { units } = weather.result;
  const hourly = weather.result.timelines.hourly;
  const daily = weather.result.timelines.daily;

  return (
    <>
      {hourly && hourly.length > 0 && (
        <div className="rainbow-panel-results">
          <div className="rainbow-panel-summary">Hourly</div>
          <div className="rainbow-panel-table-scroll">
            <table className="rainbow-panel-table">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Condition</th>
                  <th>Temp (°F)</th>
                  <th>Feels (°F)</th>
                  <th>Dew Pt (°F)</th>
                  <th>Precip ({units.precipitationChance})</th>
                  <th>Wind ({units.windSpeed})</th>
                </tr>
              </thead>
              <tbody>
                {hourly.map((h) => (
                  <tr key={h.startTimestamp}>
                    <td>{formatClockTime(h.startTimestamp)}</td>
                    <td>{h.condition}</td>
                    <td>{formatFahrenheit(h.temperature)}</td>
                    <td>{formatFahrenheit(h.feelsLikeTemperature)}</td>
                    <td>{formatFahrenheit(h.temperatureDewPoint)}</td>
                    <td>{h.precipitationChance.toFixed(0)}</td>
                    <td>{h.windSpeed.toFixed(1)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {daily && daily.length > 0 && (
        <div className="rainbow-panel-results">
          <div className="rainbow-panel-summary">Daily</div>
          <div className="rainbow-panel-table-scroll">
            <table className="rainbow-panel-table">
              <thead>
                <tr>
                  <th>Day</th>
                  <th>Condition</th>
                  <th>Min (°F)</th>
                  <th>Max (°F)</th>
                  <th>Precip ({units.precipitationChance})</th>
                  <th>Wind ({units.windSpeed})</th>
                  <th>UV</th>
                </tr>
              </thead>
              <tbody>
                {daily.map((d) => (
                  <tr key={d.startTimestamp}>
                    <td>{formatDayLabel(d.startTimestamp)}</td>
                    <td>{d.condition}</td>
                    <td>{formatFahrenheit(d.temperatureMin)}</td>
                    <td>{formatFahrenheit(d.temperatureMax)}</td>
                    <td>{d.precipitationChance.toFixed(0)}</td>
                    <td>{d.windSpeedMax.toFixed(1)}</td>
                    <td>{d.uvIndexMax}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </>
  );
}
