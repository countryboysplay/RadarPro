# ADR-0005: Cache sits between normalized providers and analysis/render, not inside providers or the UI

Status: Accepted

## Context
`ARCHITECTURE.md` places caching in a specific position in the primary data flow: `Observation/Forecast/Alert Providers -> normalized domain models -> cache/analysis/render -> map/UI`, and again in the concurrency pipeline: `network -> download queue -> decode workers -> decoded cache -> GPU upload queue -> renderer`. Radar volumes and gridded model data are large binary arrays; `GLOBAL_CONTRACT.md` forbids putting these in React state and requires that the UI thread never synchronously decode complete volumes, and that stale network/decode work be cancellable/deprioritizable. Both directives imply the cache must live on the Rust/core side of the boundary, downstream of decode and normalization, and upstream of rendering — not embedded inside individual providers (which would duplicate caching per source) and not implemented ad hoc in the UI layer (which would violate the large-binary-arrays rule and the adapter boundary from ADR-0003).

## Decision
Caching direction flows one way: decoded/normalized data (post provider-boundary translation, per ADR-0004) is cached once, centrally, before analysis and rendering consume it. Concretely: decode workers write into a decoded cache; render consumes from that cache via a GPU upload queue; the cache is addressed by handles/metadata that the UI layer holds, while the actual heavy buffers stay in Rust-side resources (`radar-cache`, per `ARCHITECTURE.md`). Providers do not maintain their own private caches of normalized data — they produce it and hand it downstream. The UI/render layers never own the canonical copy; they reference it.

## Alternatives
- **Cache at the provider layer (each provider caches its own raw/normalized responses)**: simplest to implement per-provider and keeps caching close to fetch logic, but duplicates cache invalidation/eviction logic per provider, makes it hard to reason about total memory usage across sources, and doesn't help the concurrency pipeline's real bottleneck, which is decode cost, not fetch cost.
- **Cache at the UI/render layer (e.g., keep decoded frames in React/JS-side structures for reuse)**: convenient for a web-only implementation, but directly violates "large binary arrays do not live in React state," is not reusable by the desktop shell, and reintroduces the coupling ADR-0003 exists to prevent.
- **No dedicated cache layer; re-decode on every render/analysis request**: eliminates cache-invalidation complexity entirely, but re-decoding full volumes synchronously on demand conflicts with "UI thread must not synchronously decode complete radar volumes" and would make timeline scrubbing/animation (replaying prior volumes) prohibitively expensive.

## Consequences
**Benefits**: one cache implementation and one eviction/memory-budget policy (`radar-cache`) instead of one per provider or one per UI framework; keeps heavy buffers out of React state and out of provider code, satisfying both the architecture and reliability rules; supports cancellation/deprioritization of stale decode work because the pipeline stage (decode -> cache) is explicit and inspectable, rather than caching being an incidental side effect of fetch or render code; the same cache can serve multiple consumers (render, future analysis features) without re-decoding.

**Costs/risks**: introduces a stateful, memory-bounded component that must define its own eviction policy (LRU by volume/sweep, memory budget, or similar) — getting this wrong risks unbounded memory growth for long timelines or many loaded sites; adds a layer between decode and render that must be kept simple enough not to become a bottleneck itself; handle/metadata design for UI-side references needs to be stable across cache evictions and reloads.

**Reliability implications**: because remote data is unreliable and untrusted, the cache boundary is also a natural place to isolate a malformed or partially-downloaded volume from propagating further downstream — a cache miss or explicit decode-failure entry is preferable to letting bad data reach the renderer.

## Validation
No cache crate exists yet in S00 (`radar-cache` is intentionally not created before it is needed, per `ARCHITECTURE.md`). This ADR is recorded now because it fixes the direction of data flow that decode workers, render, and the UI handle layer must be built against from S01 onward. It will be validated when `radar-cache` is introduced: the check is that decode workers write into the cache (not into provider or UI code), that render/analysis read from it via handles, and that a stale/cancelled decode does not leave a dangling or memory-leaking cache entry.
