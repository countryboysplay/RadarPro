# RadarPro

RadarPro is an open-source professional weather radar and forecasting
workstation. It aims to provide capabilities comparable to commercial radar
software — Level II radar decoding and rendering, multi-product analysis, and
model/forecast overlays — with its own branding, UI, assets, defaults, and
algorithms.

RadarPro treats the distinction between **observations** and **forecasts** as
a hard scientific rule: model output is never presented as observed radar,
and source metadata (units, timestamps, range folding, missing values) is
preserved end to end rather than smoothed away for looks.

## Status

**Stage S00 — Foundation.** This repository is not yet functional as a radar
application. This stage only establishes the monorepo layout, build
tooling, and CI checks required for subsequent stages to build on. There is
no radar decoding, rendering, or live data pipeline yet.

See `Agent Context/context/CURRENT_STAGE.md` for the authoritative current
stage and `Agent Context/context/stages/` for the staged roadmap.

## Repository layout

```
apps/
  radar-cli/        Command-line entry point for radar decoding/inspection
  web/               Development shell for the web-based workstation UI (Vite)
crates/
  radar-types/       Shared core types (units, time, geospatial, provider-neutral models)
  nexrad-level2/     NEXRAD Level II archive/stream decoding
fixtures/            Small, checked-in sample data for tests and local development
docs/
  adr/               Architecture Decision Records
Agent Context/        Full project/stage documentation for contributors and coding agents
```

Additional crates and apps will be introduced in later stages as needed;
S00 intentionally creates only the minimum required to have a healthy,
buildable workspace.

## Architecture at a glance

- **Rust** owns radar decoding, core meteorological models, geospatial math,
  and any performance-sensitive processing.
- **wgpu** is the preferred GPU abstraction for radar rendering.
- Mapping lives behind an adapter; radar rendering does not depend directly
  on any specific mapping or UI framework.
- Provider-specific formats and naming stop at provider boundaries — internal
  types are provider-neutral.
- Internal time is always UTC; forecasts track initialization time, lead
  time, and valid time separately from each other.

See `Agent Context/context/GLOBAL_CONTRACT.md` for the full set of rules that
apply across every development stage.

## Building

### Rust workspace

Requires the toolchain pinned in `rust-toolchain.toml` (stable, with
`rustfmt` and `clippy`).

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

### Web app (`apps/web`)

Requires a current Node.js LTS release (Node 20+).

```sh
cd apps/web
npm install
npm run typecheck
npm run build
```

Both are checked in CI on every push and pull request to `main` — see
`.github/workflows/ci.yml`.

## License

RadarPro is licensed under the **Apache License, Version 2.0** (see
[`LICENSE`](./LICENSE)). This license is **provisional** for the current
development stage: dependencies and third-party data-source terms will be
audited before a public release. See
`Agent Context/reference/RELEASE_AND_GOVERNANCE.md` for details.

## Documentation

Full project background, staged development context, architecture notes,
and reference material for contributors (human or AI) live under
[`Agent Context/`](./Agent%20Context/). Start with
`Agent Context/CLAUDE.md` and `Agent Context/context/CURRENT_STAGE.md`.
