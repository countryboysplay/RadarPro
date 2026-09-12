// Minimal, hand-written test-page script -- no bundler, no npm, no build
// step beyond `wasm-bindgen` generating `./pkg/` (see ../README.md).
//
// Sequence: load the wasm module -> init a GPU device/surface for the
// canvas -> fetch the real KTLX fixture already used by radar-render's
// native harness -> decode it -> upload + render one frame. Every step
// writes a line into #status (not just the console), so a human can tell
// at a glance whether it worked, partially worked, or failed without
// opening devtools.

import init, { initGpu } from "./pkg/radar_web.js";

// Fixed CSS/backing-buffer size, matched 1:1 (no devicePixelRatio scaling)
// -- see README.md's "canvas sizing" note.
const CANVAS_SIZE = 512;

// crates/radar-web/www/index.js -> repo root is three directories up.
const FIXTURE_URL =
  "../../../fixtures/nexrad-level2/KTLX20240601_000353_V06";

const statusEl = document.getElementById("status");

function logStatus(message, kind) {
  const line = document.createElement("div");
  line.textContent = message;
  if (kind) line.className = kind;
  statusEl.appendChild(line);
  (kind === "err" ? console.error : console.log)(message);
}

async function main() {
  const canvas = document.getElementById("radar-canvas");
  canvas.width = CANVAS_SIZE;
  canvas.height = CANVAS_SIZE;

  logStatus("Loading wasm module...");
  await init();
  logStatus("wasm module loaded and instantiated.");

  logStatus("Requesting GPU adapter/device for <canvas>...");
  const renderer = await initGpu(canvas, CANVAS_SIZE, CANVAS_SIZE);
  logStatus(
    `GPU adapter acquired: ${renderer.adapterName} (backend: ${renderer.backend})`,
    "ok",
  );

  logStatus(`Fetching fixture: ${FIXTURE_URL}`);
  const response = await fetch(FIXTURE_URL);
  if (!response.ok) {
    throw new Error(
      `fixture fetch failed: HTTP ${response.status} ${response.statusText} ` +
        `(are you serving from the repo root? see README.md)`,
    );
  }
  const bytes = new Uint8Array(await response.arrayBuffer());
  logStatus(`Fetched ${bytes.length.toLocaleString()} bytes.`);

  logStatus("Decoding Archive II volume + selecting lowest-elevation REF sweep...");
  const info = renderer.decodeSweep(bytes);
  logStatus(
    `Decoded ${info.sweepCount} sweeps for site ${info.siteIcao}; ` +
      `rendering sweep at elevation ${info.elevationDeg.toFixed(2)} deg ` +
      `with ${info.radialCount} radials.`,
    "ok",
  );

  logStatus("Uploading GPU buffers/lookup texture/palette and rendering...");
  renderer.renderFrame();
  logStatus("Frame rendered and presented to the canvas.", "ok");
  logStatus("SUCCESS: end-to-end decode + render completed.", "ok");
}

main().catch((err) => {
  logStatus(`FAILED: ${err && err.message ? err.message : err}`, "err");
  console.error(err);
});
