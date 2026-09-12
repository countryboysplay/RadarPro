#!/usr/bin/env node
// Builds this app's wasm32 dependencies -- `crates/radar-web` (S04/S05) and
// `crates/weather-alerts` (S06) -- and generates each one's `wasm-bindgen`
// JS/TS glue directly into `apps/web/src/wasm/`, where Vite's dev server and
// production build both pick it up as a plain ES module import (see
// `apps/web/README.md` "Wasm build pipeline" for the full explanation of
// this choice).
//
// This is a thin wrapper around the exact commands each crate's own README
// documents -- it does not reimplement or second-guess them, just runs them
// with paths resolved relative to this repo and fails loudly (non-zero
// exit, real stdout/stderr inherited) if any step fails, so
// `npm run dev`/`npm run build`/`npm run typecheck` (all of which depend on
// this via their `pre*` npm lifecycle hooks) never silently proceed against
// a stale or missing wasm module.
//
// Both crates' `wasm-bindgen --target web` output is written to the same
// `apps/web/src/wasm/` directory (distinguished by `--out-name`, so
// `radar_web.*` and `weather_alerts.*` sit side by side with no collision)
// rather than separate directories -- there is no meaningful benefit to
// splitting them, and every existing import path in `src/radar/wasmModule.ts`
// (`../wasm/radar_web.js`) keeps working unchanged. The existing
// `apps/web/src/wasm/`-wide `.gitignore` entry already covers this
// crate's output too.
//
// Prerequisites (see crates/radar-web/README.md, crates/weather-alerts/
// Cargo.toml, and this app's README.md):
//   rustup target add wasm32-unknown-unknown
//   cargo install wasm-bindgen-cli --version 0.2.128 --locked
// (0.2.128 must match the `wasm-bindgen` version pinned in both crates'
// Cargo.toml exactly -- a mismatch is a runtime error, not a compile error.)

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// apps/web/scripts -> apps/web -> apps -> repo root.
const repoRoot = path.resolve(__dirname, "..", "..", "..");
const outDir = path.resolve(__dirname, "..", "src", "wasm");

function run(command, args) {
  console.log(`[build-wasm] $ ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    shell: process.platform === "win32",
  });
  if (result.error) {
    console.error(`[build-wasm] failed to run '${command}': ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    console.error(`[build-wasm] '${command}' exited with status ${result.status}`);
    process.exit(result.status ?? 1);
  }
}

/**
 * Build one workspace crate to wasm32 and generate its `wasm-bindgen`
 * `--target web` glue into `outDir`.
 *
 * @param {string} crateName - Cargo package name (`-p` value), e.g. `"radar-web"`.
 * @param {string} artifactName - the crate's cdylib artifact stem (Cargo
 *   replaces `-` with `_`), e.g. `"radar_web"`. Also used as `--out-name`.
 */
function buildWasmCrate(crateName, artifactName) {
  const wasmArtifact = path.join(
    repoRoot,
    "target",
    "wasm32-unknown-unknown",
    "release",
    `${artifactName}.wasm`,
  );

  run("cargo", [
    "build",
    "--target",
    "wasm32-unknown-unknown",
    "-p",
    crateName,
    "--lib",
    "--release",
  ]);

  run("wasm-bindgen", [
    "--target",
    "web",
    "--out-dir",
    outDir,
    "--out-name",
    artifactName,
    wasmArtifact,
  ]);
}

buildWasmCrate("radar-web", "radar_web");
buildWasmCrate("weather-alerts", "weather_alerts");

console.log(`[build-wasm] wrote wasm-bindgen output for radar-web and weather-alerts to ${outDir}`);
