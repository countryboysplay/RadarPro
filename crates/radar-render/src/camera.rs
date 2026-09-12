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
}
