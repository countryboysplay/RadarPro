# ADR-0003: Map integration lives behind an adapter boundary

Status: Accepted

## Context
RadarPro overlays radar, gridded forecast, alert, and point data on a basemap. The planned map stack differs by target: MapLibre GL JS on web (`apps/web`), a Tauri desktop shell, and potentially MapLibre Native for native/mobile later. `GLOBAL_CONTRACT.md` requires that radar rendering must not depend directly on React or MapLibre internals, and `ARCHITECTURE.md` states explicitly that "map integration is an adapter boundary, not a core dependency." Without an enforced boundary, it is easy for radar-render or domain code to accumulate MapLibre-specific coordinate transforms, camera/projection assumptions, or React component coupling that would need to be rewritten for every future map/UI target.

## Decision
Map integration (MapLibre GL JS today, MapLibre Native or another map engine later) is treated as a swappable adapter that consumes RadarPro's core layers (`PolarRadarLayer`, `GridLayer`, `VectorLayer`, `PointLayer`) through a defined interface, rather than the core depending on the map library's types, coordinate system internals, or lifecycle. The Rust rendering core (`radar-render`, per ADR-0002) and domain/geospatial code (`radar-geo`) never import MapLibre or React; the adapter layer (web app code) is responsible for translating between the map engine's camera/projection state and the layer APIs the core exposes.

## Alternatives
- **Render radar directly as a MapLibre custom layer with tight coupling to its WebGL/camera internals**: fastest initial integration on web, but locks the polar radar geometry and rendering logic to MapLibre's specific API and rendering context, contradicting the requirement to keep radar rendering independent of MapLibre internals, and blocks reuse on desktop/native without a rewrite.
- **Build RadarPro's own full map/basemap stack instead of adapting an existing one**: maximizes control and avoids any adapter, but is a large, unjustified scope increase (tile fetching, vector tile rendering, basemap styling, geocoding) that duplicates mature open-source work MapLibre already provides — architecture astronautics the project does not need at this stage.
- **Couple directly to React state for map/camera synchronization**: simplest within `apps/web` alone, but conflicts with "large binary arrays do not live in React state" and would make the radar layer impossible to reuse in a non-React shell (desktop/Tauri, or a future native app).

## Consequences
**Benefits**: the same `PolarRadarLayer`/`GridLayer`/`VectorLayer`/`PointLayer` core can be adapted to MapLibre GL JS today and to MapLibre Native or a different engine later without touching radar-render or domain logic; keeps large radar/grid buffers in Rust-side resources exposed by handles/metadata rather than in React state or MapLibre's own object graph; isolates MapLibre API churn (breaking changes, deprecated APIs) to the adapter rather than the core.

**Costs/risks**: an adapter layer is extra indirection and code that a direct MapLibre integration would not need; if the adapter's interface is designed poorly it can become a leaky abstraction that re-exposes MapLibre concepts anyway, so the interface must be reviewed against real usage rather than designed speculatively.

**Reversibility**: high — because the boundary is explicit, changing map engines later is contained to the adapter and the web app, not a rewrite of radar-render or radar-geo.

## Validation
No map adapter exists yet in S00 (`apps/web` is a development shell only; radar layers are not yet built). This ADR is recorded now because it constrains how `apps/web` and future rendering crates should be structured from the start. It will be validated when `apps/web` first renders a radar/grid layer: the check is that no MapLibre or React types appear in `radar-render`/`radar-geo` crate dependencies, and that the same core layer APIs are exercised by at least the web adapter.
