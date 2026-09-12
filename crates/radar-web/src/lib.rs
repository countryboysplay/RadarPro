//! `radar-web` -- S04 proof-of-concept: browser/WebAssembly glue proving the
//! already-proven S01-S03 stack (`nexrad-level2` decode -> `radar-render`'s
//! `wgpu` storage-buffer/lookup-texture sweep renderer) actually decodes and
//! renders a real NEXRAD sweep inside a real browser, via WebAssembly and
//! WebGPU/WebGL2.
//!
//! # Scope
//!
//! This crate is deliberately thin. Per `ARCHITECTURE.md` ("Mapping is
//! behind an adapter. Radar rendering must not depend directly on React or
//! MapLibre internals" / "Map integration is an adapter boundary, not a
//! core dependency"), it contains **no decoding or rendering logic of its
//! own** -- every byte of Archive II parsing is `nexrad_level2::decode_volume`
//! and every GPU resource/pipeline is a `radar_render::gpu` type or function,
//! used unmodified. The only genuinely new code here is:
//!
//! 1. [`sweep_select`]: which sweep to render (pure logic, mirrors
//!    `radar-render`'s own harness convention exactly -- see that module).
//! 2. [`browser`] (wasm32-only): acquiring a `wgpu::Surface` from a
//!    `<canvas>` and presenting frames to it, which the native
//!    `radar-render` harness never needed (it renders off-screen and reads
//!    pixels back for a PNG). See that module's docs for the full design,
//!    including why `radar_render::gpu::GpuContext::request`'s
//!    `pollster`-free `async fn` turned out *not* to need a wasm32-specific
//!    replacement, and where a genuinely new async/GPU-negotiation choice
//!    was required instead.
//!
//! This is explicitly **not** the production web integration: no MapLibre,
//! no live NOAA discovery/download, no `apps/web` changes. See
//! `Agent Context/context/stages/S04-map-live-radar.md` and this crate's
//! `README.md` for what comes next and how to run the standalone test page
//! under `www/`.
//!
//! # Why `sweep_select` is not wasm32-gated but everything else is
//!
//! `wgpu`'s `SurfaceTarget::Canvas` variant, and the `web-sys`/
//! `wasm-bindgen` types this crate's real glue is built from, only exist at
//! all under `cfg(target_arch = "wasm32")` (`wgpu`'s own `web` cfg alias
//! requires it, and this crate's `Cargo.toml` puts those dependencies under
//! a matching `[target.'cfg(target_arch = "wasm32")'.dependencies]` table
//! so a native build never has to resolve a browser GPU stack it cannot
//! use). [`sweep_select`] has no such dependency, so it stays available and
//! unit-tested on every target -- `cargo build --workspace` and
//! `cargo test -p radar-web` both exercise it on the host, alongside every
//! other native crate in this workspace.

pub mod sweep_select;

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_arch = "wasm32")]
pub use browser::{init_gpu, RadarWebRenderer, SweepInfo};
