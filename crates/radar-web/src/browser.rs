//! wasm32-only browser glue: turns a `<canvas>` element and a `Uint8Array`
//! of raw Archive II bytes into a rendered radar sweep, using
//! `radar_render::gpu`'s existing pipeline/bind-group/buffer/texture code
//! unmodified. The one genuinely new piece of `wgpu` code in this crate is
//! [`RadarWebRenderer::create`]'s surface acquisition/configuration and
//! [`RadarWebRenderer::render_frame`]'s presentation -- `radar-render`'s
//! native harness renders off-screen and reads pixels back for a PNG, so it
//! never needed either.
//!
//! # GPU init is `async` all the way to JS -- no `pollster`, no blocking
//!
//! `radar_render::gpu::GpuContext::request` turned out to need **no**
//! wasm32-specific change at all: it is already a plain `async fn` with no
//! `pollster::block_on` (or any other blocking call) inside it --
//! `pollster` is only used by `radar-render`'s native
//! `src/bin/harness.rs`/tests, as the *caller* of that `async fn`, to give a
//! synchronous `main()` something to block on. This crate never does that:
//! [`init_gpu`] wraps [`RadarWebRenderer::create`] (an `async fn`) in
//! [`wasm_bindgen_futures::future_to_promise`], so the entire GPU-init
//! chain -- instance/surface/adapter/device -- is `.await`ed from JS via the
//! returned `Promise`, exactly as the task's stated risk describes should be
//! done, without needing to touch `radar-render` itself.
//!
//! What *did* need genuinely new code is unrelated to blocking: `GpuContext::
//! request` builds an `Instance`/`Adapter` with no `Surface` at all (fine for
//! an off-screen render target), but a canvas-backed `Surface` must exist
//! *before* `request_adapter` is called, because `RequestAdapterOptions::
//! compatible_surface` is mandatory for the WebGL2 fallback backend (a WebGL2
//! adapter cannot be created without knowing what it must be compatible
//! with). That ordering dependency -- surface first, then a
//! surface-compatible adapter -- is the actual reason this module cannot
//! reuse `GpuContext::request` unmodified, not `pollster`/blocking.
//!
//! # WebGPU-with-WebGL2-fallback negotiation
//!
//! [`RadarWebRenderer::create`] uses `wgpu::util::new_instance_with_webgpu_
//! detection` (rather than the plain `wgpu::Instance::new` `GpuContext::
//! request` uses) specifically because it is `wgpu`'s own documented,
//! recommended way to target WebGPU with an automatic WebGL2 fallback: a
//! synchronous `Instance::new` cannot itself probe whether the browser's
//! `navigator.gpu` can actually produce a working adapter (Chrome on some
//! platforms has historically advertised the property without being able to
//! back it), so `wgpu` ships this `async` detection helper instead. This
//! crate's `Cargo.toml` enables the `webgl` `wgpu` feature (not on by
//! default) specifically so that fallback backend exists to negotiate down
//! to at all.
//!
//! # Canvas sizing
//!
//! The caller (the `www/` test page) is expected to pass a fixed CSS pixel
//! size with the backing buffer matching 1:1 (no `devicePixelRatio`
//! scaling) -- real DPI-aware resizing is out of scope for this proof; see
//! `www/index.html`.

use js_sys::Promise;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::HtmlCanvasElement;

use radar_render::camera::clip_to_world;
use radar_render::gpu::{
    self, GpuUniforms, PaletteGpuResources, RenderTarget, SweepGpuResources, SweepPipeline,
    UniformsGpu,
};
use radar_render::lookup_texture::{build_radial_lookup, DEFAULT_LOOKUP_TEXEL_COUNT};
use radar_render::palette::{
    build_palette_lut, default_ref_palette_stops, DEFAULT_REF_MAX_DBZ, DEFAULT_REF_MIN_DBZ,
    PALETTE_TEXEL_COUNT,
};
use radar_render::sweep_buffers::{build_sweep_buffers, GpuRadialMeta};
use radar_types::{AzimuthResolution, MomentKind, Volume};

use crate::sweep_select::pick_lowest_elevation_sweep_index;

/// Runs once when the wasm module is instantiated (before any exported
/// function can be called): installs a panic hook that turns a Rust panic
/// into a real, readable `console.error` message (with the panic location
/// and message) instead of the opaque, unhelpful "unreachable executed"
/// WebAssembly trap a panicking wasm32 binary otherwise surfaces to the
/// browser console. This is what makes it possible to debug this crate at
/// all from the browser console -- see the crate root docs and
/// `README.md`.
#[wasm_bindgen(start)]
fn on_wasm_module_init() {
    console_error_panic_hook::set_once();
}

/// Metadata about the sweep [`RadarWebRenderer::decode_sweep`] selected,
/// readable from JS via plain getters. Deliberately not the full
/// `radar_types::Sweep` -- that carries every radial's gate array, which
/// belongs on the GPU (via [`RadarWebRenderer::render_frame`]), not
/// marshalled through the JS boundary.
#[wasm_bindgen]
pub struct SweepInfo {
    site_icao: String,
    sweep_count: u32,
    elevation_deg: f32,
    radial_count: u32,
}

#[wasm_bindgen]
impl SweepInfo {
    #[wasm_bindgen(getter, js_name = siteIcao)]
    pub fn site_icao(&self) -> String {
        self.site_icao.clone()
    }

    #[wasm_bindgen(getter, js_name = sweepCount)]
    pub fn sweep_count(&self) -> u32 {
        self.sweep_count
    }

    #[wasm_bindgen(getter, js_name = elevationDeg)]
    pub fn elevation_deg(&self) -> f32 {
        self.elevation_deg
    }

    #[wasm_bindgen(getter, js_name = radialCount)]
    pub fn radial_count(&self) -> u32 {
        self.radial_count
    }
}

/// GPU resources for one uploaded sweep: everything
/// [`RadarWebRenderer::render_frame`] needs to draw it again without
/// re-uploading, built once per [`RadarWebRenderer::decode_sweep`] call.
struct UploadedSweep {
    // Held for their GPU-side storage buffers/texture/sampler, which
    // `bind_group` below borrows into a GPU-visible bind group at build
    // time; not read from directly again afterwards, but must outlive
    // `bind_group`.
    _sweep_gpu: SweepGpuResources,
    _palette_gpu: PaletteGpuResources,
    _uniforms_gpu: UniformsGpu,
    bind_group: wgpu::BindGroup,
}

/// A live GPU device/surface pair targeting one `<canvas>`, plus whatever
/// sweep has most recently been decoded/uploaded. Constructed via
/// [`init_gpu`] from JS; every other method is called on the handle it
/// resolves to.
#[wasm_bindgen]
pub struct RadarWebRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    pipeline: SweepPipeline,
    width: u32,
    height: u32,
    adapter_name: String,
    adapter_backend: wgpu::Backend,
    volume: Option<Volume>,
    selected_sweep_index: Option<usize>,
    uploaded: Option<UploadedSweep>,
}

#[wasm_bindgen(js_name = initGpu)]
/// Acquire a GPU device/surface for `canvas` and return a `Promise`
/// resolving to a [`RadarWebRenderer`] (or rejecting with a `string` error
/// message). `width`/`height` are both the canvas's backing-buffer pixel
/// size and its `SurfaceConfiguration` size -- the caller is expected to
/// have already set the `<canvas>` element's `width`/`height` attributes
/// (not just its CSS size) to these same values.
///
/// This is the one call in this crate's public API that must be `.await`ed
/// from JS rather than called synchronously -- see the module docs for why
/// GPU init is `async` all the way out, and why that is unrelated to the
/// `pollster`/blocking risk this task was written to surface.
pub fn init_gpu(canvas: HtmlCanvasElement, width: u32, height: u32) -> Promise {
    future_to_promise(async move {
        RadarWebRenderer::create(canvas, width, height)
            .await
            .map(JsValue::from)
            .map_err(|message| JsValue::from_str(&message))
    })
}

impl RadarWebRenderer {
    async fn create(canvas: HtmlCanvasElement, width: u32, height: u32) -> Result<Self, String> {
        // See module docs: this negotiates WebGPU-with-WebGL2-fallback,
        // `wgpu`'s own recommended replacement for `Instance::new` when
        // targeting the web.
        let instance = wgpu::util::new_instance_with_webgpu_detection(
            wgpu::InstanceDescriptor::new_without_display_handle(),
        )
        .await;

        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| format!("failed to create a wgpu surface from the <canvas>: {e}"))?;

        // `compatible_surface` is required (not merely advisory) for the
        // WebGL2 backend -- see `Surface`'s own docs -- so it is always
        // passed here, not only when WebGPU negotiation fails.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no GPU adapter available: {e}"))?;
        let adapter_info = adapter.get_info();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("radar-web device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .map_err(|e| {
                format!(
                    "adapter '{}' could not create a device: {e}",
                    adapter_info.name
                )
            })?;

        let surface_config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| {
                format!(
                    "surface is not supported by adapter '{}'",
                    adapter_info.name
                )
            })?;
        let surface_format = surface_config.format;
        surface.configure(&device, &surface_config);

        let pipeline = gpu::create_pipeline(&device, surface_format);

        Ok(Self {
            device,
            queue,
            surface,
            pipeline,
            width,
            height,
            adapter_name: adapter_info.name,
            adapter_backend: adapter_info.backend,
            volume: None,
            selected_sweep_index: None,
            uploaded: None,
        })
    }
}

#[wasm_bindgen]
impl RadarWebRenderer {
    /// The negotiated GPU adapter's name, e.g. `"NVIDIA GeForce RTX 5070"`
    /// -- for the test page's "GPU adapter acquired: <name>" status line.
    #[wasm_bindgen(getter, js_name = adapterName)]
    pub fn adapter_name(&self) -> String {
        self.adapter_name.clone()
    }

    /// Which `wgpu` backend actually got negotiated (`"BrowserWebGpu"`,
    /// `"Gl"`, ...). Worth surfacing rather than assuming WebGPU: whether
    /// this crate's `webgl` fallback feature was ever actually exercised
    /// depends on what the browser supports, and that is exactly the kind
    /// of thing this proof exists to observe rather than assume.
    #[wasm_bindgen(getter, js_name = backend)]
    pub fn backend(&self) -> String {
        format!("{:?}", self.adapter_backend)
    }

    /// Decode `archive2_bytes` (a full raw Archive II Level II volume) and
    /// select the lowest-elevation sweep that carries a REF (reflectivity)
    /// moment -- see [`crate::sweep_select`]. Returns metadata about that
    /// sweep for the caller to display/log; does not touch the GPU.
    ///
    /// A `#[wasm_bindgen]`-exported `&[u8]` parameter accepts a JS
    /// `Uint8Array` directly (copied into this function's own `Vec<u8>` by
    /// the generated glue), so no explicit `js_sys::Uint8Array` type is
    /// needed in this signature.
    #[wasm_bindgen(js_name = decodeSweep)]
    pub fn decode_sweep(&mut self, archive2_bytes: &[u8]) -> Result<SweepInfo, JsValue> {
        let volume = nexrad_level2::decode_volume(archive2_bytes)
            .map_err(|e| JsValue::from_str(&format!("failed to decode Archive II volume: {e}")))?;

        let sweep_index = pick_lowest_elevation_sweep_index(&volume, MomentKind::Reflectivity)
            .ok_or_else(|| {
                JsValue::from_str("no sweep in this volume carries a REF (reflectivity) moment")
            })?;
        let sweep = &volume.sweeps[sweep_index];
        let info = SweepInfo {
            site_icao: volume.site.icao.clone(),
            sweep_count: volume.sweeps.len() as u32,
            elevation_deg: sweep.elevation_angle_deg,
            radial_count: sweep.radials.len() as u32,
        };

        self.selected_sweep_index = Some(sweep_index);
        self.volume = Some(volume);
        // A new decode invalidates any GPU resources uploaded for a
        // previous sweep; `render_frame` rebuilds them lazily.
        self.uploaded = None;
        Ok(info)
    }

    /// Upload the selected sweep's GPU storage buffers/lookup texture/
    /// palette (first call after a `decodeSweep` only; subsequent calls
    /// reuse them) and render + present one frame to the `<canvas>`
    /// surface. Must be called after a successful `decodeSweep`.
    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&mut self) -> Result<(), JsValue> {
        let volume = self.volume.as_ref().ok_or_else(|| {
            JsValue::from_str("renderFrame called before a successful decodeSweep")
        })?;
        let sweep_index = self
            .selected_sweep_index
            .expect("set together with `volume` in decode_sweep");
        let sweep = &volume.sweeps[sweep_index];

        if self.uploaded.is_none() {
            let buffer_data = build_sweep_buffers(sweep, MomentKind::Reflectivity);
            let azimuth_resolution = sweep
                .radials
                .first()
                .map(|r| r.azimuth_resolution)
                .unwrap_or(AzimuthResolution::One);
            let lookup_table = build_radial_lookup(
                &buffer_data.source_radials,
                azimuth_resolution,
                DEFAULT_LOOKUP_TEXEL_COUNT,
            );
            let palette_lut = build_palette_lut(
                &default_ref_palette_stops(),
                DEFAULT_REF_MIN_DBZ,
                DEFAULT_REF_MAX_DBZ,
                PALETTE_TEXEL_COUNT,
            );

            let sweep_gpu =
                gpu::upload_sweep(&self.device, &self.queue, &buffer_data, &lookup_table);
            let palette_gpu = gpu::upload_palette(&self.device, &self.queue, &palette_lut);
            let max_range_km = farthest_gate_edge_km(&buffer_data.radial_meta);
            let uniforms_gpu = UniformsGpu::new(
                &self.device,
                GpuUniforms {
                    clip_to_world: clip_to_world((0.0, 0.0), (max_range_km, max_range_km)),
                    site_max_range_km: max_range_km,
                    lookup_texel_count: DEFAULT_LOOKUP_TEXEL_COUNT,
                    palette_min_dbz: DEFAULT_REF_MIN_DBZ,
                    palette_max_dbz: DEFAULT_REF_MAX_DBZ,
                    _pad: [0; 4],
                },
            );
            let bind_group = gpu::create_bind_group(
                &self.device,
                &self.pipeline.bind_group_layout,
                &uniforms_gpu,
                &sweep_gpu,
                &palette_gpu,
            );

            self.uploaded = Some(UploadedSweep {
                _sweep_gpu: sweep_gpu,
                _palette_gpu: palette_gpu,
                _uniforms_gpu: uniforms_gpu,
                bind_group,
            });
        }
        let bind_group = &self
            .uploaded
            .as_ref()
            .expect("just populated above if it was empty")
            .bind_group;

        // Off-screen `radar-render` renders into a plain `RenderTarget` it
        // owns; a canvas has no such texture to own ahead of time -- one
        // must be acquired fresh from the surface each frame, presented,
        // and dropped. That acquisition/present pair is this function's
        // only code `radar-render`'s native harness has no equivalent of.
        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            other => {
                return Err(JsValue::from_str(&format!(
                    "surface texture unavailable: {other:?}"
                )))
            }
        };
        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // `RenderTarget::texture` is not actually read by `gpu::render_frame`
        // (only `.view` is), but cloning it (a cheap handle clone, not a GPU
        // copy) keeps this a fully-typed, honestly-constructed
        // `RenderTarget` rather than a field left semantically wrong.
        let target = RenderTarget {
            texture: surface_texture.texture.clone(),
            view,
            width: self.width,
            height: self.height,
        };

        gpu::render_frame(
            &self.device,
            &self.queue,
            &self.pipeline.pipeline,
            bind_group,
            &target,
        );
        // Deliberately no `gpu::wait_for_gpu` here: on the WebGPU backend
        // `Device::poll` is documented as a no-op (the browser polls the
        // device automatically), and this presentation path has no
        // readback that would need to wait for completion either way --
        // unlike `radar-render`'s harness, which waits only to measure
        // real GPU frame time.
        self.queue.present(surface_texture);

        Ok(())
    }
}

/// The farthest gate's far edge across every radial in `radial_meta`, in km
/// -- used as `site_max_range_km` so the default camera frames the whole
/// sweep with no wasted transparent margin. Identical to the private helper
/// of the same name in `radar-render`'s harness (`src/bin/harness.rs`),
/// duplicated here rather than exported from `radar-render` since it is
/// harness/glue-level camera-framing logic, not part of that crate's public
/// rendering API.
fn farthest_gate_edge_km(radial_meta: &[GpuRadialMeta]) -> f32 {
    radial_meta
        .iter()
        .map(|r| r.first_gate_range_km + r.gate_spacing_km * (r.gate_count as f32 - 0.5))
        .fold(0.0f32, f32::max)
        .max(1.0)
}
