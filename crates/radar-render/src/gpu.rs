//! `wgpu`-dependent code: adapter/device acquisition, GPU resource upload,
//! the render pipeline for `shaders/radar_sweep.wgsl`, and off-screen
//! render-target readback.
//!
//! Everything CPU-testable (buffer layout, lookup-table, palette-LUT
//! construction) lives in [`crate::sweep_buffers`], [`crate::lookup_texture`],
//! [`crate::palette`], and [`crate::camera`] instead, specifically so it
//! can be unit-tested with no GPU present. This module only wires those
//! already-built `Vec`s into `wgpu` resources and issues the actual draw
//! call, so **compiling** it never needs a GPU (that's just linking
//! against the `wgpu` crate, same as any other dependency) — only
//! *running* [`GpuContext::request`] and the functions that use its
//! `Device`/`Queue` does.
//!
//! Every public entry point that talks to real hardware
//! ([`GpuContext::request`]) returns an `Option`/skip result instead of
//! panicking when no adapter is available, so callers (the harness binary
//! and this crate's own `#[test]`s) can run unconditionally in CI (no GPU)
//! and still exercise the full pipeline on a machine that has one.

use crate::camera::Mat4;
use crate::lookup_texture::SENTINEL_NO_RADIAL;
use crate::sweep_buffers::{GpuGateSample, GpuRadialMeta, SweepBufferData};
use wgpu::util::DeviceExt;

const SHADER_SOURCE: &str = include_str!("shaders/radar_sweep.wgsl");

/// A live `wgpu` device/queue pair plus a little diagnostic info about
/// which adapter was selected — everything the rest of this module needs
/// to build resources and render.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_info: wgpu::AdapterInfo,
}

impl GpuContext {
    /// Request a `wgpu` adapter and device, preferring high-performance
    /// power preference. Returns `None` (after printing a clear,
    /// human-readable reason to stderr) instead of panicking/unwrapping
    /// if no adapter is available or device creation fails — this is the
    /// one place in the crate that must never crash a GPU-less CI run.
    ///
    /// Note on `wgpu` 30's API: `Instance::request_adapter` and
    /// `Adapter::request_device` return `Result`, not the `Option` older
    /// `wgpu` versions used — both a genuine "no adapter" condition and
    /// any other adapter/device-request failure surface as an `Err` here,
    /// and both are treated identically as "GPU unavailable, skip".
    pub async fn request() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());

        let adapter = match instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
        {
            Ok(adapter) => adapter,
            Err(e) => {
                eprintln!("radar-render: no GPU adapter available ({e}); skipping GPU path.");
                return None;
            }
        };

        let adapter_info = adapter.get_info();

        let (device, queue) = match adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("radar-render device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
        {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!(
                    "radar-render: adapter '{}' could not create a device ({e}); skipping GPU path.",
                    adapter_info.name
                );
                return None;
            }
        };

        Some(Self {
            device,
            queue,
            adapter_info,
        })
    }
}

/// GPU-resident form of one sweep's [`SweepBufferData`] plus its
/// radial-index lookup texture: the storage-buffer + lookup-texture
/// hybrid representation this crate exists to prove out.
pub struct SweepGpuResources {
    pub radial_meta_buffer: wgpu::Buffer,
    pub gate_sample_buffer: wgpu::Buffer,
    pub radial_lookup_view: wgpu::TextureView,
    /// Byte size of every GPU allocation this struct owns, for the
    /// harness's honest (sum-of-allocations, not driver-queried) VRAM
    /// estimate.
    pub byte_size: u64,
}

/// Upload `data`'s radial metadata and gate samples, and `lookup_table`
/// (from [`crate::lookup_texture::build_radial_lookup`]), as GPU
/// resources: two storage buffers and one `R32Uint` 1D texture.
pub fn upload_sweep(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    data: &SweepBufferData,
    lookup_table: &[u32],
) -> SweepGpuResources {
    // `create_buffer_init` refuses a zero-length `contents` slice on some
    // backends, and an empty sweep is a legitimate (if useless) input —
    // pad to at least one element so buffer creation never fails on
    // degenerate input; the shader will simply never index past
    // `gate_count`/the lookup table's sentinel for an empty sweep.
    let radial_meta: &[GpuRadialMeta] = if data.radial_meta.is_empty() {
        &[GpuRadialMeta {
            azimuth_deg: 0.0,
            elevation_angle_deg: 0.0,
            first_gate_range_km: 0.0,
            gate_spacing_km: 0.0,
            gate_count: 0,
            gate_offset: 0,
        }]
    } else {
        &data.radial_meta
    };
    let gate_samples: &[GpuGateSample] = if data.gate_samples.is_empty() {
        &[GpuGateSample {
            value: 0.0,
            flag: crate::sweep_buffers::GATE_FLAG_MISSING,
        }]
    } else {
        &data.gate_samples
    };

    let radial_meta_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("radar-render radial metadata storage buffer"),
        contents: bytemuck::cast_slice(radial_meta),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let gate_sample_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("radar-render gate sample storage buffer"),
        contents: bytemuck::cast_slice(gate_samples),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let lookup_texel_count = lookup_table.len().max(1) as u32;
    let padded_lookup: Vec<u32>;
    let lookup_data: &[u32] = if lookup_table.is_empty() {
        padded_lookup = vec![SENTINEL_NO_RADIAL];
        &padded_lookup
    } else {
        lookup_table
    };

    let radial_lookup_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("radar-render radial-index lookup texture"),
        size: wgpu::Extent3d {
            width: lookup_texel_count,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D1,
        format: wgpu::TextureFormat::R32Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &radial_lookup_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(lookup_data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(lookup_texel_count * 4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: lookup_texel_count,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let radial_lookup_view =
        radial_lookup_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let byte_size = std::mem::size_of_val(radial_meta) as u64
        + std::mem::size_of_val(gate_samples) as u64
        + u64::from(lookup_texel_count) * 4;

    SweepGpuResources {
        radial_meta_buffer,
        gate_sample_buffer,
        radial_lookup_view,
        byte_size,
    }
}

/// GPU-resident palette texture + sampler.
pub struct PaletteGpuResources {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub byte_size: u64,
}

/// Upload a palette LUT (from [`crate::palette::build_palette_lut`]) as an
/// `Rgba8Unorm` 1D texture, sampled with linear filtering (a smooth
/// on-screen gradient between stops — never applied to gate *values*,
/// only to this already-resolved color ramp) and clamp-to-edge addressing.
pub fn upload_palette(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    lut: &[[u8; 4]],
) -> PaletteGpuResources {
    let texel_count = lut.len().max(1) as u32;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("radar-render palette texture"),
        size: wgpu::Extent3d {
            width: texel_count,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D1,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_palette_texture(queue, &texture, lut, texel_count);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("radar-render palette sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    PaletteGpuResources {
        texture,
        view,
        sampler,
        byte_size: u64::from(texel_count) * 4,
    }
}

/// Overwrite an already-uploaded palette texture's contents in place
/// (rather than recreating the texture/bind group) — this is the
/// operation the harness times as "palette switch", since it is what a
/// real palette-switch UI action would actually do once GPU resources
/// already exist.
pub fn update_palette(queue: &wgpu::Queue, palette: &PaletteGpuResources, lut: &[[u8; 4]]) {
    let texel_count = lut.len().max(1) as u32;
    write_palette_texture(queue, &palette.texture, lut, texel_count);
}

fn write_palette_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    lut: &[[u8; 4]],
    texel_count: u32,
) {
    let fallback = [[0u8; 4]];
    let data: &[[u8; 4]] = if lut.is_empty() { &fallback } else { lut };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(texel_count * 4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: texel_count,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
}

/// GPU uniform buffer backing `Uniforms` in `shaders/radar_sweep.wgsl`.
/// Layout is documented on [`GpuUniforms`].
pub struct UniformsGpu {
    pub buffer: wgpu::Buffer,
}

/// Mirrors `shaders/radar_sweep.wgsl`'s `Uniforms` struct field-for-field
/// (see that file for the authoritative layout comments); `#[repr(C)]`
/// with only 4-byte-aligned fields after the matrix keeps this struct's
/// Rust layout identical to WGSL's, so no manual byte packing is needed.
///
/// `palette_min_dbz`/`palette_max_dbz` keep their S03-era, REF-specific
/// names for both fields (and the matching WGSL field names in
/// `shaders/radar_sweep.wgsl`) to avoid an invasive, purely-cosmetic
/// rename across this struct, that shader, and every caller
/// (`src/bin/harness.rs`, `gpu_tests.rs`, `radar-web`'s `browser.rs`).
/// Their actual contract, since S05, is a **generic palette-domain
/// min/max**: whatever [`crate::color_table::ColorTable::domain`] the
/// active moment's color table declares (dBZ for REF, but m/s for VEL/SW,
/// dB for ZDR, degrees for PHI, dimensionless for CC) -- see
/// [`crate::color_table::build_lut_from_table`], which this same domain
/// feeds into to build the LUT these two values are sampled against.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuUniforms {
    pub clip_to_world: Mat4,
    pub site_max_range_km: f32,
    pub lookup_texel_count: u32,
    pub palette_min_dbz: f32,
    pub palette_max_dbz: f32,
    pub _pad: [u32; 4],
}

impl UniformsGpu {
    pub fn new(device: &wgpu::Device, initial: GpuUniforms) -> Self {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("radar-render uniforms"),
            contents: bytemuck::bytes_of(&initial),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self { buffer }
    }

    /// Rewrite the uniform buffer's contents in place — used every frame
    /// to upload a new `clip_to_world` (simulated pan/zoom).
    pub fn update(&self, queue: &wgpu::Queue, value: GpuUniforms) {
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&value));
    }
}

/// The render pipeline for `shaders/radar_sweep.wgsl`, plus the bind group
/// layout it expects (uniforms, radial metadata, gate samples, lookup
/// texture, palette texture, palette sampler — bindings 0-5 in that
/// order, matching the shader source).
pub struct SweepPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
}

pub fn create_pipeline(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> SweepPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("radar-render sweep shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("radar-render sweep bind group layout"),
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
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D1,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D1,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("radar-render sweep pipeline layout"),
        bind_group_layouts: &[Some(&bind_group_layout)],
        immediate_size: 0,
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("radar-render sweep pipeline"),
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

    SweepPipeline {
        pipeline,
        bind_group_layout,
    }
}

pub fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &UniformsGpu,
    sweep: &SweepGpuResources,
    palette: &PaletteGpuResources,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("radar-render sweep bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: sweep.radial_meta_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: sweep.gate_sample_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&sweep.radial_lookup_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&palette.view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&palette.sampler),
            },
        ],
    })
}

/// An off-screen color render target (no window/surface — this proves the
/// pipeline works, it does not need to be interactively displayed).
pub struct RenderTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

pub const RENDER_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

pub fn create_render_target(device: &wgpu::Device, width: u32, height: u32) -> RenderTarget {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("radar-render off-screen target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: RENDER_TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    RenderTarget {
        texture,
        view,
        width,
        height,
    }
}

/// Encode and submit one full-screen draw call against `target`, using
/// `pipeline`/`bind_group`. Clears to fully transparent black first, so
/// any pixel the fragment shader itself resolves as "no data" and any
/// pixel outside the drawn geometry both read back as the same
/// unambiguous transparent value.
pub fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::RenderPipeline,
    bind_group: &wgpu::BindGroup,
    target: &RenderTarget,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("radar-render frame encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("radar-render sweep pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    queue.submit(std::iter::once(encoder.finish()));
}

/// Block until every previously submitted command on `device` has
/// completed. `render_frame` itself only *submits* work; call this (or
/// [`read_rgba8`], which does it internally) when frame timing must
/// reflect actual GPU completion rather than just CPU-side submission.
pub fn wait_for_gpu(device: &wgpu::Device) {
    // `wgpu` 30's `Device::poll` takes a `PollType`; `Wait` blocks the
    // calling thread until the given (here: all) submissions complete.
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
}

/// Read `target`'s pixels back to the CPU as tightly-packed RGBA8 rows
/// (no `wgpu` copy-alignment padding left in the result), blocking until
/// the copy and buffer mapping complete.
pub fn read_rgba8(device: &wgpu::Device, queue: &wgpu::Queue, target: &RenderTarget) -> Vec<u8> {
    let bytes_per_pixel = 4u32;
    let unpadded_bytes_per_row = target.width * bytes_per_pixel;
    // `wgpu` requires `COPY_BYTES_PER_ROW_ALIGNMENT` (256)-aligned rows
    // for a texture-to-buffer copy.
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;

    let buffer_size = u64::from(padded_bytes_per_row) * u64::from(target.height);
    let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("radar-render readback buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("radar-render readback encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(target.height),
            },
        },
        wgpu::Extent3d {
            width: target.width,
            height: target.height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = readback_buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver
        .recv()
        .expect("map_async callback should always fire after Wait poll")
        .expect("readback buffer mapping should succeed for a COPY_DST|MAP_READ buffer");

    let mapped = slice
        .get_mapped_range()
        .expect("buffer was just successfully mapped above");
    let mut out = Vec::with_capacity((unpadded_bytes_per_row * target.height) as usize);
    for row in 0..target.height {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + unpadded_bytes_per_row as usize;
        out.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    readback_buffer.unmap();
    out
}
