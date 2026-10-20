// S03 GPU radar renderer prototype shader.
//
// Per pixel: clip-space position -> site-relative world (x, y) via a
// CPU-supplied orthographic projection (its inverse, `clip_to_world`, so
// the shader goes directly from a pixel to world space). World is Web
// Mercator relative to the site, scaled to km at the equator, so a canvas
// laid over a Mercator map lines up with it at any zoom. Then: unproject to
// lat/lon -> great-circle ground range and bearing from the site -> slant
// range along the beam -> radial index via the lookup texture -> gate index
// via the same half-open-interval convention as
// `radar_geo::lookup::find_gate_index` -> raw/converted gate value ->
// distinct output for missing/range-folded, otherwise the palette ramp.
// The geometry matches `radar_geo::lookup::cursor_to_polar`, so the pixel
// under the cursor is the gate the probe reports.
//
// With `site_lat_rad = site_mercator_y = 0` (the native harness and GPU
// tests) world is, to within 0.3% at 460 km, the flat site-relative km plane
// this shader originally used.
//
// Never-fabricate rule: a pixel that resolves to "no radial" (lookup
// sentinel), "outside this radial's gate range", or a gate flagged
// missing/range-folded returns a color a valid dBZ palette lookup could
// never produce (fully transparent, or the reserved range-folded flag
// color below) — never a value pulled through the palette as if it were
// real data.

struct Uniforms {
    // Maps clip-space (NDC) (x, y, 0, 1) to site-relative world (x, y) in
    // km (z/w unused). Built on the CPU as an orthographic
    // scale + translate (see `render.rs::build_clip_to_world`); varying
    // this per frame simulates pan/zoom without touching any GPU
    // resource other than this uniform buffer.
    clip_to_world: mat4x4<f32>,
    // Ground range beyond which there is no sweep coverage at all
    // (the moment's farthest gate's far edge), in km.
    site_max_range_km: f32,
    // Number of texels in `radial_lookup` (its width).
    lookup_texel_count: u32,
    // Palette domain: `palette_tex` texel 0 corresponds to this physical
    // value, and its last texel corresponds to `palette_max_dbz`.
    palette_min_dbz: f32,
    palette_max_dbz: f32,
    // Site latitude (radians) and its Web Mercator y (`ln(tan(pi/4 + lat/2))`,
    // computed on the CPU in f64), for unprojecting world (x, y) to lat/lon.
    site_lat_rad: f32,
    site_mercator_y: f32,
    _pad0: u32,
    _pad1: u32,
    // The active color table's `range_folded_color`, straight (not
    // premultiplied) RGBA in 0..1.
    range_folded_color: vec4<f32>,
};

// Same sphere `radar_geo::spherical::distance_bearing` uses.
const EARTH_RADIUS_KM: f32 = 6371.0088;

struct RadialMeta {
    azimuth_deg: f32,
    elevation_deg: f32,
    first_gate_range_km: f32,
    gate_spacing_km: f32,
    gate_count: u32,
    gate_offset: u32,
};

struct GateSample {
    value: f32,
    flag: u32,
};

const GATE_FLAG_VALID: u32 = 0u;
const GATE_FLAG_RANGE_FOLDED: u32 = 2u;
const SENTINEL_NO_RADIAL: u32 = 4294967295u; // u32::MAX

const NO_DATA_COLOR: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 0.0);

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> radial_meta: array<RadialMeta>;
@group(0) @binding(2) var<storage, read> gate_samples: array<GateSample>;
@group(0) @binding(3) var radial_lookup: texture_1d<u32>;
@group(0) @binding(4) var palette_tex: texture_1d<f32>;
@group(0) @binding(5) var palette_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// Full-screen triangle from just `vertex_index` (no vertex buffer): a
// standard trick producing a triangle whose clip-space extent covers the
// entire viewport ([-1,-1] to [3,-1] to [-1,3]), so every pixel is
// rasterized by one triangle instead of two.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let uv = vec2<f32>(
        f32((vertex_index << 1u) & 2u),
        f32(vertex_index & 2u),
    );
    let ndc = uv * 2.0 - vec2<f32>(1.0, 1.0);

    var out: VertexOutput;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let world = uniforms.clip_to_world * vec4<f32>(in.ndc, 0.0, 1.0);

    // World is site-relative Web Mercator scaled to km at the equator (the
    // map's own projection), so unproject to lat/lon and take the true
    // great-circle range and bearing from the site, exactly as
    // `radar_geo::lookup::cursor_to_polar` does for the gate probe.
    let d_lambda = world.x / EARTH_RADIUS_KM;
    let phi1 = uniforms.site_lat_rad;
    let phi2 = 2.0 * atan(exp(world.y / EARTH_RADIUS_KM + uniforms.site_mercator_y)) - 1.5707963267948966;
    let d_phi = phi2 - phi1;
    let sin_half_lambda = sin(d_lambda * 0.5);
    let sin_half_phi = sin(d_phi * 0.5);
    let a = sin_half_phi * sin_half_phi + cos(phi1) * cos(phi2) * sin_half_lambda * sin_half_lambda;
    let ground_range_km = 2.0 * EARTH_RADIUS_KM * asin(min(sqrt(a), 1.0));
    if (ground_range_km > uniforms.site_max_range_km) {
        return NO_DATA_COLOR;
    }

    // Initial bearing clockwise from north, matching
    // `radar_types::Radial::azimuth_angle_deg`. The usual
    // `cos(phi1) sin(phi2) - sin(phi1) cos(phi2) cos(d_lambda)` cancels
    // badly in f32 near the site; this is the same value rewritten as
    // `sin(d_phi) + 2 sin(phi1) cos(phi2) sin^2(d_lambda / 2)`.
    let east = sin(d_lambda) * cos(phi2);
    let north = sin(d_phi) + 2.0 * sin(phi1) * cos(phi2) * sin_half_lambda * sin_half_lambda;
    var azimuth_deg = degrees(atan2(east, north));
    azimuth_deg = azimuth_deg - 360.0 * floor(azimuth_deg / 360.0);

    let texel_count_f = f32(uniforms.lookup_texel_count);
    var bucket = i32(azimuth_deg / 360.0 * texel_count_f);
    bucket = clamp(bucket, 0, i32(uniforms.lookup_texel_count) - 1);

    let radial_index = textureLoad(radial_lookup, bucket, 0).r;
    if (radial_index == SENTINEL_NO_RADIAL) {
        return NO_DATA_COLOR;
    }

    let radial = radial_meta[radial_index];

    // Mirrors `radar_geo::lookup::find_gate_index`'s half-open-interval
    // convention: gate i covers [center_i - spacing/2, center_i + spacing/2).
    // Gates are spaced along the tilted beam: slant range = ground range /
    // cos(elevation), as `radar_geo::beam::slant_range_from_ground_range_km`.
    let slant_range_km = ground_range_km / cos(radians(radial.elevation_deg));
    let first_gate_start_km = radial.first_gate_range_km - radial.gate_spacing_km * 0.5;
    if (slant_range_km < first_gate_start_km || radial.gate_spacing_km <= 0.0) {
        return NO_DATA_COLOR;
    }
    let offset_km = slant_range_km - first_gate_start_km;
    let gate_index = u32(floor(offset_km / radial.gate_spacing_km));
    if (gate_index >= radial.gate_count) {
        return NO_DATA_COLOR;
    }

    let sample = gate_samples[radial.gate_offset + gate_index];
    if (sample.flag == GATE_FLAG_RANGE_FOLDED) {
        // The table's own range-folded color, which `ColorTable::validate`
        // keeps distinct from `missing_color` so it is never mistaken for
        // "no data" (premultiplied like the palette colors below).
        let rf = uniforms.range_folded_color;
        return vec4<f32>(rf.rgb * rf.a, rf.a);
    }
    if (sample.flag != GATE_FLAG_VALID) {
        // GATE_FLAG_MISSING (or any other non-valid flag): no data.
        return NO_DATA_COLOR;
    }

    let span = uniforms.palette_max_dbz - uniforms.palette_min_dbz;
    var u = 0.0;
    if (span > 0.0) {
        u = clamp((sample.value - uniforms.palette_min_dbz) / span, 0.0, 1.0);
    }
    let palette_color = textureSampleLevel(palette_tex, palette_sampler, u, 0.0);
    // Premultiply RGB by alpha: the browser-facing surface (see
    // `radar-web`'s `RadarWebRenderer::create`) is configured for
    // premultiplied-alpha compositing, which the WebGPU canvas API requires
    // for any alpha-respecting mode. A color table's stops are authored as
    // ordinary (straight-alpha) RGBA -- see `COLOR_TABLE_FORMAT.md` -- so
    // that conversion happens here, once, at the only point a partial
    // (neither-0-nor-255) alpha value can actually reach this shader.
    // `NO_DATA_COLOR` above is already premultiply-safe (alpha 0), so it
    // returns directly without this step.
    return vec4<f32>(palette_color.rgb * palette_color.a, palette_color.a);
}
