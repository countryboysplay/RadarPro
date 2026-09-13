// RadarPro desktop shell -- Rainbow Weather Nowcast + Weather (Forecast)
// APIs, S09d Parts B/C. See `Agent Context/context/stages/
// S09d-rainbow-full-api.md` and `rainbow_weather_api_knowledge_base.md`
// (KB) §6/§7.
//
// Same CORS situation as the Tiles API (`rainbow.rs`'s module doc comment):
// `api.rainbow.ai` sends no `Access-Control-Allow-Origin` header for any
// browser origin, so these two JSON endpoints get the same native-Rust
// transport bypass, gated the same way (`isDesktop()` on the frontend, see
// `apps/web/src/rainbow/useRainbowNowcast.ts` / `useRainbowWeather.ts`).
// Reuses `rainbow.rs`'s pooled `client()` and `sanitize_reqwest_error`
// (now `pub(crate)`) rather than standing up a second HTTP client or a
// second copy of the same error-sanitization rule.
//
// Unlike the Tiles API, both endpoints here return JSON bodies (not PNG
// bytes), so the response is deserialized into a typed struct and hand
// back to JS as plain JSON over `invoke()` -- no base64 encoding needed.
use crate::rainbow::{client, sanitize_reqwest_error, RAINBOW_API_BASE};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------
// Nowcast API (KB §6) -- minute-by-minute precip forecast at a point.
// ---------------------------------------------------------------------

/// KB §6.1: `start_timestamp` must be aligned to a 1-minute boundary.
const NOWCAST_START_TIMESTAMP_ALIGN_SECONDS: i64 = 60;
/// KB §6.1: `start_timestamp` may be at most 30 minutes in the past.
const NOWCAST_START_TIMESTAMP_MAX_AGE_SECONDS: i64 = 1_800;

/// Wire shape of `PrecipForecastSummary` (KB §6.3).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrecipForecastSummary {
    pub intensity: String,
}

/// Wire shape of one `PrecipForecastItem` (KB §6.3).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrecipForecastItem {
    pub precip_rate: f64,
    pub precip_type: String,
    pub timestamp_begin: i64,
    pub timestamp_end: i64,
}

/// Wire shape of `GET /nowcast/v1/precip{,-global}/{lon}/{lat}`'s
/// `PrecipForecastResponse` (KB §6.3). Deserialized from the API's response
/// and re-serialized back to JS with the identical field names (both sides
/// use `camelCase`), so `apps/web/src/rainbow/useRainbowNowcast.ts`'s
/// TypeScript type can mirror the KB's field table directly.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrecipForecastResponse {
    pub longitude: f64,
    pub latitude: f64,
    pub summary: PrecipForecastSummary,
    pub forecast: Vec<PrecipForecastItem>,
}

/// Validates a caller-supplied `start_timestamp` against KB §6.1's three
/// documented constraints -- surfaced as a clear `Err` rather than ever
/// forwarding an invalid value to the API (which would just come back as an
/// opaque 422). `now` is injected (not read internally) so tests can supply
/// a fixed clock instead of racing the real one.
fn validate_nowcast_start_timestamp(start_timestamp: i64, now: i64) -> Result<(), String> {
    if start_timestamp % NOWCAST_START_TIMESTAMP_ALIGN_SECONDS != 0 {
        return Err(format!(
            "start_timestamp {start_timestamp} must be aligned to a 1-minute boundary (KB §6.1)"
        ));
    }
    if start_timestamp >= now {
        return Err(format!(
            "start_timestamp {start_timestamp} must be before the current time ({now}) (KB §6.1)"
        ));
    }
    if now - start_timestamp > NOWCAST_START_TIMESTAMP_MAX_AGE_SECONDS {
        return Err(format!(
            "start_timestamp {start_timestamp} is more than {} minutes in the past (KB §6.1)",
            NOWCAST_START_TIMESTAMP_MAX_AGE_SECONDS / 60
        ));
    }
    Ok(())
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Builds the Nowcast URL for either `precip` (`use_global: false`) or
/// `precip-global` (`use_global: true`) -- KB §6.1/§6.2. Coordinates are
/// **lon before lat** in the path (KB §3/§6.1) -- note the order matches
/// this project's own function-argument order too, deliberately, so a
/// transposed call is a visible diff, not a silent swap.
fn nowcast_url(use_global: bool, lon: f64, lat: f64, start_timestamp: Option<i64>, api_key: &str) -> Result<reqwest::Url, String> {
    let layer_segment = if use_global { "precip-global" } else { "precip" };
    let mut url = reqwest::Url::parse(RAINBOW_API_BASE).map_err(|_| "internal error building Rainbow URL".to_string())?;
    url.set_path(&format!("/nowcast/v1/{layer_segment}/{lon}/{lat}"));
    {
        let mut pairs = url.query_pairs_mut();
        if let Some(ts) = start_timestamp {
            pairs.append_pair("start_timestamp", &ts.to_string());
        }
        pairs.append_pair("token", api_key);
    }
    Ok(url)
}

/// Native-HTTP transport for `GET /nowcast/v1/precip{,-global}/{lon}/{lat}`
/// (KB §6.1/§6.2). `use_global` selects which of the two layers to call --
/// deliberately a plain bool param on one command rather than two near-
/// identical commands, so the 404-vs-other-error retry/fallback decision
/// stays in the frontend hook where it is visible and independently
/// testable (`useRainbowNowcast.ts`), not hidden inside this function. A
/// non-2xx response is `Err(format!("HTTP {status}"))` -- same convention as
/// `rainbow_fetch_tile` -- so the frontend can distinguish "404, retry with
/// precip-global" from a genuine network failure by inspecting the message.
#[tauri::command]
pub async fn rainbow_nowcast_precip(
    lon: f64,
    lat: f64,
    use_global: bool,
    start_timestamp: Option<i64>,
    api_key: String,
) -> Result<PrecipForecastResponse, String> {
    if let Some(ts) = start_timestamp {
        validate_nowcast_start_timestamp(ts, current_unix_timestamp())?;
    }
    let url = nowcast_url(use_global, lon, lat, start_timestamp, &api_key)?;
    let response = client().get(url).send().await.map_err(|e| sanitize_reqwest_error(&e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    response.json().await.map_err(|e| sanitize_reqwest_error(&e))
}

// ---------------------------------------------------------------------
// Weather (Forecast) API (KB §7) -- hourly/daily forecast at a point.
// ---------------------------------------------------------------------

/// KB §7.1: `day_start_hour` is a 24-hour-clock hour of day.
const DAY_START_HOUR_MAX: u32 = 23;
/// KB §7.1: documented default for `day_start_hour` when the caller omits
/// it entirely.
const DAY_START_HOUR_DEFAULT: u32 = 6;

/// Wire shape of one `HourlyItem` (KB §7.2).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourlyItem {
    pub start_timestamp: i64,
    pub start_time_iso: String,
    pub condition: String,
    pub temperature: f64,
    pub feels_like_temperature: f64,
    pub temperature_dew_point: f64,
    pub humidity: i64,
    pub pressure: f64,
    pub precipitation_amount: f64,
    pub precipitation_chance: f64,
    pub precipitation_type: String,
    pub wind_speed: f64,
    pub wind_gust: f64,
    pub wind_direction: i64,
    pub visibility: i64,
    pub uv_index: i64,
}

/// Wire shape of one `DailyItem` (KB §7.2).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyItem {
    pub start_timestamp: i64,
    pub start_time_iso: String,
    pub end_timestamp: i64,
    pub end_time_iso: String,
    pub condition: String,
    pub temperature_min: f64,
    pub temperature_max: f64,
    pub precipitation_amount: f64,
    pub precipitation_chance: f64,
    pub precipitation_type: String,
    pub wind_speed_max: f64,
    pub wind_direction_avg: i64,
    pub uv_index_max: i64,
}

/// Wire shape of `Timelines` (KB §7.2) -- either array can come back `null`
/// when its corresponding `forecast_hours`/`forecast_days` query param was
/// omitted (KB §7.1), which is a normal, expected shape, not an error.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timelines {
    pub hourly: Option<Vec<HourlyItem>>,
    pub daily: Option<Vec<DailyItem>>,
}

/// Wire shape of `ForecastLocation` (KB §7.2).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForecastLocation {
    pub lat: f64,
    pub lon: f64,
}

/// Wire shape of `ForecastUnits` (KB §7.2) -- a unit-label string per
/// numeric field elsewhere in the response. Passed through verbatim rather
/// than hardcoded on the frontend (this stage's explicit requirement) so a
/// future non-metric API change would show up as a UI label change, not a
/// silently wrong hardcoded unit.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastUnits {
    pub temperature: String,
    pub feels_like_temperature: String,
    pub temperature_dew_point: String,
    pub precipitation_amount: String,
    pub precipitation_chance: String,
    pub wind_speed: String,
    pub wind_gust: String,
    pub wind_direction: String,
    pub visibility: String,
    pub uv_index: String,
    pub humidity: String,
    pub pressure: String,
}

/// Wire shape of `GET /weather/v1/forecast/{lon}/{lat}`'s `ForecastResponse`
/// (KB §7.2). Same deserialize-then-reserialize-verbatim convention as
/// {@link PrecipForecastResponse}.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastResponse {
    pub timelines: Timelines,
    pub location: ForecastLocation,
    pub units: ForecastUnits,
    pub generated_at_timestamp: i64,
    pub generated_at_time_iso: String,
}

/// Builds the Weather API URL (KB §7.1). Coordinates are **lon before
/// lat** in the path, same as Nowcast. `forecast_hours`/`forecast_days` are
/// each included only when the caller actually asked for that timeline
/// (KB: "If a timeline isn't requested it comes back `null`") --
/// `day_start_hour` is always sent, defaulted to `DAY_START_HOUR_DEFAULT`
/// by the command below rather than silently omitted, since the API's own
/// documented default and this app's default already agree (KB §7.1).
fn weather_forecast_url(
    lon: f64,
    lat: f64,
    forecast_hours: Option<u32>,
    forecast_days: Option<u32>,
    day_start_hour: u32,
    api_key: &str,
) -> Result<reqwest::Url, String> {
    if day_start_hour > DAY_START_HOUR_MAX {
        return Err(format!("day_start_hour {day_start_hour} out of range [0, {DAY_START_HOUR_MAX}] (KB §7.1)"));
    }
    let mut url = reqwest::Url::parse(RAINBOW_API_BASE).map_err(|_| "internal error building Rainbow URL".to_string())?;
    url.set_path(&format!("/weather/v1/forecast/{lon}/{lat}"));
    {
        let mut pairs = url.query_pairs_mut();
        if let Some(hours) = forecast_hours {
            pairs.append_pair("forecast_hours", &hours.to_string());
        }
        if let Some(days) = forecast_days {
            pairs.append_pair("forecast_days", &days.to_string());
        }
        pairs.append_pair("day_start_hour", &day_start_hour.to_string());
        pairs.append_pair("token", api_key);
    }
    Ok(url)
}

/// Native-HTTP transport for `GET /weather/v1/forecast/{lon}/{lat}` (KB
/// §7.1). Neither `forecast_hours` nor `forecast_days` is required by the
/// API (KB: "leaving one unset returns `null` for it") -- both are plain
/// `Option`s here, and the frontend decides what counts as "the user asked
/// for at least one timeline" before ever calling this (Global Contract:
/// no fetch until the user explicitly presses Get).
#[tauri::command]
pub async fn rainbow_weather_forecast(
    lon: f64,
    lat: f64,
    forecast_hours: Option<u32>,
    forecast_days: Option<u32>,
    day_start_hour: Option<u32>,
    api_key: String,
) -> Result<ForecastResponse, String> {
    let day_start_hour = day_start_hour.unwrap_or(DAY_START_HOUR_DEFAULT);
    let url = weather_forecast_url(lon, lat, forecast_hours, forecast_days, day_start_hour, &api_key)?;
    let response = client().get(url).send().await.map_err(|e| sanitize_reqwest_error(&e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    response.json().await.map_err(|e| sanitize_reqwest_error(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nowcast_url_precip_no_start_timestamp() {
        let url = nowcast_url(false, 21.0065536398819, 52.2328940965463, None, "KEY").unwrap();
        assert_eq!(
            url.to_string(),
            "https://api.rainbow.ai/nowcast/v1/precip/21.0065536398819/52.2328940965463?token=KEY"
        );
    }

    #[test]
    fn nowcast_url_precip_global_with_start_timestamp() {
        let url = nowcast_url(true, -98.4, 42.1, Some(1_737_570_600), "KEY").unwrap();
        assert_eq!(
            url.to_string(),
            "https://api.rainbow.ai/nowcast/v1/precip-global/-98.4/42.1?start_timestamp=1737570600&token=KEY"
        );
    }

    #[test]
    fn nowcast_url_lon_comes_before_lat() {
        // KB §3/§6.1: path is {longitude}/{latitude}, not the reverse.
        let url = nowcast_url(false, 10.0, 20.0, None, "KEY").unwrap();
        assert!(url.path().starts_with("/nowcast/v1/precip/10/20"), "path was {}", url.path());
    }

    #[test]
    fn start_timestamp_must_be_minute_aligned() {
        let err = validate_nowcast_start_timestamp(1_737_570_601, 1_737_571_000).unwrap_err();
        assert!(err.contains("1-minute boundary"), "unexpected error: {err}");
    }

    #[test]
    fn start_timestamp_must_be_before_now() {
        let err = validate_nowcast_start_timestamp(1_737_571_020, 1_737_571_000).unwrap_err();
        assert!(err.contains("before the current time"), "unexpected error: {err}");
    }

    #[test]
    fn start_timestamp_equal_to_now_is_rejected() {
        // KB §6.1: "less than the current time" -- equal is not less.
        // `now` itself must be minute-aligned here so this test exercises
        // only the equal-to-now rule, not also tripping the alignment rule.
        let now = 1_737_571_020;
        let err = validate_nowcast_start_timestamp(now, now).unwrap_err();
        assert!(err.contains("before the current time"), "unexpected error: {err}");
    }

    #[test]
    fn start_timestamp_more_than_30_minutes_ago_is_rejected() {
        let now = 1_737_571_020; // minute-aligned so the fixture is unambiguous
        let too_old = now - 1_800 - 60; // 31 minutes back, still minute-aligned relative to now
        let err = validate_nowcast_start_timestamp(too_old, now).unwrap_err();
        assert!(err.contains("30 minutes"), "unexpected error: {err}");
    }

    #[test]
    fn start_timestamp_exactly_30_minutes_ago_is_valid() {
        let now = 1_737_571_200; // minute-aligned for a clean fixture
        let ts = now - 1_800;
        assert!(validate_nowcast_start_timestamp(ts, now).is_ok());
    }

    #[test]
    fn start_timestamp_one_minute_ago_is_valid() {
        let now = 1_737_571_200;
        assert!(validate_nowcast_start_timestamp(now - 60, now).is_ok());
    }

    #[test]
    fn precip_forecast_response_parses_documented_shape() {
        let json = r#"{
            "longitude": 21.0065536398819,
            "latitude": 52.2328940965463,
            "summary": { "intensity": "moderate" },
            "forecast": [
                { "precipRate": 1.42, "precipType": "snow", "timestampBegin": 1737570600, "timestampEnd": 1737570660 }
            ]
        }"#;
        let parsed: PrecipForecastResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.summary.intensity, "moderate");
        assert_eq!(parsed.forecast.len(), 1);
        assert_eq!(parsed.forecast[0].precip_rate, 1.42);
        assert_eq!(parsed.forecast[0].precip_type, "snow");
    }

    #[test]
    fn weather_forecast_url_includes_only_requested_timelines() {
        let url = weather_forecast_url(-0.1278, 51.5074, Some(24), None, 6, "KEY").unwrap();
        let s = url.to_string();
        assert!(s.contains("forecast_hours=24"), "{s}");
        assert!(!s.contains("forecast_days"), "{s}");
        assert!(s.contains("day_start_hour=6"), "{s}");
    }

    #[test]
    fn weather_forecast_url_with_both_timelines() {
        let url = weather_forecast_url(-0.1278, 51.5074, Some(48), Some(7), 6, "KEY").unwrap();
        let s = url.to_string();
        assert!(s.contains("forecast_hours=48"), "{s}");
        assert!(s.contains("forecast_days=7"), "{s}");
    }

    #[test]
    fn weather_forecast_url_lon_comes_before_lat() {
        let url = weather_forecast_url(10.0, 20.0, None, None, 6, "KEY").unwrap();
        assert!(url.path().starts_with("/weather/v1/forecast/10/20"), "path was {}", url.path());
    }

    #[test]
    fn day_start_hour_out_of_range_is_rejected() {
        let err = weather_forecast_url(0.0, 0.0, None, None, 24, "KEY").unwrap_err();
        assert!(err.contains("day_start_hour"), "unexpected error: {err}");
    }

    #[test]
    fn day_start_hour_23_is_valid() {
        assert!(weather_forecast_url(0.0, 0.0, None, None, 23, "KEY").is_ok());
    }

    #[test]
    fn forecast_response_parses_documented_shape_with_null_daily() {
        let json = r#"{
            "timelines": { "hourly": [], "daily": null },
            "location": { "lat": 52.2328940965463, "lon": 21.0065536398819 },
            "units": {
                "temperature": "celsius", "feelsLikeTemperature": "celsius",
                "temperatureDewPoint": "celsius", "precipitationAmount": "millimeters",
                "precipitationChance": "percent", "windSpeed": "meter_per_second",
                "windGust": "meter_per_second", "windDirection": "degree",
                "visibility": "meter", "uvIndex": "index", "humidity": "percent",
                "pressure": "hectopascal"
            },
            "generatedAtTimestamp": 1705921200,
            "generatedAtTimeIso": "2024-01-22T13:00:00+00:00"
        }"#;
        let parsed: ForecastResponse = serde_json::from_str(json).unwrap();
        assert!(parsed.timelines.daily.is_none());
        assert_eq!(parsed.timelines.hourly.unwrap().len(), 0);
        assert_eq!(parsed.units.temperature, "celsius");
    }
}
