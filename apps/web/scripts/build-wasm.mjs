#!/usr/bin/env node
// Builds `crates/radar-web`'s wasm32 module and generates its `wasm-bindgen`
// JS/TS glue directly into `apps/web/src/wasm/`, where Vite's dev server and
// production build both pick it up as a plain ES module import (see
// `apps/web/README.md` "Wasm build pipeline" for the full explanation of
// this choice).
//
// This is a thin wrapper around the exact two commands documented in
// `crates/radar-web/README.md` -- it does not reimplement or second-guess
// them, just runs them with paths resolved relative to this repo and fails
// loudly (non-zero exit, real stdout/stderr inherited) if either step
// fails, so `npm run dev`/`npm run build`/`npm run typecheck` (all of which
// depend on this via their `pre*` npm lifecycle hooks) never silently
// proceed against a stale or missing wasm module.
//
// Prerequisites (see crates/radar-web/README.md and this app's README.md):
//   rustup target add wasm32-unknown-unknown
//   cargo install wasm-bindgen-cli --version 0.2.128 --locked
// (0.2.128 must match the `wasm-bindgen` version pinned in
// crates/radar-web/Cargo.toml exactly -- a mismatch is a runtime error, not
// a compile error.)

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// apps/web/scripts -> apps/web -> apps -> repo root.
const repoRoot = path.resolve(__dirname, "..", "..", "..");
const outDir = path.resolve(__dirname, "..", "src", "wasm");
const wasmArtifact = path.join(
  repoRoot,
  "target",
  "wasm32-unknown-unknown",
  "release",
  "radar_web.wasm",
);

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

run("cargo", [
  "build",
  "--target",
  "wasm32-unknown-unknown",
  "-p",
  "radar-web",
  "--lib",
  "--release",
]);

run("wasm-bindgen", [
  "--target",
  "web",
  "--out-dir",
  outDir,
  "--out-name",
  "radar_web",
  wasmArtifact,
]);

console.log(`[build-wasm] wrote wasm-bindgen output to ${outDir}`);
