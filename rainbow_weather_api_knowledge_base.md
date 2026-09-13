# Rainbow Weather API — Complete Knowledge Base

Compiled from https://doc.rainbow.ai/ (API version 0.35.2) on 2026-09-13.
## Source pages → where they live in this document

| Source URL | Section(s) |
|---|---|
| https://doc.rainbow.ai/ (Getting Started) | §1 Overview, §2 Authentication, §3 Conventions |
| https://doc.rainbow.ai/tile_colors/ | §5 Tile Color Palettes & Raw dBZ |
| https://doc.rainbow.ai/api-ref/tiles/ | §4 Tiles API (all 5 endpoints + schemas) |
| https://doc.rainbow.ai/api-ref/nowcast/ | §6 Nowcast API |
| https://doc.rainbow.ai/api-ref/weather/ | §7 Weather API, §8 Enums |
| https://doc.rainbow.ai/faq/tiles/ | §10 FAQ |
| https://doc.rainbow.ai/examples/tiles/leaflet/ | §11.1 Leaflet map (server.py + leaflet.html) |
| https://doc.rainbow.ai/examples/nowcast/chart/ | §11.2 Nowcast chart |
| https://doc.rainbow.ai/examples/weather/temperature_chart/ | §11.3 Hourly temperature chart |
| https://doc.rainbow.ai/examples/weather/wind_rose/ | §11.4 Wind rose |
| https://doc.rainbow.ai/examples/weather/daily_summary/ | §11.5 7-day daily forecast |

---

## Table of Contents

1. [Overview](#1-overview)
2. [Authentication](#2-authentication)
3. [Base URL & Conventions](#3-base-url--conventions)
4. [Tiles API](#4-tiles-api)
5. [Tile Color Palettes & Raw dBZ](#5-tile-color-palettes--raw-dbz)
6. [Nowcast API](#6-nowcast-api)
7. [Weather (Forecast) API](#7-weather-forecast-api)
8. [Shared Schemas & Enums](#8-shared-schemas--enums)
9. [Error Handling](#9-error-handling)
10. [FAQ](#10-faq)
11. [Code Examples](#11-code-examples)
12. [Quick Reference Cheat Sheet](#12-quick-reference-cheat-sheet)

---

## 1. Overview

The Rainbow Weather API provides accurate, real-time and forecasted weather data for locations worldwide. It is built for high-resolution weather use cases: minute-by-minute precipitation forecasts and precipitation map tiles.

Intended uses: mobile weather apps, travel planners, outdoor activity organizers, logistics/route planning, and emergency response systems.

The API is split into three products, all under one host and one API key:

| Product | Purpose | Base path |
|---|---|---|
| **Tiles API** | PNG map tiles (XYZ/Web Mercator) for precipitation, global precipitation, clouds, and radars | `/tiles/v1/` |
| **Nowcast API** | Minute-by-minute precipitation forecast for the next 4 hours at a point | `/nowcast/v1/` |
| **Weather API** | Hourly and daily weather forecast at a point | `/weather/v1/` |

Developer portal (get your key): https://developer.rainbow.ai/profile

---

## 2. Authentication

Every request must include an API token. Get it from your profile page on the developer portal.

Two ways to pass it:

**Option A — HTTP header**
```
Ocp-Apim-Subscription-Key: YOUR_API_KEY
```

**Option B — query parameter**
```
https://api.rainbow.ai/v1/weather/map/snapshot?token=YOUR_API_KEY
```

The official examples all use the `?token=` query parameter form.

If the token is missing or invalid, the API returns **401 Unauthorized**.

> Security note: The Leaflet example on the docs site proxies requests through a small FastAPI backend so the token never ships to the browser. Follow that pattern for any front-end use.

---

## 3. Base URL & Conventions

- **Production server:** `https://api.rainbow.ai`
- All endpoints are `GET`.
- Coordinates in paths are always **`{longitude}/{latitude}`** (lon first, then lat) for Nowcast and Weather.
- Timestamps are **Unix epoch seconds, UTC** unless stated otherwise.
- Tile snapshots are aligned to **10-minute** boundaries; nowcast `start_timestamp` is aligned to **1-minute** boundaries.
- Validation errors come back as **422** in FastAPI's standard `{"detail": [...]}` shape.

---

## 4. Tiles API

Version 0.35.2. Provides weather map tiles for visualizing precipitation. Standard **XYZ tile scheme**, **Web Mercator (EPSG:3857)**, **PNG** output, compatible with Mapbox, Google Maps, Leaflet, OpenLayers, Cesium, and others.

Available layers: **`precip`**, **`precip-global`**, **`clouds`**, **`radars`**.

### 4.1 `GET /tiles/v1/snapshot` — Latest map snapshot

Returns the most recent snapshot timestamp for a layer. You need this before requesting any tile.

**Query parameters**

| Param | Type | Default | Description |
|---|---|---|---|
| `layer` | string | `precip` | Layer to get the snapshot for: `precip`, `precip-global`, `clouds`, `radars` |

**Response 200** (`MapSnapshot`)
```json
{ "snapshot": 1754991000 }
```

**Errors:** 422 (validation)

**Example**
```
GET https://api.rainbow.ai/tiles/v1/snapshot?token=<your_token>
GET https://api.rainbow.ai/tiles/v1/snapshot?layer=radars&token=<your_token>
```

### 4.2 `GET /tiles/v1/precip/{snapshot}/{forecast_time}/{zoom}/{tile_x}/{tile_y}` — Precipitation tile

**Path parameters**

| Param | Type | Constraints |
|---|---|---|
| `snapshot` | integer | Epoch UTC seconds, aligned to 10-min intervals. Accessible up to **2 hours** before the latest snapshot. |
| `forecast_time` | integer | Forecast offset in seconds. Range **[0, 14400]**, step **600** (0 = current observation, up to 4 h ahead in 10-min steps) |
| `zoom` | integer | **0–12** |
| `tile_x` | integer | 0 to 2^zoom |
| `tile_y` | integer | 0 to 2^zoom |

**Query parameters**

| Param | Type | Default | Description |
|---|---|---|---|
| `color` | string/int | `0` | Color palette code (see §5). Also accepts `dbz_u8` for raw values. |
| `coverage` | boolean | `0` | Coverage mask: `0` disable, `1` enable |

**Response 200:** `image/png` binary
**Errors:** 400 (`{"message": "..."}`), 404, 422

**Example**
```
https://api.rainbow.ai/tiles/v1/precip/1754389800/0/7/68/42?color=3&token=<your_token>
```

### 4.3 `GET /tiles/v1/precip-global/{snapshot}/{forecast_time}/{zoom}/{tile_x}/{tile_y}` — Global precipitation tile

Identical parameters and constraints to `/precip` (zoom 0–12, forecast_time 0–14400 step 600, `color`, `coverage`). Use the `precip-global` layer's snapshot from `/tiles/v1/snapshot?layer=precip-global`.

### 4.4 `GET /tiles/v1/clouds/{snapshot}/{zoom}/{tile_x}/{tile_y}` — Clouds tile

No `forecast_time` segment — observation only.

| Param | Type | Constraints |
|---|---|---|
| `snapshot` | integer | Same 10-min alignment / 2-hour history rule |
| `zoom` | integer | **0–7** |
| `tile_x`, `tile_y` | integer | 0 to 2^zoom |

No `color`/`coverage` query params documented. Response `image/png`; errors 400/404/422.

### 4.5 `GET /tiles/v1/radars/{snapshot}/{zoom}/{tile_x}/{tile_y}` — Radar tile

No `forecast_time` segment — observation only.

| Param | In | Type | Default | Description |
|---|---|---|---|---|
| `snapshot` | path | integer | | Same 10-min alignment / 2-hour history rule |
| `zoom` | path | integer | | **0–7** |
| `tile_x`, `tile_y` | path | integer | | 0 to 2^zoom |
| `color` | query | | `0` | Color palette code (see §5) |
| `coverage` | query | boolean | `0` | Coverage mask on/off |
| `use_precip_type` | query | boolean | `0` | `0` = show reflectivity, everything treated as rain; `1` = precipitation-type visualization (rain vs snow) |

Response `image/png`; errors 400/404/422.

### 4.6 Tiles workflow (the pattern to follow)

1. `GET /tiles/v1/snapshot?layer=<layer>` → `snapshot` timestamp.
2. Build tile URLs with that snapshot: `/tiles/v1/<layer>/<snapshot>/[<forecast_time>/]{z}/{x}/{y}`.
3. To animate a forecast, loop `forecast_time` from 0 to 14400 in steps of 600 (25 frames).
4. To animate history, subtract multiples of 600 from the snapshot, back to 7200 s (12 frames).
5. Refresh the snapshot periodically (new snapshots arrive every 10 minutes).

Leaflet layer settings used in the official example: `minZoom: 0`, `maxZoom: 7`, `tileSize: 256`, `noWrap: true`.

---

## 5. Tile Color Palettes & Raw dBZ

Pass the palette code in the `color` query parameter of a tile request.

| `color` | Name | Description |
|---|---|---|
| `0` | Rainbow | Default rainbow-style palette |
| `1` | TWC | Inspired by The Weather Channel |
| `2` | Dark Sky | Based on RainViewer's *Dark Sky* scheme |
| `3` | Meteored | Based on RainViewer's *Meteored* palette |
| `4` | Nexrad | *NEXRAD Level III* style from RainViewer |
| `5` | Rainviewer | *Rainviewer* color palette |
| `6` | Selex | *Rainbow @ SELEX-IS* palette from RainViewer |
| `7` | Titan | *TITAN* color scheme from RainViewer |
| `8` | Rainviewer Universal Blue | RainViewer's *Original* palette |
| `9` | Rainviewer TWC | RainViewer's *The Weather Channel (TWC)* palette |
| `dbz_u8` | Raw dBZ | Black-and-white raw reflectivity encoding (see below) |

Each palette has separate rain and snow color ramps. Preview images live at `https://doc.rainbow.ai/images/palletes/<name>_rain.png` / `_snow.png`.

Example:
```
https://api.rainbow.ai/tiles/v1/precip/1754389800/0/7/68/42?color=3
```

### 5.1 Raw dBZ tiles (`color=dbz_u8`)

Follows RainViewer's *Black and White dBZ Values* scheme.

- Values range from **−32 dBZ to +95 dBZ**.
- The **red channel** of each pixel encodes the reflectivity.
- **Snow** is flagged via the **highest bit** (bit 7, value 128) of the red channel.
- **Alpha = 0** means **no radar coverage**.

**Decoding algorithm**
```
R = red channel of pixel
if alpha == 0:            → no coverage
is_snow = (R & 128) == 128
dbz     = (R & 127) - 32
```

**Worked example**
```
R = 175
175 & 128 = 128  → snow
175 & 127 = 47
47 - 32   = 15 dBZ   → snow at 15 dBZ
```

**Python snippet**
```python
from PIL import Image

img = Image.open("tile.png").convert("RGBA")
px = img.load()
r, g, b, a = px[x, y]
if a == 0:
    print("no coverage")
else:
    snow = bool(r & 128)
    dbz = (r & 127) - 32
    print(f"{'snow' if snow else 'rain'} {dbz} dBZ")
```

---

## 6. Nowcast API

Version 0.35.2. Minute-by-minute precipitation forecast for the **next 4 hours** at a single point.

Use cases: short-term weather apps, logistics/route planning, outdoor activity planning.

### 6.1 `GET /nowcast/v1/precip/{longitude}/{latitude}`

Coverage: global, but **may return 404** where data is unavailable for that area.

**Path parameters**

| Param | Type | Description |
|---|---|---|
| `longitude` | number | Longitude of the location (comes **first**) |
| `latitude` | number | Latitude of the location |

**Query parameters**

| Param | Type | Description |
|---|---|---|
| `start_timestamp` | integer | Optional. Forecast start in epoch seconds. Must be **within the past 30 minutes**, **less than the current time**, and **aligned to 1-minute intervals**. Defaults to now. |

**Response 200** (`PrecipForecastResponse`)
```json
{
  "longitude": 21.0065536398819,
  "latitude": 52.2328940965463,
  "summary": { "intensity": "moderate" },
  "forecast": [
    {
      "precipRate": 1.42,
      "precipType": "snow",
      "timestampBegin": 1737570600,
      "timestampEnd": 1737570660
    }
  ]
}
```

**Errors:** 400, 404, 422

### 6.2 `GET /nowcast/v1/precip-global/{longitude}/{latitude}`

Same parameters and response shape as `/precip`, but with **global coverage** (no 404-for-coverage caveat). Use this as the fallback when `/precip` returns 404.

### 6.3 Nowcast schemas

**PrecipForecastResponse**

| Field | Type | Description |
|---|---|---|
| `longitude` | number | Longitude the forecast is for |
| `latitude` | number | Latitude the forecast is for |
| `summary` | PrecipForecastSummary | Classification of max intensity during the period |
| `forecast` | PrecipForecastItem[] | One item per minute for the next 4 hours |

**PrecipForecastItem**

| Field | Type | Description |
|---|---|---|
| `precipRate` | number | Precipitation rate in **mm/h** |
| `precipType` | string | `no_precipitation`, `snow`, `rain`, `mixed` |
| `timestampBegin` | integer | Start of the minute (epoch seconds) |
| `timestampEnd` | integer | End of the minute (epoch seconds) |

**PrecipForecastSummary**

| Field | Type | Description |
|---|---|---|
| `intensity` | string | Max precipitation intensity classification (e.g. `moderate`) |

---

## 7. Weather (Forecast) API

Version 0.35.2. Hourly and daily weather forecast for a point.

### 7.1 `GET /weather/v1/forecast/{longitude}/{latitude}`

**Path parameters**

| Param | Type | Description |
|---|---|---|
| `longitude` | number | Longitude (comes **first**) |
| `latitude` | number | Latitude |

**Query parameters**

| Param | Type | Default | Description |
|---|---|---|---|
| `forecast_hours` | integer | — | Number of hourly items to return (populates `timelines.hourly`) |
| `forecast_days` | integer | — | Number of daily items to return (populates `timelines.daily`) |
| `day_start_hour` | integer | `6` | Hour of day (0–23, in the location's timezone) at which each forecast "day" begins |

Examples from the docs use `forecast_hours=24`, `forecast_hours=48`, and `forecast_days=7`. If a timeline isn't requested it comes back `null`.

**Response 200** (`ForecastResponse`)
```json
{
  "timelines": { "hourly": [ ...HourlyItem ], "daily": [ ...DailyItem ] },
  "location": { "lat": 52.2328940965463, "lon": 21.0065536398819 },
  "units": {
    "temperature": "celsius",
    "feelsLikeTemperature": "celsius",
    "temperatureDewPoint": "celsius",
    "precipitationAmount": "millimeters",
    "precipitationChance": "percent",
    "windSpeed": "meter_per_second",
    "windGust": "meter_per_second",
    "windDirection": "degree",
    "visibility": "meter",
    "uvIndex": "index",
    "humidity": "percent",
    "pressure": "hectopascal"
  },
  "generatedAtTimestamp": 1705921200,
  "generatedAtTimeIso": "2024-01-22T13:00:00+00:00"
}
```

**Errors:** 422

### 7.2 Weather schemas

**ForecastResponse**

| Field | Type | Description |
|---|---|---|
| `timelines` | Timelines | `hourly` and `daily` arrays |
| `location` | ForecastLocation | `lat`, `lon` |
| `units` | ForecastUnits | Unit for every numeric field |
| `generatedAtTimestamp` | integer | Generation time, epoch UTC |
| `generatedAtTimeIso` | string | Generation time, ISO 8601 with offset |

**HourlyItem**

| Field | Type | Description |
|---|---|---|
| `startTimestamp` | integer | Hour start, epoch UTC |
| `startTimeIso` | string | Hour start, ISO 8601 with tz offset |
| `condition` | WeatherCondition | Dominant condition (enum, see §8) |
| `temperature` | number | Air temperature |
| `feelsLikeTemperature` | number | Apparent temperature |
| `temperatureDewPoint` | number | Dew point |
| `humidity` | integer | Relative humidity 0–100 |
| `pressure` | number | Sea-level pressure |
| `precipitationAmount` | number | Total precipitation |
| `precipitationChance` | number | Probability 0–100 |
| `precipitationType` | string | `no_precipitation`, `snow`, `rain`, `mixed` |
| `windSpeed` | number | Wind speed |
| `windGust` | number | Gust speed |
| `windDirection` | integer | Degrees 0–360, clockwise from north |
| `visibility` | integer | Visibility |
| `uvIndex` | integer | UV index |

**DailyItem**

| Field | Type | Description |
|---|---|---|
| `startTimestamp` / `startTimeIso` | integer / string | Day start |
| `endTimestamp` / `endTimeIso` | integer / string | Day end |
| `condition` | WeatherCondition | Dominant condition for the day |
| `temperatureMin` | number | Daily min |
| `temperatureMax` | number | Daily max |
| `precipitationAmount` | number | Daily total |
| `precipitationChance` | number | Peak probability 0–100 |
| `precipitationType` | string | Dominant type |
| `windSpeedMax` | number | Max wind speed |
| `windDirectionAvg` | integer | Avg direction, degrees |
| `uvIndexMax` | integer | Max UV index |

**ForecastUnits** — string unit label for each of: `temperature`, `feelsLikeTemperature`, `temperatureDewPoint`, `precipitationAmount`, `precipitationChance`, `pressure`, `humidity`, `uvIndex`, `visibility`, `windDirection`, `windGust`, `windSpeed`. Documented values are metric (celsius, millimeters, meter_per_second, meter, hectopascal, percent, degree, index). No unit-selection parameter is documented; convert client-side.

---

## 8. Shared Schemas & Enums

**ResponsePrecipType** (used by Nowcast and Weather)
`no_precipitation` | `snow` | `rain` | `mixed`

**ResponsePrecipIntensity** (Nowcast summary) — string classification, e.g. `moderate`. Full enum not listed in docs.

**WeatherCondition** (Weather hourly/daily `condition`) — 35 values:

| Group | Values |
|---|---|
| Sky | `Unknown`, `Clear`, `MostlyClear`, `PartlyCloudy`, `MostlyCloudy`, `Cloudy` |
| Visibility | `Foggy`, `Haze`, `Smoky`, `BlowingDust` |
| Wind | `Breezy`, `Windy` |
| Rain | `Drizzle`, `Rain`, `HeavyRain`, `SunShowers` |
| Storms | `IsolatedThunderstorms`, `ScatteredThunderstorms`, `Thunderstorms`, `StrongStorms` |
| Temperature | `Frigid`, `Hot` |
| Winter | `Flurries`, `SunFlurries`, `Snow`, `HeavySnow`, `BlowingSnow`, `Blizzard`, `Sleet`, `WintryMix`, `FreezingDrizzle`, `FreezingRain`, `Hail` |
| Tropical | `TropicalStorm`, `Hurricane` |

**TilesLayer** — string: `precip` | `precip-global` | `clouds` | `radars`

**MapSnapshot** — `{ "snapshot": integer }`

---

## 9. Error Handling

| Status | Meaning | Body shape |
|---|---|---|
| 200 | Success | Endpoint-specific |
| 400 | Bad request (e.g. snapshot out of range, bad forecast_time) | `{ "message": "string" }` (`ResponseError`) |
| 401 | Missing or invalid token | — |
| 404 | Not found — tile doesn't exist, or nowcast data unavailable for that area | `{ "message": "string" }` |
| 422 | Validation error (bad types, out-of-range path/query params) | `{ "detail": [ { "loc": [...], "msg": "...", "type": "..." } ] }` (`HTTPValidationError`) |

Practical handling:
- On 401 → check the token/header spelling (`Ocp-Apim-Subscription-Key`).
- On 404 from `/nowcast/v1/precip` → retry with `/nowcast/v1/precip-global`.
- On 400/404 from tiles → re-fetch `/tiles/v1/snapshot` (your timestamp may be stale, >2 h old, or not 10-min aligned).
- On 422 → read `detail[].loc` to see which parameter failed.

---

## 10. FAQ

### How do I get past (historical) tile data?

1. Get the current snapshot:
   ```
   GET https://api.rainbow.ai/tiles/v1/snapshot?token=<your_token>
   → {"snapshot": 1754991000}
   ```
2. Latest observation tiles:
   ```
   https://api.rainbow.ai/tiles/v1/precip/1754991000/0/0/0/0?token=<your_token>
   ```
3. For 10 minutes earlier, subtract 600 s:
   ```
   https://api.rainbow.ai/tiles/v1/precip/1754990400/0/0/0/0?token=<your_token>
   ```

Limits: up to **2 hours** back (snapshot − 7200), timestamps must be **10-minute aligned**.

### How far ahead can tiles forecast?
Up to **4 hours** (`forecast_time` ≤ 14400) in 10-minute steps, for `precip` and `precip-global` layers only. `clouds` and `radars` have no forecast dimension.

### What zoom levels are supported?
`precip` / `precip-global`: 0–12. `clouds` / `radars`: 0–7.

### Can the nowcast start in the past?
Yes — `start_timestamp` may be up to 30 minutes in the past, 1-minute aligned.

---

## 11. Code Examples

All examples read the key from the `RAINBOW_API_TOKEN` environment variable:
```bash
export RAINBOW_API_TOKEN=<your-api-key>        # macOS/Linux
$env:RAINBOW_API_TOKEN="<your-api-key>"        # Windows PowerShell
```

### 11.1 Leaflet precipitation map (FastAPI proxy + HTML)

Install: `pip install fastapi uvicorn requests`

**server.py** — proxies snapshot and tile requests, keeps the token server-side, adds CORS.
```python
import os
import requests
import uvicorn

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse, Response

RAINBOW_API_TOKEN = os.getenv("RAINBOW_API_TOKEN")

app = FastAPI()

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],  # You can restrict this later to specific domains
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

@app.get("/tiles/v1/snapshot")
def get_snapshot_timestamp_handler():
    response = requests.get(f"https://api.rainbow.ai/tiles/v1/snapshot?token={RAINBOW_API_TOKEN}")
    return JSONResponse(content=response.json())

@app.get("/tiles/v1/precip/{snapshot_timestamp}/{forecast_time}/{zoom}/{x}/{y}")
def get_tile_handler(snapshot_timestamp: int, forecast_time: int, zoom: int, x: int, y: int):
    url = f"https://api.rainbow.ai/tiles/v1/precip/{snapshot_timestamp}/{forecast_time}/{zoom}/{x}/{y}?token={RAINBOW_API_TOKEN}"
    response = requests.get(url, stream=True)
    return Response(content=response.content, media_type="image/png")

if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=8000)
```

**leaflet.html**
```html
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Rainbow AI Precipitation Map</title>
    <link rel="stylesheet" href="https://unpkg.com/leaflet@1.9.4/dist/leaflet.css" />
    <style>
        body { margin: 0; padding: 0; }
        #map { width: 100%; height: 100vh; }
    </style>
</head>
<body>
    <div id="map"></div>
    <script src="https://unpkg.com/leaflet@1.9.4/dist/leaflet.js"></script>
    <script>
        const BACKEND_URL = "http://localhost:8000"; // Replace with your actual backend URL

        async function getLatestSnapshotTimestamp() {
            const response = await fetch(`${BACKEND_URL}/tiles/v1/snapshot`);
            const data = await response.json();
            return data;
        }

        async function initializeMap() {
            const snapshot = await getLatestSnapshotTimestamp();
            const snapshotTimestamp = snapshot["snapshot"];
            const forecastTime = 0;

            const map = L.map('map').setView([50, 10], 5);

            // Base OpenStreetMap layer
            L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
                maxZoom: 19,
                attribution: '&copy; <a href="http://www.openstreetmap.org/copyright">OpenStreetMap</a>'
            }).addTo(map);

            // Custom precipitation tile layer
            const TILES_URL = `${BACKEND_URL}/tiles/v1/precip/${snapshotTimestamp}/${forecastTime}/{z}/{x}/{y}`;
            L.tileLayer(TILES_URL, {
                minZoom: 0,
                maxZoom: 7,
                tileSize: 256,
                attribution: 'Rainbow AI Precipitation Tiles',
                noWrap: true
            }).addTo(map);
        }

        document.addEventListener("DOMContentLoaded", () => { initializeMap(); });
    </script>
</body>
</html>
```

Run: `python server.py`, then open `leaflet.html`.

### 11.2 Nowcast chart (matplotlib)

Install: `pip install matplotlib requests`
```python
import matplotlib.pyplot as plt
import os
import requests
from datetime import datetime, timezone

RAINBOW_API_TOKEN = os.getenv("RAINBOW_API_TOKEN")

# Put your location here
LON = -98.41419632222147
LAT = 42.146755295096966

nowcast_url = f"https://api.rainbow.ai/nowcast/v1/precip/{LON}/{LAT}?token={RAINBOW_API_TOKEN}"
response = requests.get(nowcast_url).json()

timestamps = [datetime.fromtimestamp(f["timestampBegin"], timezone.utc).strftime("%H:%M")
              for f in response["forecast"]]
precipitation_rates = [f["precipRate"] for f in response["forecast"]]

fig, ax = plt.subplots(figsize=(10, 6))
ax.fill_between(timestamps, precipitation_rates, color="skyblue", alpha=0.6)
ax.plot(timestamps, precipitation_rates, color="skyblue", linewidth=2)

ax.set_xlabel("Time")
ax.set_xlim(timestamps[0], timestamps[-1])
ax.set_ylabel("Precipitation Rate (mm/hr)")
ax.set_ylim(bottom=0.1)
ax.set_xticks(timestamps[::10])
ax.set_xticklabels(timestamps[::10], rotation=45)

plt.title("Precipitation Rate Over Time (Filled Line)")
plt.tight_layout()
plt.show()
```

### 11.3 Hourly temperature chart (24 h)

Install: `pip install matplotlib requests`
```python
import matplotlib.pyplot as plt
import os
import requests
from datetime import datetime

RAINBOW_API_TOKEN = os.getenv("RAINBOW_API_TOKEN")

# London
LON = -0.1278
LAT = 51.5074

url = (
    f"https://api.rainbow.ai/weather/v1/forecast/{LON}/{LAT}"
    f"?forecast_hours=24&token={RAINBOW_API_TOKEN}"
)
data = requests.get(url).json()

hourly = data["timelines"]["hourly"]
times = [datetime.fromisoformat(h["startTimeIso"]) for h in hourly]
temperature = [h["temperature"] for h in hourly]
feels_like = [h["feelsLikeTemperature"] for h in hourly]

fig, ax = plt.subplots(figsize=(12, 5))
ax.plot(times, temperature, color="tomato", linewidth=2, label="Temperature (°C)")
ax.plot(times, feels_like, color="steelblue", linewidth=2, linestyle="--", label="Feels like (°C)")

ax.set_xlabel("Time")
ax.set_ylabel("Temperature (°C)")
ax.set_title(f"Hourly Temperature Forecast — ({LAT}, {LON})")
ax.xaxis.set_major_formatter(plt.matplotlib.dates.DateFormatter("%H:%M"))
plt.gcf().autofmt_xdate(rotation=45)
ax.legend()
ax.grid(axis="y", alpha=0.3)

plt.tight_layout()
plt.show()
```

### 11.4 Wind rose (48 h)

Install: `pip install matplotlib numpy requests`
```python
import matplotlib.pyplot as plt
import numpy as np
import os
import requests

RAINBOW_API_TOKEN = os.getenv("RAINBOW_API_TOKEN")

# London
LON = -0.1278
LAT = 51.5074

url = (
    f"https://api.rainbow.ai/weather/v1/forecast/{LON}/{LAT}"
    f"?forecast_hours=48&token={RAINBOW_API_TOKEN}"
)
data = requests.get(url).json()

hourly = data["timelines"]["hourly"]
directions = [h["windDirection"] for h in hourly]
speeds = [h["windSpeed"] for h in hourly]

# Bin directions into 16 compass sectors (22.5° each)
num_sectors = 16
sector_width = 360 / num_sectors
speed_bins = [0, 3, 6, 9, 12, float("inf")]
speed_labels = ["0–3", "3–6", "6–9", "9–12", ">12"]
colors = ["#d0e8f7", "#81c3e8", "#3498db", "#1a6fa8", "#0d3d6b"]

sector_counts = np.zeros((num_sectors, len(speed_labels)))
for direction, speed in zip(directions, speeds):
    sector = int((direction + sector_width / 2) % 360 / sector_width)
    for i in range(len(speed_bins) - 1):
        if speed_bins[i] <= speed < speed_bins[i + 1]:
            sector_counts[sector][i] += 1
            break

angles = np.linspace(0, 2 * np.pi, num_sectors, endpoint=False)
width = 2 * np.pi / num_sectors

fig, ax = plt.subplots(figsize=(8, 8), subplot_kw={"projection": "polar"})
ax.set_theta_zero_location("N")
ax.set_theta_direction(-1)

bottom = np.zeros(num_sectors)
for i, (label, color) in enumerate(zip(speed_labels, colors)):
    counts = sector_counts[:, i]
    ax.bar(angles, counts, width=width * 0.9, bottom=bottom,
           color=color, label=f"{label} m/s", alpha=0.9)
    bottom += counts

ax.set_xticks(np.linspace(0, 2 * np.pi, 8, endpoint=False))
ax.set_xticklabels(["N", "NE", "E", "SE", "S", "SW", "W", "NW"])
ax.set_title(f"Wind Rose — 48-Hour Forecast\n({LAT}, {LON})", pad=20)
ax.legend(loc="lower right", bbox_to_anchor=(1.3, -0.1), title="Wind speed")

plt.tight_layout()
plt.show()
```

### 11.5 7-day daily forecast chart

Install: `pip install matplotlib requests`
```python
import matplotlib.pyplot as plt
import matplotlib.patches as mpatches
import os
import requests
from datetime import datetime

RAINBOW_API_TOKEN = os.getenv("RAINBOW_API_TOKEN")

# London
LON = -0.1278
LAT = 51.5074

url = (
    f"https://api.rainbow.ai/weather/v1/forecast/{LON}/{LAT}"
    f"?forecast_days=7&token={RAINBOW_API_TOKEN}"
)
data = requests.get(url).json()

daily = data["timelines"]["daily"]
labels = [datetime.fromisoformat(d["startTimeIso"]).strftime("%a\n%b %d") for d in daily]
temp_min = [d["temperatureMin"] for d in daily]
temp_max = [d["temperatureMax"] for d in daily]
precip_chance = [d["precipitationChance"] for d in daily]

x = range(len(daily))
fig, ax1 = plt.subplots(figsize=(12, 6))

bar_width = 0.5
ax1.bar(x, [mx - mn for mx, mn in zip(temp_max, temp_min)],
        bottom=temp_min, width=bar_width,
        color="tomato", alpha=0.7, label="Temp range (°C)")

for i, (mn, mx) in enumerate(zip(temp_min, temp_max)):
    ax1.text(i, mn - 0.8, f"{mn:.0f}°", ha="center", va="top", fontsize=9, color="steelblue")
    ax1.text(i, mx + 0.3, f"{mx:.0f}°", ha="center", va="bottom", fontsize=9, color="tomato")

ax1.set_ylabel("Temperature (°C)")
ax1.set_xticks(list(x))
ax1.set_xticklabels(labels)
ax1.grid(axis="y", alpha=0.3)

ax2 = ax1.twinx()
ax2.plot(list(x), precip_chance, color="steelblue", linewidth=2,
         marker="o", markersize=5, label="Precip chance (%)")
ax2.set_ylabel("Precipitation Chance (%)")
ax2.set_ylim(0, 110)
ax2.yaxis.label.set_color("steelblue")

ax1.set_title(f"7-Day Daily Forecast — ({LAT}, {LON})")

lines = [
    mpatches.Patch(color="tomato", alpha=0.7, label="Temp range"),
    plt.Line2D([0], [0], color="steelblue", linewidth=2, marker="o", label="Precip chance (%)"),
]
ax1.legend(handles=lines, loc="upper left")

plt.tight_layout()
plt.show()
```

### 11.6 Minimal helper (not from docs — derived from the reference)

```python
import os, requests

BASE = "https://api.rainbow.ai"
HEADERS = {"Ocp-Apim-Subscription-Key": os.environ["RAINBOW_API_TOKEN"]}

def snapshot(layer="precip"):
    return requests.get(f"{BASE}/tiles/v1/snapshot", params={"layer": layer}, headers=HEADERS).json()["snapshot"]

def tile(layer, snap, z, x, y, forecast_time=None, **q):
    path = f"{BASE}/tiles/v1/{layer}/{snap}/" + (f"{forecast_time}/" if forecast_time is not None else "") + f"{z}/{x}/{y}"
    return requests.get(path, params=q, headers=HEADERS).content  # PNG bytes

def nowcast(lon, lat, use_global=False, start_timestamp=None):
    layer = "precip-global" if use_global else "precip"
    q = {"start_timestamp": start_timestamp} if start_timestamp else {}
    r = requests.get(f"{BASE}/nowcast/v1/{layer}/{lon}/{lat}", params=q, headers=HEADERS)
    if r.status_code == 404 and not use_global:
        return nowcast(lon, lat, use_global=True, start_timestamp=start_timestamp)
    r.raise_for_status()
    return r.json()

def forecast(lon, lat, hours=None, days=None, day_start_hour=6):
    q = {"day_start_hour": day_start_hour}
    if hours: q["forecast_hours"] = hours
    if days:  q["forecast_days"] = days
    return requests.get(f"{BASE}/weather/v1/forecast/{lon}/{lat}", params=q, headers=HEADERS).json()
```

---

## 12. Quick Reference Cheat Sheet

```
AUTH      header: Ocp-Apim-Subscription-Key: KEY   |   or ?token=KEY
HOST      https://api.rainbow.ai

TILES
  GET /tiles/v1/snapshot?layer={precip|precip-global|clouds|radars}      → {"snapshot": epoch}
  GET /tiles/v1/precip/{snap}/{fcst}/{z}/{x}/{y}?color=&coverage=          z 0-12, fcst 0..14400 step 600
  GET /tiles/v1/precip-global/{snap}/{fcst}/{z}/{x}/{y}?color=&coverage=   z 0-12, fcst 0..14400 step 600
  GET /tiles/v1/clouds/{snap}/{z}/{x}/{y}                                   z 0-7
  GET /tiles/v1/radars/{snap}/{z}/{x}/{y}?color=&coverage=&use_precip_type= z 0-7
  snapshot: 10-min aligned epoch UTC; history ≤ 2 h back
  color: 0 Rainbow | 1 TWC | 2 Dark Sky | 3 Meteored | 4 Nexrad | 5 Rainviewer
         6 Selex | 7 Titan | 8 RV Universal Blue | 9 RV TWC | dbz_u8 raw
  dbz_u8: snow = R&128 ; dbz = (R&127)-32 ; alpha 0 = no coverage

NOWCAST  (lon BEFORE lat)
  GET /nowcast/v1/precip/{lon}/{lat}?start_timestamp=          may 404 outside coverage
  GET /nowcast/v1/precip-global/{lon}/{lat}?start_timestamp=   global
  start_timestamp: ≤ 30 min in past, 1-min aligned; default now
  → forecast[]: precipRate (mm/h), precipType, timestampBegin/End ; summary.intensity

WEATHER  (lon BEFORE lat)
  GET /weather/v1/forecast/{lon}/{lat}?forecast_hours=&forecast_days=&day_start_hour=6
  → timelines.hourly[] / timelines.daily[], units{}, location{}, generatedAt*

ERRORS   400 {"message"} | 401 bad token | 404 not found | 422 {"detail":[{loc,msg,type}]}
```
