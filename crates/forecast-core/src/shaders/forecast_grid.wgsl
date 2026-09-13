// forecast-core (S08) provider-agnostic gridded-forecast-field renderer
// shader -- generalized from S07's `provider-gefs/src/shaders/gefs_grid.wgsl`.
//
// Per pixel: clip-space position -> world (longitude, latitude) in decimal
// degrees via a CPU-supplied orthographic projection (its inverse,
// `clip_to_world`, mirroring `radar-render`'s `radar_sweep.wgsl`) -> nearest
// grid (row, col) -> raw value from the grid value texture -> palette ramp.
//
// # One shader, two grid projections
//
// This is the ONE shared grid-render call path both `provider-gefs` (a
// regular latitude/longitude grid, GRIB2 Grid Definition Template 3.0) and
// `provider-hrrr` (a Lambert Conformal Conic grid, Template 3.30 --
// confirmed empirically, see `docs/adr/0012-hrrr-lambert-conformal-grid.md`)
// render through -- `uniforms.projection_kind` selects which world-position
// -> grid-cell formula to use, entirely data-driven from the uniform
// buffer. This is a branch on *projection kind* (a property of the grid's
// own geometry, reusable by any future provider using either projection),
// not a branch on *provider identity* -- exactly the S08 stage file's own
// rule: "if HRRR requires provider-specific conditionals throughout the
// UI, improve the abstraction rather than special-casing broadly." Both
// call paths converge on the same texture lookup and palette sampling
// below.
//
// This is a **regular flat-plane** renderer (not a real map projection for
// display, same scope boundary as `radar_sweep.wgsl`/`gefs_grid.wgsl`):
// resolving world position onto an actual web map is a future map-adapter
// concern (ARCHITECTURE.md: "Mapping is behind an adapter").

struct Uniforms {
    // Maps clip-space (NDC) (x, y, 0, 1) to world (longitude, latitude)
    // degrees (z/w unused).
    clip_to_world: mat4x4<f32>,
    // 0 = regular latitude/longitude, 1 = Lambert Conformal Conic.
    projection_kind: u32,
    grid_width: u32,
    grid_height: u32,
    _pad0: u32,
    // --- projection_kind == 0 (regular lat/lon) ---
    origin_lon_deg: f32,
    origin_lat_deg: f32,
    lon_step_deg: f32,
    lat_step_deg: f32,
    // --- projection_kind == 1 (Lambert Conformal Conic) ---
    // Central meridian, radians.
    lcc_lam0_rad: f32,
    // Cone constant.
    lcc_n: f32,
    // Earth radius * projection scale factor F.
    lcc_f_times_r: f32,
    // Polar distance to the projection's own latitude-of-origin parallel.
    lcc_rho0: f32,
    // Projected (x, y) of grid cell (row=0, col=0), meters.
    lcc_origin_x_m: f32,
    lcc_origin_y_m: f32,
    // Signed meters-per-column/-row step.
    lcc_dx_m: f32,
    lcc_dy_m: f32,
    // Palette domain: `palette_tex` texel 0 corresponds to this physical
    // value, and its last texel corresponds to `palette_max`.
    palette_min: f32,
    palette_max: f32,
    _pad1: vec2<f32>,
};

const NO_DATA_COLOR: vec4<f32> = vec4<f32>(0.0, 0.0, 0.0, 0.0);
const PI: f32 = 3.14159265358979;

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var grid_tex: texture_2d<f32>;
@group(0) @binding(2) var palette_tex: texture_1d<f32>;
@group(0) @binding(3) var palette_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

// Full-screen triangle from just `vertex_index` -- same trick
// `radar_sweep.wgsl`/`gefs_grid.wgsl` use.
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

// Regular latitude/longitude grid: linear (lon, lat) -> (col, row).
fn nearest_cell_regular(lon: f32, lat: f32) -> vec2<i32> {
    let col_f = (lon - uniforms.origin_lon_deg) / uniforms.lon_step_deg;
    let row_f = (lat - uniforms.origin_lat_deg) / uniforms.lat_step_deg;
    return vec2<i32>(i32(round(col_f)), i32(round(row_f)));
}

// Lambert Conformal Conic grid: forward-project (lon, lat) to the
// projection's (x, y) meters, then linearly resolve to (col, row) --
// mirrors `forecast_core::projection::LccProjection::project` and
// `LambertConformalGrid::nearest_cell` exactly (see those Rust
// implementations' doc comments for the projection math and its
// real-HRRR-data validation).
fn nearest_cell_lambert(lon: f32, lat: f32) -> vec2<i32> {
    let phi = radians(lat);
    let lam = radians(lon);
    var dlam = lam - uniforms.lcc_lam0_rad;
    // Normalize to (-pi, pi] so a longitude given in either the [0, 360)
    // or (-180, 180] convention projects identically -- see
    // `LccProjection::normalize_delta`'s doc comment for why this matters.
    dlam = dlam - 2.0 * PI * floor(dlam / (2.0 * PI) + 0.5);
    let theta = uniforms.lcc_n * dlam;
    let rho = uniforms.lcc_f_times_r / pow(tan(PI / 4.0 + phi / 2.0), uniforms.lcc_n);
    let x = rho * sin(theta);
    let y = uniforms.lcc_rho0 - rho * cos(theta);
    let col_f = (x - uniforms.lcc_origin_x_m) / uniforms.lcc_dx_m;
    let row_f = (y - uniforms.lcc_origin_y_m) / uniforms.lcc_dy_m;
    return vec2<i32>(i32(round(col_f)), i32(round(row_f)));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let world = uniforms.clip_to_world * vec4<f32>(in.ndc, 0.0, 1.0);
    let lon = world.x;
    let lat = world.y;

    var cell: vec2<i32>;
    if (uniforms.projection_kind == 0u) {
        cell = nearest_cell_regular(lon, lat);
    } else {
        cell = nearest_cell_lambert(lon, lat);
    }
    let col = cell.x;
    let row = cell.y;
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
    // convention.
    return vec4<f32>(palette_color.rgb * palette_color.a, palette_color.a);
}
