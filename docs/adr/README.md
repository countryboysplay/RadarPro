# Architecture Decision Records

This index lists RadarPro's ADRs in order. See `Agent Context/templates/ADR_TEMPLATE.md` for the format new ADRs should follow, and `CLAUDE.md` for when an ADR is required (any architecture deviation).

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-rust-core-for-domain-and-performance.md) | Rust for radar decoding, domain models, and performance-sensitive processing | Accepted |
| [0002](0002-wgpu-for-gpu-radar-rendering.md) | `wgpu` as the GPU abstraction for radar rendering | Accepted |
| [0003](0003-map-integration-behind-adapter-boundary.md) | Map integration lives behind an adapter boundary | Accepted |
| [0004](0004-provider-boundary-architecture.md) | Provider boundary architecture for observations, forecasts, and alerts | Accepted |
| [0005](0005-cache-direction.md) | Cache sits between normalized providers and analysis/render, not inside providers or the UI | Accepted |
| [0006](0006-earth-model-for-radar-geometry.md) | Spherical Earth + 4/3 effective-Earth-radius model for radar geometry | Accepted |
| [0007](0007-radar-render-gpu-data-representation.md) | Storage-buffer + radial-index lookup-texture GPU representation for polar radar rendering | Accepted |
