//! GOES Geostationary Lightning Mapper (GLM) Level 2 flashes.
//!
//! NOAA publishes one `GLM-L2-LCFA` file every 20 s per satellite
//! (`noaa-goes19` on AWS is GOES-East, `noaa-goes18` GOES-West). Each is netCDF4, i.e. HDF5, read
//! here with the pure-Rust `hdf5-pure` so the same code runs in the browser.
//! Only the flash level is read: where each flash was and when it began.

use hdf5_pure::{AttrValue, File};

#[cfg(target_arch = "wasm32")]
mod wasm_api;

/// Unix seconds at GOES's epoch, 2000-01-01 12:00:00 UTC (`product_time`'s
/// units: "seconds since 2000-01-01 12:00:00").
const GOES_EPOCH_UNIX_SECONDS: f64 = 946_728_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flash {
    pub lon: f32,
    pub lat: f32,
    /// When the flash's first optical event was seen, Unix milliseconds.
    pub time_ms: f64,
}

/// First value of a float attribute; netCDF stores scalars as 1-element arrays.
fn attr_f32(
    attrs: &std::collections::HashMap<String, AttrValue>,
    name: &str,
) -> Result<f32, String> {
    match attrs.get(name) {
        Some(AttrValue::F32(v)) => Ok(*v),
        Some(AttrValue::F32Array(v)) if !v.is_empty() => Ok(v[0]),
        other => Err(format!("flash time attribute {name}: {other:?}")),
    }
}

/// Every flash in one LCFA file.
pub fn decode_flashes(bytes: Vec<u8>) -> Result<Vec<Flash>, String> {
    let e = |err: hdf5_pure::Error| err.to_string();
    let file = File::from_bytes(bytes).map_err(e)?;
    let start = *file
        .dataset("product_time")
        .and_then(|d| d.read_f64())
        .map_err(e)?
        .first()
        .ok_or("empty product_time")?;
    let lat = file
        .dataset("flash_lat")
        .and_then(|d| d.read_f32())
        .map_err(e)?;
    let lon = file
        .dataset("flash_lon")
        .and_then(|d| d.read_f32())
        .map_err(e)?;
    let offsets = file
        .dataset("flash_time_offset_of_first_event")
        .map_err(e)?;
    let attrs = offsets.attrs().map_err(e)?;
    let (scale, add) = (
        attr_f32(&attrs, "scale_factor")?,
        attr_f32(&attrs, "add_offset")?,
    );
    let raw = offsets.read_i16().map_err(e)?;
    if lon.len() != lat.len() || raw.len() != lat.len() {
        return Err(format!(
            "flash arrays differ in length: {} {} {}",
            lat.len(),
            lon.len(),
            raw.len()
        ));
    }
    let start_unix = GOES_EPOCH_UNIX_SECONDS + start;
    Ok(lat
        .iter()
        .zip(&lon)
        .zip(&raw)
        .map(|((&lat, &lon), &raw)| Flash {
            lon,
            lat,
            // Stored as signed 16-bit but flagged `_Unsigned`.
            time_ms: (start_unix + f64::from(raw as u16) * f64::from(scale) + f64::from(add))
                * 1000.0,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_real_goes19_lcfa_file() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/glm/OR_GLM-L2-LCFA_G19_s20262931715000_e20262931715200_c20262931715218.nc");
        let flashes = decode_flashes(std::fs::read(path).unwrap()).unwrap();
        assert_eq!(flashes.len(), 291);
        // The file covers 17:15:00-17:15:20 UTC on 2026-10-20; a flash may have begun up to 5 s earlier.
        let start_ms = 1_792_516_500_000.0;
        assert!(flashes
            .iter()
            .all(|f| (start_ms - 5_000.0..start_ms + 20_000.0).contains(&f.time_ms)));
        assert!(flashes
            .iter()
            .all(|f| (-90.0..=90.0).contains(&f.lat) && (-180.0..=180.0).contains(&f.lon)));
        let flash = flashes[1];
        assert!((flash.lat - 19.980_974).abs() < 1e-4 && (flash.lon + 73.289_24).abs() < 1e-4);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_flashes(vec![0u8; 64]).is_err());
    }
}
