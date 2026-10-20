//! NWS Level 3 storm products from tgftp. tgftp sends no CORS headers, so
//! the webview can't fetch them itself; this relays the raw bytes. `fetch_url`
//! does the same for placefiles.

use crate::rainbow::{client, sanitize_reqwest_error};

const TGFTP_RADAR: &str = "https://tgftp.nws.noaa.gov/SL.us008001/DF.of/DC.radar";
/// Storm tracks, mesocyclone, TVS, hail -- the only products the frontend may ask for.
const PRODUCTS: [&str; 4] = ["58sti", "141md", "61tvs", "p59hi"];

/// Latest issue of `product` for `site` (lowercase ICAO, e.g. `ktlx`).
#[tauri::command]
pub async fn nws_radar_product(product: String, site: String) -> Result<tauri::ipc::Response, String> {
    if !PRODUCTS.contains(&product.as_str()) {
        return Err(format!("unsupported product {product}"));
    }
    if site.len() != 4 || !site.bytes().all(|b| b.is_ascii_lowercase()) {
        return Err("site must be a lowercase 4-letter ICAO".into());
    }
    let url = format!("{TGFTP_RADAR}/DS.{product}/SI.{site}/sn.last");
    let response = client().get(url).send().await.map_err(|e| sanitize_reqwest_error(&e))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let bytes = response.bytes().await.map_err(|e| sanitize_reqwest_error(&e))?;
    Ok(tauri::ipc::Response::new(bytes.to_vec()))
}

/// Placefiles and their icon sheets are larger than this only by mistake.
const FETCH_URL_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Raw bytes of an http(s) URL: placefile servers send no CORS headers.
#[tauri::command]
pub async fn fetch_url(url: String) -> Result<tauri::ipc::Response, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("only http and https URLs".into());
    }
    let unreachable = |e: reqwest::Error| if e.is_timeout() { "timed out".to_string() } else { "could not reach the server".to_string() };
    let mut response = client().get(url).timeout(std::time::Duration::from_secs(20)).send().await.map_err(unreachable)?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unreachable)? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > FETCH_URL_MAX_BYTES {
            return Err("file is over 8 MB".into());
        }
    }
    Ok(tauri::ipc::Response::new(bytes))
}
