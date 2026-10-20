//! Pure CPU-side camera math: builds the `clip_to_world` orthographic
//! matrix uploaded to the shader's `Uniforms.clip_to_world`.
//!
//! No `wgpu` dependency — this is plain 4x4 matrix arithmetic, fully
//! unit-testable without a GPU. The measurement harness varies the
//! `center_km`/`half_extent_km` inputs frame-to-frame to simulate a
//! pan/zoom camera without touching any GPU resource other than the small
//! uniform buffer this matrix is uploaded into.

/// A 4x4 matrix stored column-major (`columns[i]` is column `i`), matching
/// WGSL's `mat4x4<f32>` in-memory layout: four consecutive 16-byte
/// columns. This is the layout `bytemuck` will cast directly into the
/// uniform buffer — no transposition needed on either side.
pub type Mat4 = [[f32; 4]; 4];

/// Build the `clip_to_world` matrix for an orthographic camera centered at
/// `center_km` (site-relative x, y, in km) covering `half_extent_km`
/// (half-width, half-height, in km) from the viewport center to its edge.
///
/// Applied to a clip-space point `(ndc_x, ndc_y, 0, 1)` (both components
/// in `[-1, 1]`), this produces `(world_x, world_y, 0, 1)` where:
/// - `world_x = ndc_x * half_extent_km.0 + center_km.0`
/// - `world_y = ndc_y * half_extent_km.1 + center_km.1`
///
/// This is the CPU-supplied orthographic projection the S03 stage doc
/// calls for: the shader only ever needs its inverse (pixel -> world), so
/// that inverse is what is actually built and uploaded, rather than a
/// forward world-to-clip matrix the shader would have to invert per pixel.
pub fn clip_to_world(center_km: (f32, f32), half_extent_km: (f32, f32)) -> Mat4 {
    [
        [half_extent_km.0, 0.0, 0.0, 0.0],
        [0.0, half_extent_km.1, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [center_km.0, center_km.1, 0.0, 1.0],
    ]
}

/// Same sphere `radar_geo` and the sweep shader use.
const EARTH_RADIUS_KM: f64 = 6371.0088;

/// Web Mercator y of `lat_rad`: `ln(tan(pi/4 + lat/2))`.
pub fn mercator_y(lat_rad: f64) -> f64 {
    (std::f64::consts::FRAC_PI_4 + lat_rad / 2.0).tan().ln()
}

/// `clip_to_world` for a canvas covering the lon/lat box `west..east`,
/// `south..north` on a Web Mercator map, in the sweep shader's world units:
/// Mercator relative to the site at `(site_lat_deg, site_lon_deg)`, scaled to
/// km at the equator. Computed in f64 so only the final, site-relative
/// values are rounded to f32.
pub fn mercator_view(
    site_lat_deg: f64,
    site_lon_deg: f64,
    west_lon: f64,
    north_lat: f64,
    east_lon: f64,
    south_lat: f64,
) -> Mat4 {
    let site_y = mercator_y(site_lat_deg.to_radians());
    let x = |lon: f64| (lon - site_lon_deg).to_radians() * EARTH_RADIUS_KM;
    let y = |lat: f64| (mercator_y(lat.to_radians()) - site_y) * EARTH_RADIUS_KM;
    let (left, right, top, bottom) = (x(west_lon), x(east_lon), y(north_lat), y(south_lat));
    clip_to_world(
        (((left + right) / 2.0) as f32, ((top + bottom) / 2.0) as f32),
        (((right - left) / 2.0) as f32, ((top - bottom) / 2.0) as f32),
    )
}

/// A square [`mercator_view`] around the site reaching at least `range_km`
/// in every direction: the default framing before the map reports a view.
pub fn full_range_view(site_lat_deg: f64, range_km: f32) -> Mat4 {
    // Mercator stretches distances by 1/cos(lat); the extra 10% covers the
    // stretch growing toward the pole over the sweep's own north-south span.
    let half = f64::from(range_km) / site_lat_deg.to_radians().cos() * 1.1;
    clip_to_world((0.0, 0.0), (half as f32, half as f32))
}

/// Apply `matrix` to `(ndc_x, ndc_y, 0.0, 1.0)`, returning the resulting
/// `(x, y)` — used by this module's own tests to verify
/// [`clip_to_world`] without needing the shader to run on a GPU.
#[cfg(test)]
fn apply(matrix: &Mat4, ndc_x: f32, ndc_y: f32) -> (f32, f32) {
    let v = [ndc_x, ndc_y, 0.0, 1.0];
    let mut out = [0.0f32; 4];
    for (row, out_component) in out.iter_mut().enumerate() {
        *out_component = matrix[0][row] * v[0]
            + matrix[1][row] * v[1]
            + matrix[2][row] * v[2]
            + matrix[3][row] * v[3];
    }
    (out[0], out[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_ndc_maps_to_camera_center() {
        let m = clip_to_world((12.0, -34.0), (100.0, 100.0));
        assert_eq!(apply(&m, 0.0, 0.0), (12.0, -34.0));
    }

    #[test]
    fn ndc_corners_map_to_expected_world_extent() {
        let m = clip_to_world((0.0, 0.0), (200.0, 150.0));
        assert_eq!(apply(&m, -1.0, -1.0), (-200.0, -150.0));
        assert_eq!(apply(&m, 1.0, 1.0), (200.0, 150.0));
        assert_eq!(apply(&m, 1.0, -1.0), (200.0, -150.0));
    }

    #[test]
    fn zoom_changes_extent_but_not_center() {
        let zoomed_in = clip_to_world((5.0, 5.0), (10.0, 10.0));
        let zoomed_out = clip_to_world((5.0, 5.0), (400.0, 400.0));

        assert_eq!(apply(&zoomed_in, 0.0, 0.0), apply(&zoomed_out, 0.0, 0.0));
        let (near_x, _) = apply(&zoomed_in, 1.0, 0.0);
        let (far_x, _) = apply(&zoomed_out, 1.0, 0.0);
        assert!(far_x - 5.0 > near_x - 5.0);
    }

    #[test]
    fn mercator_view_frames_the_box_around_the_site() {
        // A box centred on the site at the equator is centred on world (0, 0),
        // one degree of longitude wide = 111.2 km at the equator.
        let m = mercator_view(0.0, -100.0, -100.5, 0.5, -99.5, -0.5);
        let (cx, cy) = apply(&m, 0.0, 0.0);
        assert!(cx.abs() < 1e-3 && cy.abs() < 1e-3, "centre {cx},{cy}");
        let (right, top) = apply(&m, 1.0, 1.0);
        assert!((right - 55.6).abs() < 0.1, "right {right}");
        // Mercator y of 0.5 deg is a hair more than its x.
        assert!(top > right && top - right < 0.01, "top {top}");

        // At 35N the same degree of longitude still spans 111.2 Mercator km,
        // and a box north of the site sits at positive y.
        let m = mercator_view(35.0, -100.0, -100.5, 36.5, -99.5, 35.5);
        let (cx, cy) = apply(&m, 0.0, 0.0);
        assert!(cx.abs() < 1e-3, "centre x {cx}");
        let expected_y =
            (mercator_y(36.0f64.to_radians()) - mercator_y(35.0f64.to_radians())) * EARTH_RADIUS_KM;
        assert!(
            (f64::from(cy) - expected_y).abs() < 0.5,
            "centre y {cy} vs {expected_y}"
        );
    }
}
