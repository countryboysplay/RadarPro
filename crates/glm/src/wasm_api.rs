//! Browser API: one call per downloaded LCFA file.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
fn on_wasm_module_init() {
    console_error_panic_hook::set_once();
}

/// The file's flashes as a flat `Float64Array`: `[lon, lat, timeMs, lon, lat, timeMs, ...]`.
#[wasm_bindgen(js_name = decodeGlmFlashes)]
pub fn decode_glm_flashes(bytes: Vec<u8>) -> Result<Vec<f64>, JsValue> {
    let flashes = crate::decode_flashes(bytes).map_err(|e| JsValue::from_str(&e))?;
    Ok(flashes
        .iter()
        .flat_map(|f| [f64::from(f.lon), f64::from(f.lat), f.time_ms])
        .collect())
}
