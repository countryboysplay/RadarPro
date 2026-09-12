# ADR-0002: `wgpu` as the GPU abstraction for radar rendering

Status: Accepted

## Context
RadarPro must render large polar radar volumes (potentially millions of gates) and gridded model layers (`GridLayer`) interactively, across web (via `apps/web`) and desktop (via `apps/desktop`, Tauri-based) targets, without depending on a single native graphics API per platform. `GLOBAL_CONTRACT.md` states large binary arrays must not live in React state and that radar rendering must not depend directly on React or MapLibre internals — the render path needs a GPU abstraction that is native to Rust (per ADR-0001) and works both compiled natively and compiled to WebAssembly targeting WebGPU/WebGL.

## Decision
`wgpu` is the preferred abstraction for radar GPU rendering (`radar-render`), unless measured evidence (profiling, a concrete platform gap, or an unsupported feature) justifies changing it. `wgpu` is used for both the polar radar layer and gridded layers so the render crate has one GPU backend to target instead of one per platform.

## Alternatives
- **Direct native APIs per platform (Vulkan/Metal/DX12 directly)**: maximum control and peak performance, but means writing and maintaining three backends, none of which run in a browser without an additional abstraction layer anyway — excessive complexity for a project whose current priority is a correct, shippable renderer, not squeezing out the last percent of GPU throughput.
- **Bevy or another full game engine's renderer**: would provide more built-in tooling (scene graph, ECS, asset pipeline) but pulls in a large dependency surface and opinions (ECS-first architecture) that don't fit a radar-domain-first design; RadarPro needs a rendering abstraction, not a game engine.
- **WebGL-only via a JS/TS rendering library (e.g., deck.gl, three.js) on the web, with a separate native renderer for desktop**: fastest path to a web prototype and reuses a mature ecosystem, but violates the architecture rule that radar rendering must not depend directly on React/web-only internals, and creates exactly the duplicated-implementation problem ADR-0001 is meant to avoid — two renderers to keep numerically and visually consistent.
- **Vulkan via `ash` with a hand-rolled abstraction**: comparable performance to `wgpu` on native targets but no path to web without separately targeting WebGPU, and no ready abstraction over swapchain/pipeline differences that `wgpu` already provides.

## Consequences
**Benefits**: one Rust rendering codebase targets native (Vulkan/Metal/DX12 via `wgpu`'s backends) and web (via WebGPU, falling back to WebGL2) from `apps/web` through Wasm; keeps the render crate free of React/MapLibre coupling, satisfying the architecture rule that mapping is an adapter, not a core dependency; `wgpu`'s Rust-native API keeps radar-render aligned with the rest of the Rust core (ADR-0001) instead of introducing a second language at the rendering boundary.

**Costs/risks**: `wgpu`'s WebGPU support and browser support are still maturing relative to native WebGL/Canvas2D approaches, so web rendering fidelity/perf may lag native until browser WebGPU support is universal; `wgpu` is a step removed from vendor-specific features that a direct Vulkan/Metal implementation could exploit; the project takes on `wgpu`'s upgrade cadence and any of its own abstraction bugs.

**Reversibility**: this is an internal implementation detail of `radar-render`; as long as the crate boundary is respected (no React/MapLibre code reaching into the renderer), swapping the GPU backend later is a `radar-render`-internal change, not a cross-cutting rewrite.

## Validation
No renderer exists yet — `radar-render` is intentionally not created in S00 (see `ARCHITECTURE.md`: "Do not create every planned crate before it is needed"). This decision is recorded now because it is a foundational, hard-to-reverse-late choice referenced by the intended repository shape. Validation criteria for when `radar-render` is built: `wgpu` must successfully initialize and draw a basic polar geometry test scene on at least one native backend and one web backend (WebGPU or WebGL2 fallback) before the decision is considered validated in practice; if that fails, this ADR should be revisited with the measured evidence that motivated the change.
