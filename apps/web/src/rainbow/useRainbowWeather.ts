// S09d Part C: Rainbow Weather (Forecast) API (KB §7) -- hourly/daily
// blended forecast at a point. Distinct product from this app's own
// GEFS/HRRR NWP "Forecast" panel (`forecast/`) -- never call this "Forecast"
// in UI copy; see `RainbowWeatherPanel.tsx`.
//
// Same phase-machine shape as `useRainbowNowcast.ts` (itself modeled on
// `forecast/useForecastProvider.ts`'s shape, not its content -- see that
// hook's own doc comment). Same desktop-only CORS situation as every other
// Rainbow endpoint.
import { useCallback, useRef, useState } from "react";
import { isDesktop } from "../platform/desktop";
import { useRainbowApiKey } from "./useRainbowApiKey";
import { rainbowErrorMessage, withTimeout } from "./rainbowAsync";
import { useRainbowPoint, type RainbowPoint } from "./useRainbowPoint";

export type RainbowWeatherPhase = "idle" | "loading" | "ready" | "error";

/** Wire shape of one `HourlyItem` (KB §7.2). */
export interface RainbowHourlyItem {
  startTimestamp: number;
  startTimeIso: string;
  condition: string;
  temperature: number;
  feelsLikeTemperature: number;
  temperatureDewPoint: number;
  humidity: number;
  pressure: number;
  precipitationAmount: number;
  precipitationChance: number;
  precipitationType: string;
  windSpeed: number;
  windGust: number;
  windDirection: number;
  visibility: number;
  uvIndex: number;
}

/** Wire shape of one `DailyItem` (KB §7.2). */
export interface RainbowDailyItem {
  startTimestamp: number;
  startTimeIso: string;
  endTimestamp: number;
  endTimeIso: string;
  condition: string;
  temperatureMin: number;
  temperatureMax: number;
  precipitationAmount: number;
  precipitationChance: number;
  precipitationType: string;
  windSpeedMax: number;
  windDirectionAvg: number;
  uvIndexMax: number;
}

/** Wire shape of `Timelines` (KB §7.2) -- either array is `null` when its
 * corresponding `forecast_hours`/`forecast_days` param wasn't requested. */
export interface RainbowTimelines {
  hourly: RainbowHourlyItem[] | null;
  daily: RainbowDailyItem[] | null;
}

export interface RainbowForecastLocation {
  lat: number;
  lon: number;
}

/** Wire shape of `ForecastUnits` (KB §7.2) -- a unit-label string per
 * numeric field. The UI must read these rather than hardcode units (this
 * stage's explicit requirement) -- see `RainbowWeatherPanel.tsx`. */
export interface RainbowForecastUnits {
  temperature: string;
  feelsLikeTemperature: string;
  temperatureDewPoint: string;
  precipitationAmount: string;
  precipitationChance: string;
  windSpeed: string;
  windGust: string;
  windDirection: string;
  visibility: string;
  uvIndex: string;
  humidity: string;
  pressure: string;
}

/** Wire shape of `ForecastResponse` (KB §7.2). */
export interface RainbowForecastResponse {
  timelines: RainbowTimelines;
  location: RainbowForecastLocation;
  units: RainbowForecastUnits;
  generatedAtTimestamp: number;
  generatedAtTimeIso: string;
}

/** The user-facing options this stage's UI exposes before pressing Get
 * (KB §7.1) -- `forecastHours`/`forecastDays` are each optional
 * independently ("leaving one unset returns `null` for it, that's fine,
 * don't force both" per this stage's brief); `dayStartHour` always has a
 * value (defaults to 6, KB's own documented default). */
export interface RainbowWeatherRequestOptions {
  forecastHours: number | null;
  forecastDays: number | null;
  dayStartHour: number;
}

export interface RainbowWeatherState {
  /** The point this panel's inputs currently show -- owned here (rather
   * than inside `RainbowWeatherPanel`) so `App.tsx` can push a clicked map
   * point into it (S09d follow-up: "click the map to set the point")
   * without reaching past this hook into the panel's own local state. */
  point: RainbowPoint;
  setLon: (lon: number) => void;
  setLat: (lat: number) => void;
  /** Set both coordinates at once -- what a map click uses (see
   * `useRainbowPoint`'s `setPoint` doc comment). */
  setPoint: (lon: number, lat: number) => void;
  resetToDefault: () => void;
  configured: boolean;
  desktopAvailable: boolean;
  phase: RainbowWeatherPhase;
  error: string | null;
  result: RainbowForecastResponse | null;
  /**
   * Fetch the forecast for `point` with `options`. Only ever runs when
   * called -- never as a reaction to a picker changing (this stage's "no
   * fetch until the user presses Get" requirement).
   */
  fetchWeather: (point: RainbowPoint, options: RainbowWeatherRequestOptions) => void;
}

async function invokeWeather(point: RainbowPoint, options: RainbowWeatherRequestOptions, apiKey: string): Promise<RainbowForecastResponse> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<RainbowForecastResponse>("rainbow_weather_forecast", {
    lon: point.lon,
    lat: point.lat,
    forecastHours: options.forecastHours,
    forecastDays: options.forecastDays,
    dayStartHour: options.dayStartHour,
    apiKey,
  });
}

/**
 * Owns one Weather (Rainbow's own blended forecast, KB §7) fetch at a time:
 * `GET /weather/v1/forecast/{lon}/{lat}` via the native
 * `rainbow_weather_forecast` Tauri command
 * (`apps/desktop/src-tauri/src/rainbow_weather.rs`). No 404-vs-fallback
 * concern here (unlike Nowcast) -- this endpoint has no documented
 * "-global" sibling.
 *
 * `defaultPoint` (same value passed everywhere else in this feature)
 * prefills the point and is followed until the user edits it by hand or
 * via a map click -- see `useRainbowPoint`'s doc comment.
 */
export function useRainbowWeather(defaultPoint: RainbowPoint): RainbowWeatherState {
  const { effectiveKey, configured } = useRainbowApiKey();
  const desktopAvailable = isDesktop();
  const { point, setLon, setLat, setPoint, resetToDefault } = useRainbowPoint(defaultPoint);

  const [phase, setPhase] = useState<RainbowWeatherPhase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<RainbowForecastResponse | null>(null);

  const generationRef = useRef(0);

  const fetchWeather = useCallback(
    (point: RainbowPoint, options: RainbowWeatherRequestOptions) => {
      const generation = ++generationRef.current;

      if (!desktopAvailable) {
        setPhase("error");
        setError("Rainbow forecast requires the desktop app -- api.rainbow.ai blocks direct browser requests (CORS).");
        return;
      }
      if (!configured) {
        setPhase("error");
        setError("Rainbow API key not configured -- add one in Settings.");
        return;
      }
      if (options.forecastHours === null && options.forecastDays === null) {
        setPhase("error");
        setError("pick hourly, daily, or both before fetching.");
        return;
      }

      setPhase("loading");
      setError(null);

      void (async () => {
        try {
          const data = await withTimeout(invokeWeather(point, options, effectiveKey), "Rainbow forecast");
          if (generationRef.current !== generation) return; // superseded by a newer fetchWeather call.
          setResult(data);
          setPhase("ready");
        } catch (err) {
          if (generationRef.current !== generation) return;
          setResult(null);
          setPhase("error");
          setError(rainbowErrorMessage(err));
        }
      })();
    },
    [configured, desktopAvailable, effectiveKey],
  );

  return { point, setLon, setLat, setPoint, resetToDefault, configured, desktopAvailable, phase, error, result, fetchWeather };
}
