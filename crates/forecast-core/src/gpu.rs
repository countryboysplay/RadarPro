//! `wgpu`-dependent code for this crate's grid renderer: GPU resource
//! upload, the render pipeline for `shaders/forecast_grid.wgsl`, and
//! building this crate's own bind group. Moved here from S07's
//! `provider-gefs::gpu` and generalized: this module knows nothing about
//! any specific provider, only about [`crate::grid::GridGeometry`]'s two
//! projection kinds (see `shaders/forecast_grid.wgsl`'s module doc for why
//! that is a projection-kind branch, not a provider-identity branch).
//!
//! Everything genuinely generic is reused directly from `radar-render`'s
//! public API rather than duplicated: [`radar_render::gpu::GpuContext`]
//! (adapter/device acquisition), [`radar_render::gpu::upload_palette`]/
//! [`radar_render::gpu::PaletteGpuResources`]/[`radar_render::gpu::update_palette`]
//! (the palette LUT texture), and [`radar_render::gpu::create_render_target`]/
//! [`radar_render::gpu::render_frame`]/[`radar_render::gpu::read_rgba8_async`]/
//! [`radar_render::gpu::RENDER_TARGET_FORMAT`] (off-screen target + draw +
//! readback -- generic over any pipeline/bind group). Only the grid *value*
//! texture, this crate's own `Uniforms` layout, and the pipeline/bind-group-layout
//! tied to `shaders/forecast_grid.wgsl`'s specific bindings are defined here.
//!
//! [`render_forecast_grid`] specifically uses
//! [`radar_render::gpu::read_rgba8_async`], not the older
//! [`radar_render::gpu::wait_for_gpu`]/[`radar_render::gpu::read_rgba8`]
//! pair: this crate's whole reason for existing is to be called from
//! `forecast-web`'s wasm-bindgen glue (see `crates/forecast-web/src/wasm_api.rs`),
//! and the blocking `read_rgba8` deadlocks the browser tab on wasm32 --
//! `Device::poll` is a documented no-op on the WebGPU backend (see
//! `radar_web::browser`'s doc comment on this exact point), so nothing
//! would ever wake a thread blocked on `std::sync::mpsc::Receiver::recv`.
//! `read_rgba8_async` fixes this by resolving through a real `.await`
//! instead, which is what actually yields control back to the browser's
//! event loop (via `wasm_bindgen_futures::future_to_promise` on the
//! `forecast-web` side) rather than blocking it.

use crate::grid::GridGeometry;
use radar_render::camera::Mat4;
use wgpu::util::DeviceExt;

const SHADER_SOURCE: &str = include_str!("shaders/forecast_grid.wgsl");

const PROJECTION_KIND_REGULAR_LAT_LON: u32 = 0;
const PROJECTION_KIND_LAMBERT_CONFORMAL: u32 = 1;

/// GPU-resident form of a decoded grid's values: a single-channel 32-bit
/// float 2D texture, `width` x `height`, row-major (matching
/// [`crate::grid::ForecastGrid::values`]'s own layout exactly).
pub struct GridGpuResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub byte_size: u64,
}

/// Upload a display-unit value array (e.g. [`crate::render::to_display_celsius`]'s
/// output) as an `R32Float` 2D texture. `values.len()` must equal
/// `width * height`.
pub fn upload_grid(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    values: &[f32],
) -> GridGpuResources {
    assert_eq!(
        values.len(),
        width as usize * height as usize,
        "upload_grid: value count must equal width * height"
    );

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("forecast-core grid value texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    GridGpuResources {
        texture,
        view,
        byte_size: u64::from(width) * u64::from(height) * 4,
    }
}

/// Mirrors `shaders/forecast_grid.wgsl`'s `Uniforms` struct field-for-field.
/// `#[repr(C)]` with only 4-byte-aligned fields after the matrix keeps this
/// struct's Rust layout identical to WGSL's; the total size (144 bytes) is
/// already a multiple of 16.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuUniforms {
    pub clip_to_world: Mat4,
    pub projection_kind: u32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub _pad0: u32,
    pub origin_lon_deg: f32,
    pub origin_lat_deg: f32,
    pub lon_step_deg: f32,
    pub lat_step_deg: f32,
    pub lcc_lam0_rad: f32,
    pub lcc_n: f32,
    pub lcc_f_times_r: f32,
    pub lcc_rho0: f32,
    pub lcc_origin_x_m: f32,
    pub lcc_origin_y_m: f32,
    pub lcc_dx_m: f32,
    pub lcc_dy_m: f32,
    pub palette_min: f32,
    pub palette_max: f32,
    pub _pad1: [f32; 2],
}

impl GpuUniforms {
    /// Build the uniform payload for `geometry`, filling in only the
    /// fields that matter for its actual projection kind (the other
    /// branch's fields are left zeroed) -- this is the one place that
    /// translates a provider-agnostic [`GridGeometry`] into the shader's
    /// flat, data-driven uniform layout, used identically regardless of
    /// which provider produced the grid.
    pub fn for_geometry(
        clip_to_world: Mat4,
        geometry: &GridGeometry,
        palette_min: f32,
        palette_max: f32,
    ) -> Self {
        let mut uniforms = GpuUniforms {
            clip_to_world,
            projection_kind: PROJECTION_KIND_REGULAR_LAT_LON,
            grid_width: geometry.width(),
            grid_height: geometry.height(),
            _pad0: 0,
            origin_lon_deg: 0.0,
            origin_lat_deg: 0.0,
            lon_step_deg: 0.0,
            lat_step_deg: 0.0,
            lcc_lam0_rad: 0.0,
            lcc_n: 0.0,
            lcc_f_times_r: 0.0,
            lcc_rho0: 0.0,
            lcc_origin_x_m: 0.0,
            lcc_origin_y_m: 0.0,
            lcc_dx_m: 0.0,
            lcc_dy_m: 0.0,
            palette_min,
            palette_max,
            _pad1: [0.0, 0.0],
        };
        match geometry {
            GridGeometry::RegularLatLon(g) => {
                uniforms.projection_kind = PROJECTION_KIND_REGULAR_LAT_LON;
                uniforms.origin_lon_deg = g.origin_lon_deg as f32;
                uniforms.origin_lat_deg = g.origin_lat_deg as f32;
                uniforms.lon_step_deg = g.lon_step_deg as f32;
                uniforms.lat_step_deg = g.lat_step_deg as f32;
            }
            GridGeometry::LambertConformal(g) => {
                uniforms.projection_kind = PROJECTION_KIND_LAMBERT_CONFORMAL;
                uniforms.lcc_lam0_rad = g.projection.lam0_rad() as f32;
                uniforms.lcc_n = g.projection.n() as f32;
                uniforms.lcc_f_times_r = g.projection.f_times_r() as f32;
                uniforms.lcc_rho0 = g.projection.rho0() as f32;
                uniforms.lcc_origin_x_m = g.origin_x_m as f32;
                uniforms.lcc_origin_y_m = g.origin_y_m as f32;
                uniforms.lcc_dx_m = g.dx_m as f32;
                uniforms.lcc_dy_m = g.dy_m as f32;
            }
        }
        uniforms
    }
}

pub struct UniformsGpu {
    pub buffer: wgpu::Buffer,
}

impl UniformsGpu {
    pub fn new(device: &wgpu::Device, initial: GpuUniforms) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("forecast-core uniforms"),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self { buffer }
    }

    pub fn update(&self, queue: &wgpu::Queue, value: GpuUniforms) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&value));
    }
}

/// The render pipeline for `shaders/forecast_grid.wgsl`, plus the bind
/// group layout it expects: uniforms, grid value texture, palette texture,
/// palette sampler -- bindings 0-3 in that order.
pub struct GridPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

pub fn create_pipeline(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> GridPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("forecast-core grid shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("forecast-core grid bind group layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                // R32Float is not filterable on every backend without an
                // extra feature; this shader only ever uses `textureLoad`
                // (nearest, integer-indexed).
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D1,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("forecast-core grid pipeline layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("forecast-core grid pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    GridPipeline {
        pipeline,
        bind_group_layout,
    }
}

pub fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &UniformsGpu,
    grid: &GridGpuResources,
    palette: &radar_render::gpu::PaletteGpuResources,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("forecast-core grid bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&grid.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&palette.view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&palette.sampler),
            },
        ],
    })
}

/// The single, provider-agnostic render call path: upload `display_values`/
/// `palette_lut`, build the pipeline and bind group from `geometry` alone
/// (no branch on any provider/variable identity anywhere in this function),
/// draw one full-screen frame, and read it back as RGBA8. Returns `None` if
/// no GPU adapter is available.
///
/// Takes a bare [`GridGeometry`] rather than a whole `ForecastGrid` --
/// this function never touched any of `ForecastGrid`'s forecast-specific
/// fields (`run_time`/`forecast_lead_hours`/`ensemble`/etc.), only its
/// `geometry`, so requiring a full `ForecastGrid` here forced every
/// caller (including a future non-forecast, observation-only caller such
/// as `mrms`, which has no run/lead-time/ensemble concept at all -- see
/// `docs/adr/0014-mrms-grib2-png-unpack-and-local-discipline.md`) to either
/// have a real `ForecastGrid` on hand or construct a misleading fake one
/// just to call this shared renderer. This is the literal shared call path
/// the S08 stage's exit criteria and this crate's own cross-provider test
/// (`tests/cross_provider_render.rs`, see also `provider-hrrr`'s and
/// `provider-gefs`'s harnesses) rely on: called once for a GEFS-decoded
/// grid, once for an HRRR-decoded one, and once for an MRMS-decoded one
/// (S09), with the exact same code -- only `geometry`'s own projection kind
/// is ever branched on, inside this function's own `GpuUniforms::for_geometry`.
#[allow(clippy::too_many_arguments)]
pub async fn render_forecast_grid(
    geometry: &GridGeometry,
    display_values: &[f32],
    palette_lut: &[[u8; 4]],
    palette_min: f32,
    palette_max: f32,
    clip_to_world: Mat4,
    render_width: u32,
    render_height: u32,
) -> Option<Vec<u8>> {
    let ctx = radar_render::gpu::GpuContext::request().await?;

    let grid_gpu = upload_grid(
        &ctx.device,
        &ctx.queue,
        geometry.width(),
        geometry.height(),
        display_values,
    );
    let palette_gpu = radar_render::gpu::upload_palette(&ctx.device, &ctx.queue, palette_lut);
    let pipeline = create_pipeline(&ctx.device, radar_render::gpu::RENDER_TARGET_FORMAT);
    let uniforms_gpu = UniformsGpu::new(
        &ctx.device,
        GpuUniforms::for_geometry(clip_to_world, geometry, palette_min, palette_max),
    );
    let bind_group = create_bind_group(
        &ctx.device,
        &pipeline.bind_group_layout,
        &uniforms_gpu,
        &grid_gpu,
        &palette_gpu,
    );
    let target = radar_render::gpu::create_render_target(&ctx.device, render_width, render_height);

    radar_render::gpu::render_frame(
        &ctx.device,
        &ctx.queue,
        &pipeline.pipeline,
        &bind_group,
        &target,
    );
    // `read_rgba8_async`, not `wait_for_gpu`+`read_rgba8`: the latter pair
    // blocks a thread on `std::sync::mpsc::Receiver::recv`, which deadlocks
    // the browser tab on wasm32 (see this module's doc comment). This
    // `.await` itself drives the wait for the GPU to finish (native: an
    // internal `Device::poll(wait_indefinitely)`; wasm32: the browser's own
    // polling, surfaced back to this future via `map_async`'s callback) --
    // no separate `wait_for_gpu` call is needed either way.
    Some(radar_render::gpu::read_rgba8_async(&ctx.device, &ctx.queue, &target).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_uniforms_size_is_a_multiple_of_16() {
        // WGSL uniform-buffer struct layout requires the total size be a
        // multiple of the largest member's alignment (16, from
        // `mat4x4<f32>`).
        assert_eq!(std::mem::size_of::<GpuUniforms>() % 16, 0);
    }
}
