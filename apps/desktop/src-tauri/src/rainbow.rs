// RadarPro desktop shell -- Rainbow Weather tile CORS-bypass (S10 follow-up,
// generalized to the full Tiles API in S09d Part A).
//
// `api.rainbow.ai` (an Azure API Management gateway behind Cloudflare) sends
// no `Access-Control-Allow-Origin` header for ANY browser origin -- verified
// three ways (see the `radarpro-rainbow-cors-no-browser-support` memory):
// `no-cors` mode reaches the server fine, `cors` mode (what the real app
// code uses) always throws "Failed to fetch" in both a plain browser tab and
// this app's own webview, and a raw `OPTIONS` preflight returns 200 with
// zero `Access-Control-*` headers. This is not fixable in JS/browser code --
// the webview's `fetch()`/XHR always enforces CORS, full stop.
//
// Native HTTP has no CORS enforcement at all, so these commands do the
// actual request on the Rust side (this same user's own API key, on their
// own machine -- not a public relay; GLOBAL_CONTRACT's "no public relay for
// a keyed provider" rule is about redistributing *other* users' access, not
// moving where one user's own authenticated request originates) and hand
// the result back to the webview:
//   - `rainbow_get_snapshot` -- the native transport for the documented
//     `GET /tiles/v1/snapshot?layer=<layer>` call (KB §4.1). Used by
//     `snapshot.ts`'s `resolveRainbowSnapshot` to seed its starting
//     candidate boundary instead of only ever locally guessing the latest
//     10-minute boundary -- the existing probe-and-step-back loop still runs
//     from there, since this endpoint's answer can still be momentarily
//     ahead of what's actually published for tile fetching.
//   - `rainbow_probe_tile` -- the native transport for `snapshot.ts`'s
//     snapshot-availability probe (does a given snapshot/forecast_time/tile
//     exist?). Returns only the HTTP status code; `resolveRainbowSnapshot`'s
//     existing retry/fallback policy still decides what a status means.
//   - `rainbow_fetch_tile` -- fetches one tile's actual PNG bytes, for
//     `apps/web`'s `rainbow-tile://` `maplibregl.addProtocol` handler.
//
// All three are layer-aware (`precip`, `precip-global`, `clouds`, `radars`
// -- KB §4/§8) via `RainbowLayer`/`rainbow_tile_url` below, generalized from
// this module's original precip-only `precip_tile_url`.
//
// The API key is passed in per-call from JS (see
// `apps/web/src/rainbow/desktopTiles.ts`) and used only to build that one
// request's URL -- never logged, stored, or echoed back in an error message
// (see `sanitize_reqwest_error`'s doc comment for why that needs care).
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Mirrors `apps/web/src/rainbow/config.ts`'s `RAINBOW_API_BASE`. Kept as a
/// separate literal (not shared across the JS/Rust boundary) since this
/// crate intentionally has no build-time coupling to `apps/web`'s
/// TypeScript -- see `lib.rs`'s module doc comment on this shell being a
/// thin host, not a reimplementation.
pub(crate) const RAINBOW_API_BASE: &str = "https://api.rainbow.ai";

/// The four documented Tiles API layers (KB §4, §8's `TilesLayer` enum).
/// Each layer has its own URL shape and query-parameter rules -- see the
/// per-variant methods below, all sourced from KB §4.2-§4.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RainbowLayer {
    Precip,
    PrecipGlobal,
    Clouds,
    Radars,
}

impl RainbowLayer {
    /// The literal path segment this layer occupies in
    /// `/tiles/v1/<segment>/...` -- also this layer's wire identifier as
    /// used by `apps/web/src/rainbow/types.ts`'s `RainbowTileLayer` union
    /// (kept as matching string literals by convention, not a shared type).
    fn path_segment(self) -> &'static str {
        match self {
            RainbowLayer::Precip => "precip",
            RainbowLayer::PrecipGlobal => "precip-global",
            RainbowLayer::Clouds => "clouds",
            RainbowLayer::Radars => "radars",
        }
    }

    /// KB §4.4/§4.5 (also FAQ §10 "What zoom levels are supported?"):
    /// `precip`/`precip-global` go to 12, `clouds`/`radars` stop at 7.
    fn max_zoom(self) -> u8 {
        match self {
            RainbowLayer::Precip | RainbowLayer::PrecipGlobal => 12,
            RainbowLayer::Clouds | RainbowLayer::Radars => 7,
        }
    }

    /// KB §4.2/§4.3 vs §4.4/§4.5: only `precip`/`precip-global` have a
    /// `forecast_time` path segment; `clouds`/`radars` are observation-only.
    fn supports_forecast_time(self) -> bool {
        matches!(self, RainbowLayer::Precip | RainbowLayer::PrecipGlobal)
    }

    /// KB §4.2/§4.3/§4.5: `color`/`coverage` apply to `precip`,
    /// `precip-global`, and `radars`. `clouds` (§4.4) documents neither
    /// param -- omitted entirely for that layer, not just defaulted.
    fn supports_color_coverage(self) -> bool {
        !matches!(self, RainbowLayer::Clouds)
    }

    /// KB §4.5 only: `use_precip_type` is a `radars`-only query param.
    fn supports_use_precip_type(self) -> bool {
        matches!(self, RainbowLayer::Radars)
    }
}

impl FromStr for RainbowLayer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "precip" => Ok(RainbowLayer::Precip),
            "precip-global" => Ok(RainbowLayer::PrecipGlobal),
            "clouds" => Ok(RainbowLayer::Clouds),
            "radars" => Ok(RainbowLayer::Radars),
            other => Err(format!(
                "unknown Rainbow tile layer \"{other}\" -- expected one of precip, precip-global, clouds, radars"
            )),
        }
    }
}

/// Rainbow tile snapshots are epoch-UTC-seconds aligned to a 10-minute
/// boundary (KB §3/§4.2), the same constant `apps/web/src/rainbow/
/// snapshot.ts`'s `SNAPSHOT_STEP_SECONDS` uses.
const FORECAST_TIME_STEP_SECONDS: i64 = 600;
/// KB §4.2/§4.3: `forecast_time` tops out at 4 hours ahead.
const FORECAST_TIME_MAX_SECONDS: i64 = 14_400;

/// Ensure a global `rustls` crypto provider is installed exactly once, per
/// process. Required before any TLS connection because this crate builds
/// `reqwest` with `rustls-no-provider` (deliberately, to select `ring` over
/// the `aws-lc-rs` default -- same reasoning as `crates/radar-cache`'s own
/// `ensure_crypto_provider_installed`, which this mirrors: that crate isn't
/// a dependency here since `apps/desktop` is intentionally its own isolated
/// Cargo workspace, see this crate's `Cargo.toml`). Idempotent and safe to
/// call from multiple client-construction call sites -- `install_default`
/// errors if a provider is already installed, which this treats as success,
/// not a failure. Without this, every `reqwest::Client` request panics at
/// first use with "No rustls crypto provider is configured" (confirmed
/// during this feature's own live verification).
///
/// # Real startup race with `tauri-plugin-updater` (found live, not assumed)
///
/// `lib.rs` also calls this once, deterministically, BEFORE
/// `tauri::Builder::default()` runs at all -- not just relying on the lazy
/// `client()` call site below. `tauri-plugin-updater` (registered as a
/// plugin in `lib.rs`) has its own internal `reqwest`/`rustls` HTTP stack
/// for checking the update endpoint, almost certainly built against
/// whatever `rustls`'s ecosystem-default provider is (`aws-lc-rs`), not
/// `ring`. Left to each crate's own lazy first-use initialization, the two
/// stacks raced to install the process-global crypto provider singleton --
/// confirmed live: the very first Rainbow-tile toggle after a fresh app
/// launch intermittently panicked deep inside `reqwest`'s client code
/// (`async_impl/client.rs`), while every later attempt in the same process
/// succeeded (by then, whichever provider had already "won" was being used
/// consistently, avoiding the race). Installing `ring` as the default here,
/// first, before either plugin/command has a chance to run, makes the
/// outcome deterministic instead of a startup-order race.
pub(crate) fn ensure_crypto_provider_installed() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

/// A single shared client (connection pooling across repeated tile
/// requests as the user pans/zooms) rather than building one per call.
/// `pub(crate)` so `rainbow_weather.rs`'s Nowcast/Weather commands (S09d
/// Parts B/C) reuse this same pooled client and its crypto-provider
/// initialization instead of standing up a second one.
pub(crate) fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        ensure_crypto_provider_installed();
        reqwest::Client::builder().build().expect("failed to build reqwest client")
    })
}

/// Builds one Tiles API tile URL for `(layer, snapshot, forecast_time, z, x,
/// y)` plus the layer's applicable query params -- generalizes this
/// module's original precip-only `precip_tile_url` to all four documented
/// layers (KB §4.2-§4.5). Uses `Url`'s own query-pair encoding rather than
/// hand-rolled percent-encoding (the query-param auth method is the one
/// already confirmed to reach the server; a header here would be an
/// untested auth path for no benefit, since native HTTP has no preflight to
/// avoid in the first place).
///
/// Validates rather than silently clamps two documented constraints so a
/// caller mistake is surfaced, not hidden:
/// - `z` against the layer's own zoom ceiling (`max_zoom` -- 12 for
///   precip/precip-global, 7 for clouds/radars, KB §4.4/§4.5/FAQ §10).
/// - `forecast_time` against `[0, 14400]` step `600` for layers that have
///   that dimension at all (KB §4.2/§4.3), and against "must be 0" for
///   layers that don't (`clouds`/`radars` have no `forecast_time` path
///   segment at all -- a non-zero value here can never take effect, so
///   accepting it silently would hide a caller bug rather than surface it).
///
/// `color`/`coverage`/`use_precip_type` are each included only when the
/// layer documents that param (`RainbowLayer`'s `supports_*` methods) --
/// passed but inapplicable values (e.g. a `color` alongside `clouds`) are
/// silently dropped rather than erroring, since the frontend's UI already
/// hides those controls per-layer (S09d Part A) and a stale/leftover value
/// from a prior layer selection is expected, not a bug to surface.
#[allow(clippy::too_many_arguments)]
fn rainbow_tile_url(
    layer: RainbowLayer,
    snapshot: i64,
    forecast_time: i64,
    z: u8,
    x: u32,
    y: u32,
    color: Option<&str>,
    coverage: bool,
    use_precip_type: bool,
    api_key: &str,
) -> Result<reqwest::Url, String> {
    if z > layer.max_zoom() {
        return Err(format!(
            "zoom {z} exceeds layer \"{}\"'s maximum supported zoom ({}) -- see KB §4.4/§4.5",
            layer.path_segment(),
            layer.max_zoom()
        ));
    }
    if layer.supports_forecast_time() {
        if !(0..=FORECAST_TIME_MAX_SECONDS).contains(&forecast_time) || forecast_time % FORECAST_TIME_STEP_SECONDS != 0 {
            return Err(format!(
                "forecast_time {forecast_time} is invalid for layer \"{}\" -- must be in [0, {FORECAST_TIME_MAX_SECONDS}], step {FORECAST_TIME_STEP_SECONDS}",
                layer.path_segment()
            ));
        }
    } else if forecast_time != 0 {
        return Err(format!(
            "layer \"{}\" has no forecast_time dimension (KB §4.4/§4.5) -- pass 0",
            layer.path_segment()
        ));
    }

    let mut url = reqwest::Url::parse(RAINBOW_API_BASE).map_err(|_| "internal error building Rainbow URL".to_string())?;
    let mut path = format!("/tiles/v1/{}/{snapshot}/", layer.path_segment());
    if layer.supports_forecast_time() {
        path.push_str(&format!("{forecast_time}/"));
    }
    path.push_str(&format!("{z}/{x}/{y}"));
    url.set_path(&path);

    {
        let mut pairs = url.query_pairs_mut();
        if layer.supports_color_coverage() {
            if let Some(c) = color {
                pairs.append_pair("color", c);
            }
            if coverage {
                pairs.append_pair("coverage", "1");
            }
        }
        if layer.supports_use_precip_type() && use_precip_type {
            pairs.append_pair("use_precip_type", "1");
        }
        pairs.append_pair("token", api_key);
    }
    Ok(url)
}

/// Turns a `reqwest::Error` into a message safe to return to JS (and from
/// there, potentially into the on-disk log file via `attachDesktopLogging`'s
/// console forwarding -- see `apps/web/src/platform/desktop.ts`).
/// Deliberately never uses `err.to_string()` or `err.url()`: `reqwest`
/// error `Display` output can include the failed request's URL, which here
/// always carries the caller's API key in its `?token=` query parameter.
/// GLOBAL_CONTRACT: "never log or expose the API key value anywhere ...
/// beyond what's strictly needed to make the authenticated request."
/// `pub(crate)` so `rainbow_weather.rs`'s Nowcast/Weather commands (S09d
/// Parts B/C) share this one sanitization rule instead of a second copy.
pub(crate) fn sanitize_reqwest_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "timed out contacting Rainbow Weather".to_string()
    } else if err.is_connect() {
        "could not connect to Rainbow Weather".to_string()
    } else if err.is_decode() {
        "invalid response from Rainbow Weather".to_string()
    } else if err.is_builder() {
        "internal error building the Rainbow request".to_string()
    } else {
        "network error contacting Rainbow Weather".to_string()
    }
}

/// Wire shape of `GET /tiles/v1/snapshot`'s `MapSnapshot` response (KB
/// §4.1): `{ "snapshot": <epoch seconds> }`.
#[derive(Deserialize)]
struct MapSnapshotResponse {
    snapshot: i64,
}

/// Native-HTTP transport for `GET /tiles/v1/snapshot?layer=<layer>` (KB
/// §4.1) -- returns the layer's latest published snapshot timestamp
/// directly, so `snapshot.ts`'s `resolveRainbowSnapshot` can seed its
/// starting candidate from the documented source of truth instead of only
/// ever locally guessing the latest 10-minute boundary. The existing
/// probe-and-step-back loop there still runs afterward using this as the
/// first candidate -- this endpoint's answer can still be momentarily ahead
/// of what `/tiles/v1/<layer>/...` will actually serve, so it is a better
/// starting guess, not a replacement for confirming the tile itself exists.
#[tauri::command]
pub async fn rainbow_get_snapshot(layer: String, api_key: String) -> Result<i64, String> {
    let layer = RainbowLayer::from_str(&layer)?;
    let mut url = reqwest::Url::parse(RAINBOW_API_BASE).map_err(|_| "internal error building Rainbow URL".to_string())?;
    url.set_path("/tiles/v1/snapshot");
    url.query_pairs_mut()
        .append_pair("layer", layer.path_segment())
        .append_pair("token", &api_key);

    let response = client().get(url).send().await.map_err(|e| sanitize_reqwest_error(&e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let body: MapSnapshotResponse = response.json().await.map_err(|e| sanitize_reqwest_error(&e))?;
    Ok(body.snapshot)
}

/// Native-HTTP equivalent of `snapshot.ts`'s `probeOnce` -- checks whether a
/// given (layer, snapshot, forecast_time) tile is published, without
/// downloading or returning its body. Returns the raw HTTP status code on
/// any completed response (including 404 -- that is a normal, expected
/// outcome for the newest boundary, not an error); only a genuine network
/// failure, or an invalid parameter combination caught by
/// {@link rainbow_tile_url} (bad zoom/forecast_time), is an `Err`. The
/// frontend's existing retry/fallback policy (`resolveRainbowSnapshot`) is
/// unchanged by this command -- it is transport only, matching this
/// command's counterpart on the browser path (`browserProbeTile`, which
/// does the same GET-and-inspect-status dance via `fetch`).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn rainbow_probe_tile(
    layer: String,
    snapshot: i64,
    forecast_time: i64,
    z: u8,
    x: u32,
    y: u32,
    color: Option<String>,
    coverage: bool,
    use_precip_type: bool,
    api_key: String,
) -> Result<u16, String> {
    let layer = RainbowLayer::from_str(&layer)?;
    let url = rainbow_tile_url(layer, snapshot, forecast_time, z, x, y, color.as_deref(), coverage, use_precip_type, &api_key)?;
    let response = client().get(url).send().await.map_err(|e| sanitize_reqwest_error(&e))?;
    Ok(response.status().as_u16())
}

/// One fetched Rainbow tile, returned to JS as base64 (see this module's
/// doc comment on why -- no binary-IPC convention exists yet in this
/// crate). `content_type` lets the frontend hand MapLibre the real MIME
/// type instead of assuming `image/png`.
#[derive(Serialize)]
pub struct RainbowTile {
    data_base64: String,
    content_type: Option<String>,
}

/// Fetches one Rainbow tile's raw image bytes for `apps/web`'s
/// `rainbow-tile://` `maplibregl.addProtocol` handler
/// (`apps/web/src/rainbow/desktopTiles.ts`). A non-2xx response is an
/// `Err(format!("HTTP {status}"))` (never the sanitized-but-still-vague
/// network-error string, since a real HTTP status is not sensitive and is
/// useful to see in the UI/logs); a genuine network failure uses the same
/// `sanitize_reqwest_error` as {@link rainbow_probe_tile}.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn rainbow_fetch_tile(
    layer: String,
    snapshot: i64,
    forecast_time: i64,
    z: u8,
    x: u32,
    y: u32,
    color: Option<String>,
    coverage: bool,
    use_precip_type: bool,
    api_key: String,
) -> Result<RainbowTile, String> {
    use base64::Engine as _;

    let layer = RainbowLayer::from_str(&layer)?;
    let url = rainbow_tile_url(layer, snapshot, forecast_time, z, x, y, color.as_deref(), coverage, use_precip_type, &api_key)?;
    let response = client().get(url).send().await.map_err(|e| sanitize_reqwest_error(&e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = response.bytes().await.map_err(|e| sanitize_reqwest_error(&e))?;
    Ok(RainbowTile {
        data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
        content_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn url(
        layer: &str,
        snapshot: i64,
        forecast_time: i64,
        z: u8,
        x: u32,
        y: u32,
        color: Option<&str>,
        coverage: bool,
        use_precip_type: bool,
    ) -> Result<String, String> {
        let layer = RainbowLayer::from_str(layer)?;
        rainbow_tile_url(layer, snapshot, forecast_time, z, x, y, color, coverage, use_precip_type, "KEY").map(|u| u.to_string())
    }

    #[test]
    fn precip_builds_forecast_time_color_and_coverage() {
        let got = url("precip", 1_754_991_000, 600, 7, 68, 42, Some("3"), true, false).unwrap();
        assert_eq!(
            got,
            "https://api.rainbow.ai/tiles/v1/precip/1754991000/600/7/68/42?color=3&coverage=1&token=KEY"
        );
    }

    #[test]
    fn precip_global_has_same_shape_as_precip() {
        let got = url("precip-global", 1_754_991_000, 0, 0, 0, 0, None, false, false).unwrap();
        assert_eq!(got, "https://api.rainbow.ai/tiles/v1/precip-global/1754991000/0/0/0/0?token=KEY");
    }

    #[test]
    fn clouds_has_no_forecast_time_segment_and_no_color_coverage() {
        // forecast_time must be passed as 0 (no dimension to carry it) and
        // is correctly omitted from the path; a `color`/`coverage` value
        // supplied anyway is silently dropped, not erroring, since KB §4.4
        // documents neither param for this layer.
        let got = url("clouds", 1_754_991_000, 0, 7, 3, 2, Some("3"), true, false).unwrap();
        assert_eq!(got, "https://api.rainbow.ai/tiles/v1/clouds/1754991000/7/3/2?token=KEY");
    }

    #[test]
    fn clouds_rejects_nonzero_forecast_time_instead_of_ignoring_it() {
        let err = url("clouds", 1_754_991_000, 600, 0, 0, 0, None, false, false).unwrap_err();
        assert!(err.contains("no forecast_time dimension"), "unexpected error: {err}");
    }

    #[test]
    fn radars_builds_color_coverage_and_use_precip_type() {
        let got = url("radars", 1_754_991_000, 0, 7, 1, 1, Some("dbz_u8"), true, true).unwrap();
        assert_eq!(
            got,
            "https://api.rainbow.ai/tiles/v1/radars/1754991000/7/1/1?color=dbz_u8&coverage=1&use_precip_type=1&token=KEY"
        );
    }

    #[test]
    fn radars_has_no_forecast_time_segment() {
        let got = url("radars", 1_754_991_000, 0, 0, 0, 0, None, false, false).unwrap();
        assert_eq!(got, "https://api.rainbow.ai/tiles/v1/radars/1754991000/0/0/0?token=KEY");
    }

    #[test]
    fn use_precip_type_ignored_for_non_radars_layers() {
        let got = url("precip", 1_754_991_000, 0, 0, 0, 0, None, false, true).unwrap();
        assert!(!got.contains("use_precip_type"), "use_precip_type leaked onto precip: {got}");
    }

    #[test]
    fn precip_zoom_ceiling_is_twelve() {
        assert!(url("precip", 1_754_991_000, 0, 12, 0, 0, None, false, false).is_ok());
        let err = url("precip", 1_754_991_000, 0, 13, 0, 0, None, false, false).unwrap_err();
        assert!(err.contains("zoom 13"), "unexpected error: {err}");
    }

    #[test]
    fn precip_global_zoom_ceiling_is_twelve() {
        assert!(url("precip-global", 1_754_991_000, 0, 12, 0, 0, None, false, false).is_ok());
        assert!(url("precip-global", 1_754_991_000, 0, 13, 0, 0, None, false, false).is_err());
    }

    #[test]
    fn clouds_zoom_ceiling_is_seven() {
        assert!(url("clouds", 1_754_991_000, 0, 7, 0, 0, None, false, false).is_ok());
        let err = url("clouds", 1_754_991_000, 0, 8, 0, 0, None, false, false).unwrap_err();
        assert!(err.contains("zoom 8"), "unexpected error: {err}");
    }

    #[test]
    fn radars_zoom_ceiling_is_seven() {
        assert!(url("radars", 1_754_991_000, 0, 7, 0, 0, None, false, false).is_ok());
        assert!(url("radars", 1_754_991_000, 0, 8, 0, 0, None, false, false).is_err());
    }

    #[test]
    fn forecast_time_must_be_step_aligned() {
        let err = url("precip", 1_754_991_000, 601, 0, 0, 0, None, false, false).unwrap_err();
        assert!(err.contains("step 600"), "unexpected error: {err}");
    }

    #[test]
    fn forecast_time_must_not_exceed_four_hours() {
        let err = url("precip", 1_754_991_000, 14_400 + 600, 0, 0, 0, None, false, false).unwrap_err();
        assert!(err.contains("14400"), "unexpected error: {err}");
    }

    #[test]
    fn forecast_time_zero_is_valid_for_forecast_capable_layers() {
        assert!(url("precip", 1_754_991_000, 0, 0, 0, 0, None, false, false).is_ok());
    }

    #[test]
    fn unknown_layer_is_rejected() {
        assert!(RainbowLayer::from_str("bogus").is_err());
    }

    #[test]
    fn map_snapshot_response_parses_documented_shape() {
        let parsed: MapSnapshotResponse = serde_json::from_str(r#"{"snapshot": 1754991000}"#).unwrap();
        assert_eq!(parsed.snapshot, 1_754_991_000);
    }
}
