//! `radar-web` -- browser/WebAssembly glue exposing the already-proven
//! `nexrad-level2` decode / `radar-geo` polar-geometry math /
//! `radar-render` `wgpu` renderer stack to a `<canvas>`, via WebAssembly and
//! WebGPU/WebGL2.
//!
//! Originally an S04 proof-of-concept (decode + render exactly one fixed
//! sweep/moment); generalized in S05 to decode a volume once and let a
//! caller switch between any `(elevation, moment)` selection without
//! re-decoding, load/apply a user-supplied color table
//! (`radar_render::color_table`), probe a cursor position against the
//! currently-rendered sweep, and fetch range-ring geometry -- see
//! [`browser`]'s module docs for the full S05 API.
//!
//! # Scope
//!
//! This crate is deliberately thin. Per `ARCHITECTURE.md` ("Mapping is
//! behind an adapter. Radar rendering must not depend directly on React or
//! MapLibre internals" / "Map integration is an adapter boundary, not a
//! core dependency"), it contains **no decoding, geometry, or palette-math
//! logic of its own** -- every byte of Archive II parsing is
//! `nexrad_level2::decode_volume`, every GPU resource/pipeline is a
//! `radar_render::gpu`/`color_table` type or function, and every
//! cursor-probe/range-ring computation is a `radar_geo` function, all used
//! unmodified. The only genuinely new code here is:
//!
//! 1. [`sweep_select`]/[`render_select`]/[`srv_select`]: which sweep(s) to
//!    render, what metadata to expose for a picker, and (S11 Phase 2b)
//!    building a synthetic Storm-Relative Velocity sweep from an already-
//!    resolved VEL sweep (pure logic, no `wgpu`/`wasm-bindgen` dependency --
//!    see those modules).
//! 2. [`browser`] (wasm32-only): acquiring a `wgpu::Surface` from a
//!    `<canvas>` and presenting frames to it (which the native
//!    `radar-render` harness never needed -- it renders off-screen and
//!    reads pixels back for a PNG), plus the wasm-bindgen API surface
//!    itself. See that module's docs for the full design, including why
//!    `radar_render::gpu::GpuContext::request`'s `pollster`-free `async fn`
//!    turned out *not* to need a wasm32-specific replacement, and where a
//!    genuinely new async/GPU-negotiation choice was required instead.
//!
//! This is explicitly **not** the production web integration: no MapLibre
//! rendering code, no live NOAA discovery/download, no `apps/web` UI
//! changes (pickers, animation, a color-table editor, or the actual
//! probe-on-click interaction) -- those consume this crate's API in a
//! separate follow-up task. See
//! `Agent Context/context/stages/S05-radar-workstation.md` and this crate's
//! `README.md` for how to rebuild the wasm module and run the standalone
//! test page under `www/`.
//!
//! # Why `sweep_select`/`render_select`/`srv_select` are not wasm32-gated but `browser` is
//!
//! `wgpu`'s `SurfaceTarget::Canvas` variant, and the `web-sys`/
//! `wasm-bindgen` types this crate's real glue is built from, only exist at
//! all under `cfg(target_arch = "wasm32")` (`wgpu`'s own `web` cfg alias
//! requires it, and this crate's `Cargo.toml` puts those dependencies under
//! a matching `[target.'cfg(target_arch = "wasm32")'.dependencies]` table
//! so a native build never has to resolve a browser GPU stack it cannot
//! use). [`sweep_select`]/[`render_select`]/[`srv_select`] have no such
//! dependency, so they stay available and unit-tested on every target --
//! `cargo build --workspace` and `cargo test -p radar-web` both exercise
//! them on the host, alongside every other native crate in this workspace.
//! [`render_select`]'s `#[cfg(test)]` module additionally depends on
//! `nexrad-level2`/`radar-render` as ordinary `[dev-dependencies]` (not
//! wasm32-gated either) to decode a real fixture and build its GPU-buffer-
//! input representation on the host, proving the "decode once, switch
//! selection freely" property without needing a GPU or a browser.

pub mod render_select;
pub mod srv_select;
pub mod sweep_select;

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_arch = "wasm32")]
pub use browser::{
    init_gpu, range_rings_geojson, ColorTableApplyResult, GateProbeResult, RadarWebRenderer,
    VolumeSummary,
};
