//! `wgpu`-dependent code for this crate's grid renderer: GPU resource
//! upload, the render pipeline for `shaders/gefs_grid.wgsl`, and building
//! this crate's own bind group.
//!
//! Everything genuinely generic is reused directly from `radar-render`'s
//! public API rather than duplicated: [`radar_render::gpu::GpuContext`]
//! (adapter/device acquisition), [`radar_render::gpu::upload_palette`]/
//! [`radar_render::gpu::PaletteGpuResources`]/[`radar_render::gpu::update_palette`]
//! (the palette LUT texture -- nothing about it is polar-radar-specific),
//! and [`radar_render::gpu::create_render_target`]/
//! [`radar_render::gpu::render_frame`]/[`radar_render::gpu::wait_for_gpu`]/
//! [`radar_render::gpu::read_rgba8`]/[`radar_render::gpu::RENDER_TARGET_FORMAT`]
//! (off-screen target + draw + readback -- generic over any pipeline/bind
//! group). Only the grid *value* texture, this crate's own `Uniforms`
//! layout, and the pipeline/bind-group-layout tied to
//! `shaders/gefs_grid.wgsl`'s specific bindings are new here -- per
//! ARCHITECTURE.md, a regular lat/lon grid is not forced through
//! `radar-render`'s polar-radial lookup-texture machinery.

use radar_render::camera::Mat4;
use wgpu::util::DeviceExt;

const SHADER_SOURCE: &str = include_str!("shaders/gefs_grid.wgsl");

/// GPU-resident form of a decoded grid's values: a single-channel 32-bit
/// float 2D texture, `width` x `height`, row-major (matching
/// [`crate::field::GriddedField::values`]'s own layout exactly -- no
/// reshaping beyond the flat-`Vec` -> 2D-texture reinterpretation).
pub struct GridGpuResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub byte_size: u64,
}

/// Upload a display-unit value array (e.g. [`crate::render::to_display_celsius`]'s
/// output) as an `R32Float` 2D texture. `values.len()` must equal
/// `width * height` (the same invariant [`crate::decode::decode_field`]
/// already guarantees for a [`crate::field::GriddedField`]).
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
        label: Some("provider-gefs grid value texture"),
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

/// Mirrors `shaders/gefs_grid.wgsl`'s `Uniforms` struct field-for-field.
/// `#[repr(C)]` with only 4-byte-aligned fields after the matrix keeps
/// this struct's Rust layout identical to WGSL's; the total size (96
/// bytes) is already a multiple of 16, so no explicit tail padding is
/// needed (verified against `wgpu`'s validation: a uniform buffer struct
/// whose size is not a multiple of its largest member's alignment is
/// rejected at pipeline-creation time, which would fail this crate's own
/// tests/harness immediately rather than silently misrendering).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuUniforms {
    pub clip_to_world: Mat4,
    pub origin_lon_deg: f32,
    pub origin_lat_deg: f32,
    pub lon_step_deg: f32,
    pub lat_step_deg: f32,
    pub grid_width: u32,
    pub grid_height: u32,
    pub palette_min: f32,
    pub palette_max: f32,
}

pub struct UniformsGpu {
    pub buffer: wgpu::Buffer,
}

impl UniformsGpu {
    pub fn new(device: &wgpu::Device, initial: GpuUniforms) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("provider-gefs uniforms"),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self { buffer }
    }

    pub fn update(&self, queue: &wgpu::Queue, value: GpuUniforms) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&value));
    }
}

/// The render pipeline for `shaders/gefs_grid.wgsl`, plus the bind group
/// layout it expects: uniforms, grid value texture, palette texture,
/// palette sampler -- bindings 0-3 in that order.
pub struct GridPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

pub fn create_pipeline(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> GridPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("provider-gefs grid shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("provider-gefs grid bind group layout"),
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
                // (nearest, integer-indexed -- no interpolation between
                // grid cells), so `filterable: false` is both correct and
                // requires no extra `wgpu::Features`.
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
        label: Some("provider-gefs grid pipeline layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("provider-gefs grid pipeline"),
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
        label: Some("provider-gefs grid bind group"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_uniforms_size_is_a_multiple_of_16() {
        // WGSL uniform-buffer struct layout requires the total size be a
        // multiple of the largest member's alignment (16, from
        // `mat4x4<f32>`) -- checked here so a future field addition that
        // breaks this is caught by `cargo test` (no GPU needed) rather
        // than only at pipeline-creation time on a machine with a GPU.
        assert_eq!(std::mem::size_of::<GpuUniforms>() % 16, 0);
    }
}
