//! wasm32-only browser glue: turns a `<canvas>` element and a `Uint8Array`
//! of raw Archive II bytes into a rendered radar sweep, using
//! `radar_render::gpu`'s existing pipeline/bind-group/buffer/texture code
//! unmodified. The one genuinely new piece of `wgpu` code in this crate is
//! [`RadarWebRenderer::create`]'s surface acquisition/configuration and
//! [`RadarWebRenderer::select_and_render`]'s presentation -- `radar-render`'s
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
//!
//! # S05: decode once, switch (elevation, moment) selection freely
//!
//! [`RadarWebRenderer::decode_volume`] decodes the full
//! [`radar_types::Volume`] (every sweep, every moment) exactly once and
//! keeps it in memory; [`RadarWebRenderer::select_and_render`] re-runs only
//! the CPU-side buffer-build + GPU upload + render/present steps for a new
//! `(sweep_index, MomentKind)` selection, never calling
//! `nexrad_level2::decode_volume` again. See [`crate::render_select`] for
//! the pure, host-testable selection logic this delegates to, and its
//! `#[cfg(test)]` module for a structural proof (no GPU required) that
//! switching selection really does skip re-decoding.
//!
//! This module also exposes S05 deliverables 2 (the original color-table
//! format, [`radar_render::color_table`]) and 3 (`radar-geo`'s cursor-probe
//! and range-ring logic) through [`RadarWebRenderer::load_color_table`]/
//! [`RadarWebRenderer::active_color_table_json`]/
//! [`RadarWebRenderer::probe_gate`] and the free function
//! [`range_rings_geojson`].

use std::collections::HashMap;

use js_sys::Promise;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;
use web_sys::HtmlCanvasElement;

use radar_render::camera::clip_to_world;
use radar_render::color_table::{build_lut_from_table, default_color_table, ColorTable};
use radar_render::gpu::{
    self, GpuUniforms, PaletteGpuResources, RenderTarget, SweepGpuResources, SweepPipeline,
    UniformsGpu,
};
use radar_render::lookup_texture::{build_radial_lookup, DEFAULT_LOOKUP_TEXEL_COUNT};
use radar_render::palette::PALETTE_TEXEL_COUNT;
use radar_render::sweep_buffers::{build_sweep_buffers, GpuRadialMeta};
use radar_types::{AzimuthResolution, GateValue, MomentKind, Volume};

use crate::{render_select, sweep_select};

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

/// Metadata about a decoded [`radar_types::Volume`], readable from JS via
/// plain getters -- enough for a UI to build an elevation picker.
/// Deliberately not the full `Volume` -- that carries every sweep's every
/// radial's gate array, which belongs on the GPU
/// ([`RadarWebRenderer::select_and_render`]), not marshalled through the
/// JS boundary.
///
/// Per-sweep moment availability (which [`MomentKind`]s a given sweep
/// actually carries -- not every sweep in a VCP carries every moment, e.g.
/// SAILS/split cuts) is a separate call,
/// [`RadarWebRenderer::moment_wire_codes_for_sweep`], rather than baked
/// into this struct, so a UI does not have to marshal a full
/// sweep-by-moment matrix up front if it only needs one sweep's moments at
/// a time.
#[wasm_bindgen]
pub struct VolumeSummary {
    site_icao: String,
    sweep_count: u32,
    elevation_degs: Vec<f32>,
}

#[wasm_bindgen]
impl VolumeSummary {
    #[wasm_bindgen(getter, js_name = siteIcao)]
    pub fn site_icao(&self) -> String {
        self.site_icao.clone()
    }

    #[wasm_bindgen(getter, js_name = sweepCount)]
    pub fn sweep_count(&self) -> u32 {
        self.sweep_count
    }

    /// One elevation angle (degrees) per sweep, in `volume.sweeps` order --
    /// index `i` here is sweep index `i` everywhere else in this API
    /// (`selectAndRender`, `momentWireCodesForSweep`, `probeGate`).
    #[wasm_bindgen(js_name = elevationDegs)]
    pub fn elevation_degs(&self) -> Vec<f32> {
        self.elevation_degs.clone()
    }
}

/// What [`RadarWebRenderer::load_color_table`] applied, for a UI to
/// confirm/display after a successful load.
#[wasm_bindgen]
pub struct ColorTableApplyResult {
    name: String,
    applied_to: Vec<String>,
}

#[wasm_bindgen]
impl ColorTableApplyResult {
    #[wasm_bindgen(getter, js_name = name)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// [`MomentKind::wire_code`] strings this table is now the active
    /// palette for.
    #[wasm_bindgen(js_name = appliedTo)]
    pub fn applied_to(&self) -> Vec<String> {
        self.applied_to.clone()
    }
}

/// Result of [`RadarWebRenderer::probe_gate`]: a cursor resolved to a
/// specific radial/gate on the currently-probed sweep, with its
/// missing/range-folded/valid state kept distinct -- never collapsed --
/// per `GLOBAL_CONTRACT.md`.
#[wasm_bindgen]
pub struct GateProbeResult {
    azimuth_deg: f64,
    slant_range_km: f64,
    radial_index: u32,
    gate_index: u32,
    /// One of `"valid"`, `"missing"`, or `"range_folded"`.
    state: String,
    /// The raw physical value, present only when `state == "valid"`.
    value: Option<f64>,
    /// `value`'s physical unit (from the active color table's `units`
    /// field), present only when `state == "valid"`.
    units: Option<String>,
}

#[wasm_bindgen]
impl GateProbeResult {
    #[wasm_bindgen(getter, js_name = azimuthDeg)]
    pub fn azimuth_deg(&self) -> f64 {
        self.azimuth_deg
    }

    #[wasm_bindgen(getter, js_name = slantRangeKm)]
    pub fn slant_range_km(&self) -> f64 {
        self.slant_range_km
    }

    #[wasm_bindgen(getter, js_name = radialIndex)]
    pub fn radial_index(&self) -> u32 {
        self.radial_index
    }

    #[wasm_bindgen(getter, js_name = gateIndex)]
    pub fn gate_index(&self) -> u32 {
        self.gate_index
    }

    #[wasm_bindgen(getter, js_name = state)]
    pub fn state(&self) -> String {
        self.state.clone()
    }

    #[wasm_bindgen(getter, js_name = value)]
    pub fn value(&self) -> Option<f64> {
        self.value
    }

    #[wasm_bindgen(getter, js_name = units)]
    pub fn units(&self) -> Option<String> {
        self.units.clone()
    }
}

/// GPU resources for one uploaded `(sweep, moment)` selection: everything
/// [`RadarWebRenderer::select_and_render`] needs to draw it again without
/// re-uploading, built once per distinct selection.
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

/// A live GPU device/surface pair targeting one `<canvas>`, the most
/// recently decoded [`Volume`] (if any), any custom color tables loaded
/// via [`RadarWebRenderer::load_color_table`], and whatever `(sweep,
/// moment)` selection has most recently been uploaded. Constructed via
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
    /// User-loaded color tables, keyed by the moment(s) they were applied
    /// to. A moment absent here falls back to
    /// [`radar_render::color_table::default_color_table`] -- see
    /// [`RadarWebRenderer::active_color_table_for`].
    color_tables: HashMap<MomentKind, ColorTable>,
    /// The `(sweep_index, MomentKind)` selection [`UploadedSweep`] (if any)
    /// currently reflects.
    selection: Option<(usize, MomentKind)>,
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
            color_tables: HashMap::new(),
            selection: None,
            uploaded: None,
        })
    }

    /// `moment`'s active palette: a user-loaded override if one was
    /// applied via [`RadarWebRenderer::load_color_table`], otherwise this
    /// crate's own built-in default for that moment.
    fn active_color_table_for(&self, moment: MomentKind) -> ColorTable {
        self.color_tables
            .get(&moment)
            .cloned()
            .unwrap_or_else(|| default_color_table(moment))
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

    /// Decode `archive2_bytes` (a full raw Archive II Level II volume)
    /// **once**, keeping every sweep/moment in memory. Does not touch the
    /// GPU or select anything to render -- call
    /// [`RadarWebRenderer::select_and_render`] afterward with a chosen
    /// `(sweep_index, moment)` pair.
    ///
    /// A previously-loaded color table (via
    /// [`RadarWebRenderer::load_color_table`]) is unaffected by decoding a
    /// new volume -- palettes are a per-moment display choice, independent
    /// of which volume's data is being displayed.
    ///
    /// A `#[wasm_bindgen]`-exported `&[u8]` parameter accepts a JS
    /// `Uint8Array` directly (copied into this function's own `Vec<u8>` by
    /// the generated glue), so no explicit `js_sys::Uint8Array` type is
    /// needed in this signature.
    #[wasm_bindgen(js_name = decodeVolume)]
    pub fn decode_volume(&mut self, archive2_bytes: &[u8]) -> Result<VolumeSummary, JsValue> {
        let volume = nexrad_level2::decode_volume(archive2_bytes)
            .map_err(|e| JsValue::from_str(&format!("failed to decode Archive II volume: {e}")))?;

        let summary = VolumeSummary {
            site_icao: volume.site.icao.clone(),
            sweep_count: volume.sweeps.len() as u32,
            elevation_degs: volume
                .sweeps
                .iter()
                .map(|s| s.elevation_angle_deg)
                .collect(),
        };

        self.volume = Some(volume);
        // A new decode invalidates any GPU resources uploaded for the
        // previous volume's selection; `select_and_render` rebuilds them
        // lazily on the next call.
        self.uploaded = None;
        self.selection = None;
        Ok(summary)
    }

    /// Every [`MomentKind::wire_code`] present on at least one radial of
    /// sweep `sweep_index` of the most recently decoded volume -- for
    /// building a per-elevation moment picker. Must be called after a
    /// successful [`RadarWebRenderer::decode_volume`].
    #[wasm_bindgen(js_name = momentWireCodesForSweep)]
    pub fn moment_wire_codes_for_sweep(&self, sweep_index: u32) -> Result<Vec<String>, JsValue> {
        let volume = self.volume.as_ref().ok_or_else(|| {
            JsValue::from_str("momentWireCodesForSweep called before a successful decodeVolume")
        })?;
        let summaries = render_select::volume_sweep_summaries(volume);
        let summary = summaries.get(sweep_index as usize).ok_or_else(|| {
            JsValue::from_str(&format!(
                "sweep index {sweep_index} out of range (volume has {} sweeps)",
                summaries.len()
            ))
        })?;
        Ok(summary
            .moments
            .iter()
            .map(|kind| kind.wire_code().to_string())
            .collect())
    }

    /// The lowest-elevation sweep index carrying `moment_wire_code`, or
    /// `None` if no sweep does -- the same default-selection rule this
    /// crate's earlier S04 proof always used ([`crate::sweep_select`]),
    /// exposed as a convenience for a UI's initial pick before the user
    /// has chosen anything.
    #[wasm_bindgen(js_name = defaultSweepIndexForMoment)]
    pub fn default_sweep_index_for_moment(
        &self,
        moment_wire_code: &str,
    ) -> Result<Option<u32>, JsValue> {
        let moment = resolve_moment(moment_wire_code)?;
        let volume = self.volume.as_ref().ok_or_else(|| {
            JsValue::from_str("defaultSweepIndexForMoment called before a successful decodeVolume")
        })?;
        Ok(sweep_select::pick_lowest_elevation_sweep_index(volume, moment).map(|i| i as u32))
    }

    /// Parse, validate, and apply a color table (see
    /// `COLOR_TABLE_FORMAT.md` and [`radar_render::color_table`]) from a
    /// JSON string. On success, the table becomes the active palette for
    /// every [`MomentKind`] it names (replacing that moment's previous
    /// active table, default or otherwise) and this method returns which
    /// moments were affected. On a malformed table, returns a `JsValue`
    /// error describing exactly what failed validation -- never panics.
    #[wasm_bindgen(js_name = loadColorTable)]
    pub fn load_color_table(&mut self, json: &str) -> Result<ColorTableApplyResult, JsValue> {
        let table = ColorTable::from_json(json).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let moments = table.resolved_moments();
        let applied_to: Vec<String> = moments
            .iter()
            .map(|kind| kind.wire_code().to_string())
            .collect();
        let name = table.name.clone();

        for moment in moments {
            self.color_tables.insert(moment, table.clone());
            self.invalidate_upload_if_selected(moment);
        }

        Ok(ColorTableApplyResult { name, applied_to })
    }

    /// Revert `moment_wire_code` to this crate's built-in default palette,
    /// discarding any table previously loaded for it via
    /// [`RadarWebRenderer::load_color_table`].
    #[wasm_bindgen(js_name = resetColorTable)]
    pub fn reset_color_table(&mut self, moment_wire_code: &str) -> Result<(), JsValue> {
        let moment = resolve_moment(moment_wire_code)?;
        self.color_tables.remove(&moment);
        self.invalidate_upload_if_selected(moment);
        Ok(())
    }

    /// The active color table for `moment_wire_code` (a user-loaded
    /// override, or this crate's built-in default), serialized back to the
    /// same JSON shape [`RadarWebRenderer::load_color_table`] reads -- for
    /// a UI to display or offer as a starting point for editing.
    #[wasm_bindgen(js_name = activeColorTableJson)]
    pub fn active_color_table_json(&self, moment_wire_code: &str) -> Result<String, JsValue> {
        let moment = resolve_moment(moment_wire_code)?;
        self.active_color_table_for(moment)
            .to_json_pretty()
            .map_err(|e| JsValue::from_str(&e.to_string()))
    }

    /// Render + present one frame for the given `(sweep_index, moment)`
    /// selection. If this exact selection was the last one rendered, its
    /// GPU resources are reused unchanged (just a re-render); otherwise
    /// (a different elevation, a different moment, or the active color
    /// table for this moment changed since the last render) the CPU-side
    /// buffers/lookup texture/palette are rebuilt and re-uploaded first.
    ///
    /// Crucially, none of this ever re-decodes the Archive II bytes --
    /// every code path here operates on the [`Volume`] already decoded by
    /// [`RadarWebRenderer::decode_volume`]. Must be called after a
    /// successful `decodeVolume`.
    #[wasm_bindgen(js_name = selectAndRender)]
    pub fn select_and_render(
        &mut self,
        sweep_index: u32,
        moment_wire_code: &str,
    ) -> Result<(), JsValue> {
        let moment = resolve_moment(moment_wire_code)?;
        let selection = (sweep_index as usize, moment);

        if self.uploaded.is_none() || self.selection != Some(selection) {
            self.upload_selection(selection.0, moment, moment_wire_code)?;
            self.selection = Some(selection);
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

    /// Given the site's lat/lon, which sweep/moment is being probed, and a
    /// cursor's lat/lon, resolve the cursor to a specific radial/gate on
    /// that sweep and return its state and (if valid) value -- see
    /// [`GateProbeResult`]'s docs for the exact never-collapse guarantee.
    /// Returns `Ok(None)` when the cursor does not resolve to any
    /// radial/gate at all (outside the sweep's angular/range coverage),
    /// which is a normal "off the sweep" outcome, not an error.
    ///
    /// Reuses `radar-geo`'s already-tested
    /// [`radar_geo::locate_gate_value`]/[`radar_geo::cursor_to_polar`]
    /// directly rather than reimplementing the cursor -> polar ->
    /// radial/gate resolution here.
    #[wasm_bindgen(js_name = probeGate)]
    pub fn probe_gate(
        &self,
        site_lat_deg: f64,
        site_lon_deg: f64,
        sweep_index: u32,
        moment_wire_code: &str,
        cursor_lat_deg: f64,
        cursor_lon_deg: f64,
    ) -> Result<Option<GateProbeResult>, JsValue> {
        let moment = resolve_moment(moment_wire_code)?;
        let volume = self.volume.as_ref().ok_or_else(|| {
            JsValue::from_str("probeGate called before a successful decodeVolume")
        })?;
        let sweep = volume.sweeps.get(sweep_index as usize).ok_or_else(|| {
            JsValue::from_str(&format!(
                "sweep index {sweep_index} out of range (volume has {} sweeps)",
                volume.sweeps.len()
            ))
        })?;

        let site = radar_geo::LatLon::new(site_lat_deg, site_lon_deg);
        let cursor = radar_geo::LatLon::new(cursor_lat_deg, cursor_lon_deg);

        let Some((radial_gate, gate_value)) =
            radar_geo::locate_gate_value(site, sweep, moment, cursor)
        else {
            return Ok(None);
        };
        let polar = radar_geo::cursor_to_polar(site, f64::from(sweep.elevation_angle_deg), cursor)
            .expect(
                "locate_gate_value already resolved this exact cursor via cursor_to_polar \
                 internally, so the same call here must also succeed",
            );

        let (state, value, units) = match *gate_value {
            GateValue::Missing => ("missing".to_string(), None, None),
            GateValue::RangeFolded => ("range_folded".to_string(), None, None),
            GateValue::Value(v) => {
                let table = self.active_color_table_for(moment);
                ("valid".to_string(), Some(f64::from(v)), Some(table.units))
            }
        };

        Ok(Some(GateProbeResult {
            azimuth_deg: polar.azimuth_deg,
            slant_range_km: polar.slant_range_km,
            radial_index: radial_gate.radial_index as u32,
            gate_index: radial_gate.gate_index as u32,
            state,
            value,
            units,
        }))
    }
}

impl RadarWebRenderer {
    /// If `moment` is the moment of the currently-uploaded selection,
    /// invalidate the upload so the next `selectAndRender` call rebuilds
    /// it with the (now-changed) active color table -- called after
    /// [`RadarWebRenderer::load_color_table`]/
    /// [`RadarWebRenderer::reset_color_table`] change what "active
    /// palette" means for that moment.
    fn invalidate_upload_if_selected(&mut self, moment: MomentKind) {
        if let Some((_, selected_moment)) = self.selection {
            if selected_moment == moment {
                self.uploaded = None;
            }
        }
    }

    /// Build the CPU-side sweep buffers/lookup texture/palette for
    /// `(sweep_index, moment)` and upload them as a fresh
    /// [`UploadedSweep`], replacing `self.uploaded`. The only code path
    /// that touches `self.volume`/`nexrad_level2` state for a render --
    /// [`RadarWebRenderer::select_and_render`] calls this only when the
    /// selection or active palette actually changed.
    fn upload_selection(
        &mut self,
        sweep_index: usize,
        moment: MomentKind,
        moment_wire_code: &str,
    ) -> Result<(), JsValue> {
        // Scoped so the borrow of `self.volume` ends before this function
        // needs `&self.color_tables`/`&mut self` again -- `build_sweep_buffers`
        // returns an owned `SweepBufferData` (it clones each matching
        // radial), so nothing below depends on `volume`/`sweep` staying
        // borrowed.
        let buffer_data = {
            let volume = self.volume.as_ref().ok_or_else(|| {
                JsValue::from_str("selectAndRender called before a successful decodeVolume")
            })?;
            let sweep =
                render_select::resolve_sweep(volume, sweep_index, moment).ok_or_else(|| {
                    JsValue::from_str(&format!(
                        "sweep {sweep_index} does not carry moment {moment_wire_code}"
                    ))
                })?;
            build_sweep_buffers(sweep, moment)
        };

        let azimuth_resolution = buffer_data
            .source_radials
            .first()
            .map(|r| r.azimuth_resolution)
            .unwrap_or(AzimuthResolution::One);
        let lookup_table = build_radial_lookup(
            &buffer_data.source_radials,
            azimuth_resolution,
            DEFAULT_LOOKUP_TEXEL_COUNT,
        );

        let table = self.active_color_table_for(moment);
        let palette_lut = build_lut_from_table(&table, PALETTE_TEXEL_COUNT);
        let max_range_km = farthest_gate_edge_km(&buffer_data.radial_meta);

        let sweep_gpu = gpu::upload_sweep(&self.device, &self.queue, &buffer_data, &lookup_table);
        let palette_gpu = gpu::upload_palette(&self.device, &self.queue, &palette_lut);
        let uniforms_gpu = UniformsGpu::new(
            &self.device,
            GpuUniforms {
                clip_to_world: clip_to_world((0.0, 0.0), (max_range_km, max_range_km)),
                site_max_range_km: max_range_km,
                lookup_texel_count: DEFAULT_LOOKUP_TEXEL_COUNT,
                // Despite the field names (dating from the S03 REF-only
                // proof), these are now a generic palette-domain min/max --
                // whatever unit `table.units` documents (m/s, dB, deg, ...),
                // not necessarily dBZ. See `GpuUniforms`'s doc comment.
                palette_min_dbz: table.domain.min,
                palette_max_dbz: table.domain.max,
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
        Ok(())
    }
}

/// Range rings around `(site_lat_deg, site_lon_deg)` at each radius in
/// `radii_km`, as a GeoJSON-coordinate-ordered (`[lon, lat]`, per GeoJSON's
/// own convention -- longitude first) nested array: one inner array of
/// `[lon, lat]` pairs per ring, in the same order as `radii_km`, directly
/// usable as a MapLibre GeoJSON `MultiLineString`'s `coordinates`.
///
/// A free function (not a [`RadarWebRenderer`] method) since it needs no
/// decoded volume/GPU state at all -- it is a thin, direct exposure of
/// `radar-geo`'s [`radar_geo::range_rings`], returning geometry only, with
/// no knowledge of MapLibre or any other map renderer (`ARCHITECTURE.md`:
/// "map integration is an adapter boundary, not a core dependency").
#[wasm_bindgen(js_name = rangeRingsGeoJson)]
pub fn range_rings_geojson(
    site_lat_deg: f64,
    site_lon_deg: f64,
    radii_km: Vec<f64>,
    num_points: u32,
) -> JsValue {
    let site = radar_geo::LatLon::new(site_lat_deg, site_lon_deg);
    let rings = radar_geo::range_rings(site, &radii_km, num_points as usize);

    let outer = js_sys::Array::new();
    for ring in rings {
        let ring_coords = js_sys::Array::new();
        for point in ring {
            let pair = js_sys::Array::of2(
                &JsValue::from_f64(point.lon_deg),
                &JsValue::from_f64(point.lat_deg),
            );
            ring_coords.push(&pair);
        }
        outer.push(&ring_coords);
    }
    outer.into()
}

/// Resolve a JS-supplied moment wire code to a [`MomentKind`], or a
/// descriptive `JsValue` error -- shared by every method that takes one,
/// so an unknown code always fails the same documented way.
fn resolve_moment(moment_wire_code: &str) -> Result<MomentKind, JsValue> {
    MomentKind::from_wire_code(moment_wire_code)
        .ok_or_else(|| JsValue::from_str(&format!("unknown moment wire code {moment_wire_code:?}")))
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
