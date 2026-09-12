// S03 GPU radar renderer prototype shader.
//
// Per pixel: clip-space position -> site-relative world (x, y) in km via a
// CPU-supplied orthographic projection (its inverse, `clip_to_world`, so
// the shader can go directly from a pixel to world space) -> azimuth +
// ground-range from that flat local (x, y) plane -> radial index via the
// lookup texture -> gate index via the same half-open-interval convention
// as `radar_geo::lookup::find_gate_index` -> raw/converted gate value ->
// distinct output for missing/range-folded, otherwise the palette ramp.
//
// Scope boundary (documented per the S03 task): this shader does *not* do
// real lat/lon/great-circle geodesy. It treats the already-decoded,
// site-relative Cartesian plane as flat, and treats straight-line distance
// in that plane as directly comparable to a moment's slant-range gate
// spacing. Resolving an actual geographic cursor to azimuth/range (real
// geodesy, `radar_geo`'s job) is a separate concern from proving this
// render pipeline; that resolution already happened on the CPU when this
// crate's `render.rs` built `clip_to_world` from a site-relative view.
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
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
};

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

// Reserved diagnostic color for a range-folded gate: full-alpha magenta,
// a color the REF palette (green -> yellow -> red) never produces, so a
// viewer can never mistake a range-folded return for a real reading.
const RANGE_FOLDED_COLOR: vec4<f32> = vec4<f32>(1.0, 0.0, 1.0, 1.0);
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
    let x = world.x;
    let y = world.y;

    let ground_range_km = length(vec2<f32>(x, y));
    if (ground_range_km > uniforms.site_max_range_km) {
        return NO_DATA_COLOR;
    }

    // Bearing clockwise from north: north is +y, east is +x, so
    // atan2(x, y) (not the usual atan2(y, x)) gives compass bearing
    // directly, matching `radar_types::Radial::azimuth_angle_deg`'s
    // convention.
    var azimuth_deg = degrees(atan2(x, y));
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
    let first_gate_start_km = radial.first_gate_range_km - radial.gate_spacing_km * 0.5;
    if (ground_range_km < first_gate_start_km || radial.gate_spacing_km <= 0.0) {
        return NO_DATA_COLOR;
    }
    let offset_km = ground_range_km - first_gate_start_km;
    let gate_index = u32(floor(offset_km / radial.gate_spacing_km));
    if (gate_index >= radial.gate_count) {
        return NO_DATA_COLOR;
    }

    let sample = gate_samples[radial.gate_offset + gate_index];
    if (sample.flag == GATE_FLAG_RANGE_FOLDED) {
        return RANGE_FOLDED_COLOR;
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
    // `NO_DATA_COLOR`/`RANGE_FOLDED_COLOR` above are already
    // premultiply-safe (alpha 0 or 1), so they return directly without
    // this step.
    return vec4<f32>(palette_color.rgb * palette_color.a, palette_color.a);
}
