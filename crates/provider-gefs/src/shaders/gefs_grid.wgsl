// provider-gefs (S07) grid renderer shader.
//
// Per pixel: clip-space position -> world (longitude, latitude) in decimal
// degrees via a CPU-supplied orthographic projection (its inverse,
// `clip_to_world`, mirroring `radar-render`'s `radar_sweep.wgsl` --
// "the shader only ever needs the inverse, pixel -> world") -> nearest
// grid (row, col) via the field's own regular-grid origin/step (no
// interpolation between grid cells, matching this project's existing
// nearest-only convention for radar -- GLOBAL_CONTRACT: do not fabricate
// structure between real samples) -> raw value from the grid value
// texture -> palette ramp.
//
// This is a **regular lat/lon grid**, not polar radar geometry: it gets
// its own simple vertex/UV mapping rather than being forced through
// `radar-render`'s radial-lookup-texture machinery (ARCHITECTURE.md
// explicitly permits this: "Do not force polar radar and gridded model
// data into the same geometry representation").
//
// Scope boundary (same as `radar_sweep.wgsl`): this shader treats
// (longitude, latitude) as a flat plane, not a real map projection --
// resolving that onto an actual web map (Mercator, etc.) is a future map-
// adapter concern (ARCHITECTURE.md: "Mapping is behind an adapter"), not
// this S07 proof-of-concept's job. The harness's own camera (see
// `src/bin/harness.rs`) frames the field's own lat/lon bounding box
// directly, which is sufficient to prove the render is geographically
// correct for this PoC.

struct Uniforms {
    // Maps clip-space (NDC) (x, y, 0, 1) to world (longitude, latitude)
    // degrees (z/w unused).
    clip_to_world: mat4x4<f32>,
    origin_lon_deg: f32,
    origin_lat_deg: f32,
    lon_step_deg: f32,
    lat_step_deg: f32,
    grid_width: u32,
    grid_height: u32,
    // Palette domain: `palette_tex` texel 0 corresponds to this physical
    // value (Celsius, for this crate's temperature field), and its last
    // texel corresponds to `palette_max`.
    palette_min: f32,
    palette_max: f32,
};

const NO_DATA_COLOR: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 0.0);

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var grid_tex: texture_2d<f32>;
@group(0) @binding(2) var palette_tex: texture_1d<f32>;
@group(0) @binding(3) var palette_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// Full-screen triangle from just `vertex_index` -- same trick
// `radar_sweep.wgsl` uses, so every pixel in the viewport is rasterized by
// one triangle.
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
    let lon = world.x;
    let lat = world.y;

    let col_f = (lon - uniforms.origin_lon_deg) / uniforms.lon_step_deg;
    let row_f = (lat - uniforms.origin_lat_deg) / uniforms.lat_step_deg;
    let col = i32(round(col_f));
    let row = i32(round(row_f));
    if (col < 0 || col >= i32(uniforms.grid_width) || row < 0 || row >= i32(uniforms.grid_height)) {
        return NO_DATA_COLOR;
    }

    let value = textureLoad(grid_tex, vec2<i32>(col, row), 0).r;

    let span = uniforms.palette_max - uniforms.palette_min;
    var u = 0.0;
    if (span > 0.0) {
        u = clamp((value - uniforms.palette_min) / span, 0.0, 1.0);
    }
    let palette_color = textureSampleLevel(palette_tex, palette_sampler, u, 0.0);
    // Premultiply RGB by alpha, matching `radar_sweep.wgsl`'s own
    // convention (see that shader's header comment for the full
    // reasoning): a color table's stops are authored as ordinary
    // (straight-alpha) RGBA, and this is the one point that needs
    // converting for premultiplied-alpha compositing.
    return vec4<f32>(palette_color.rgb * palette_color.a, palette_color.a);
}
