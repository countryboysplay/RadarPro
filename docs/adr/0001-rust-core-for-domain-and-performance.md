# ADR-0001: Rust for radar decoding, domain models, and performance-sensitive processing

Status: Accepted

## Context
RadarPro decodes binary NEXRAD Level II (Archive II / Message 31) data, maintains polar radar geometry (`Volume -> Sweep -> Radial -> Moment`), performs geospatial math, and must feed a GPU renderer at interactive frame rates. This data is untrusted (arrives over the network from public and commercial providers) and is high-volume (a single volume scan can contain millions of gates across many radials and sweeps). `GLOBAL_CONTRACT.md` requires that remote/binary input never cause uncontrolled panics, that the UI thread never synchronously decode complete volumes, and that missing values, range folding, units, and timestamps be preserved exactly — none of which tolerate an interpreter's GC pauses, weak type safety around binary layout, or ad-hoc error handling.

The project also targets multiple delivery surfaces long-term (web via `apps/web`, desktop via `apps/desktop`/Tauri, and a CLI). A single core language for decoding and domain modeling avoids re-implementing meteorologically sensitive logic once per platform.

## Decision
Rust owns radar decoding (`nexrad-level2`), core meteorological/domain models (`radar-types`, later `forecast-core`), geospatial math (`radar-geo`), and all other performance-sensitive or correctness-sensitive processing. Higher-level UI/application shells (web via TypeScript/React, desktop via Tauri, CLI via `radar-cli`) call into this Rust core rather than re-implementing domain logic.

## Alternatives
- **TypeScript/Node for the full stack**: fastest to prototype and keeps one language across `apps/web`, but weak binary parsing ergonomics, no compile-time memory/aliasing guarantees for high-throughput decode loops, and GC pauses are a poor fit for a UI thread that must stay responsive during decode.
- **C/C++ core**: comparable raw performance to Rust and mature geospatial/GPU ecosystems, but manual memory management is a poor fit for parsing untrusted, malformed binary input without uncontrolled crashes (a `GLOBAL_CONTRACT.md` requirement), and it lacks Rust's `Result`-based error handling and package ecosystem (`cargo`).
- **Rust compiled to WebAssembly only where needed, TypeScript everywhere else**: rejected as the primary boundary (rather than as an implementation detail of the web target) because it would fragment where domain logic lives across platforms and encourage duplicating decode/model logic in TypeScript for the desktop/CLI paths that don't need Wasm at all.

## Consequences
**Benefits**: memory safety without a garbage collector; `Result`/`?` encourages explicit handling of malformed input instead of panics; one implementation of decoding and domain rules shared by web (via Wasm or a service boundary), desktop, and CLI; strong alignment with `wgpu` (ADR-0002) for the render path; a real type system to encode invariants like "observations and forecasts are distinguishable" and "display units never mutate source data."

**Costs/risks**: steeper contributor onboarding than TypeScript-only; slower iteration on UI-adjacent logic than a scripting language; requires a defined boundary (FFI, Wasm bindings, or an IPC/service layer) between Rust core and the web/desktop shells, which is nontrivial engineering surface.

**Portability**: Rust's cross-compilation and `wasm32` target support keep the same core deployable to web (Wasm), desktop (native via Tauri), and CLI (native) without three separate implementations.

**Scientific implications**: keeping decode and unit/time handling in one strongly-typed core reduces the chance that a platform-specific reimplementation silently drifts from `GLOBAL_CONTRACT.md` rules (missing-value preservation, UTC internal time, no invented parsing behavior).

## Validation
No decoding exists yet (S00 scope explicitly excludes real Archive II parsing). Validation for this ADR at S00 is structural: the workspace (`Cargo.toml`) builds `crates/radar-types`, `crates/nexrad-level2`, and `apps/radar-cli` under `cargo build`, and `cargo fmt --check` / `cargo clippy --all-targets -- -D warnings` / `cargo test` pass, demonstrating the toolchain and workspace shape are viable. Real validation — decode correctness against known Archive II fixtures, and panic-freedom on fuzzed/malformed input — is deferred to S01 per `fixtures/README.md`.
