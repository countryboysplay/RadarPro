// RadarPro desktop shell -- Rainbow Weather tile CORS-bypass (S10 follow-up).
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
// Native HTTP has no CORS enforcement at all, so these two commands do the
// actual request on the Rust side (this same user's own API key, on their
// own machine -- not a public relay; GLOBAL_CONTRACT's "no public relay for
// a keyed provider" rule is about redistributing *other* users' access, not
// moving where one user's own authenticated request originates) and hand
// the result back to the webview:
//   - `rainbow_probe_tile` -- the native transport for `snapshot.ts`'s
//     snapshot-availability probe (does a given snapshot/forecast_time/tile
//     exist?). Returns only the HTTP status code; `resolveRainbowSnapshot`'s
//     existing retry/fallback policy still decides what a status means.
//   - `rainbow_fetch_tile` -- fetches one tile's actual PNG bytes, for
//     `apps/web`'s `rainbow-tile://` `maplibregl.addProtocol` handler.
//
// The API key is passed in per-call from JS (see
// `apps/web/src/rainbow/desktopTiles.ts`) and used only to build that one
// request's URL -- never logged, stored, or echoed back in an error message
// (see `sanitize_reqwest_error`'s doc comment for why that needs care).
use serde::Serialize;

/// Mirrors `apps/web/src/rainbow/config.ts`'s `RAINBOW_API_BASE`. Kept as a
/// separate literal (not shared across the JS/Rust boundary) since this
/// crate intentionally has no build-time coupling to `apps/web`'s
/// TypeScript -- see `lib.rs`'s module doc comment on this shell being a
/// thin host, not a reimplementation.
const RAINBOW_API_BASE: &str = "https://api.rainbow.ai";

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
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        ensure_crypto_provider_installed();
        reqwest::Client::builder().build().expect("failed to build reqwest client")
    })
}

/// Builds the regional precip tile URL for one (snapshot, forecast_time,
/// z, x, y) -- same `/tiles/v1/precip/...` shape and `?token=` query-param
/// auth as `apps/web/src/rainbow/snapshot.ts`'s `precipTileUrl`/
/// `buildRainbowPrecipTileUrlTemplate` (the query-param method is the one
/// already confirmed to reach the server; switching to a header here would
/// be an untested auth path for no benefit, since native HTTP has no
/// preflight to avoid in the first place). Uses `Url`'s own query-pair
/// encoding rather than hand-rolled percent-encoding.
fn precip_tile_url(snapshot: i64, forecast_time: i64, z: u8, x: u32, y: u32, api_key: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(RAINBOW_API_BASE).map_err(|_| "internal error building Rainbow URL".to_string())?;
    url.set_path(&format!("/tiles/v1/precip/{snapshot}/{forecast_time}/{z}/{x}/{y}"));
    url.query_pairs_mut().append_pair("token", api_key);
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
fn sanitize_reqwest_error(err: &reqwest::Error) -> String {
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

/// Native-HTTP equivalent of `snapshot.ts`'s `probeOnce` -- checks whether a
/// given (snapshot, forecast_time) tile is published, without downloading
/// or returning its body. Returns the raw HTTP status code on any completed
/// response (including 404 -- that is a normal, expected outcome for the
/// newest boundary, not an error); only a genuine network failure is an
/// `Err`. The frontend's existing retry/fallback policy
/// (`resolveRainbowSnapshot`) is unchanged by this command -- it is transport
/// only, matching this command's counterpart on the browser path
/// (`probeOnce`, which does the same GET-and-inspect-status dance via
/// `fetch`).
#[tauri::command]
pub async fn rainbow_probe_tile(
    snapshot: i64,
    forecast_time: i64,
    z: u8,
    x: u32,
    y: u32,
    api_key: String,
) -> Result<u16, String> {
    let url = precip_tile_url(snapshot, forecast_time, z, x, y, &api_key)?;
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

/// Fetches one Rainbow precip tile's raw image bytes for
/// `apps/web`'s `rainbow-tile://` `maplibregl.addProtocol` handler
/// (`apps/web/src/rainbow/desktopTiles.ts`). A non-2xx response is an
/// `Err(format!("HTTP {status}"))` (never the sanitized-but-still-vague
/// network-error string, since a real HTTP status is not sensitive and is
/// useful to see in the UI/logs); a genuine network failure uses the same
/// `sanitize_reqwest_error` as {@link rainbow_probe_tile}.
#[tauri::command]
pub async fn rainbow_fetch_tile(
    snapshot: i64,
    forecast_time: i64,
    z: u8,
    x: u32,
    y: u32,
    api_key: String,
) -> Result<RainbowTile, String> {
    use base64::Engine as _;

    let url = precip_tile_url(snapshot, forecast_time, z, x, y, &api_key)?;
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
