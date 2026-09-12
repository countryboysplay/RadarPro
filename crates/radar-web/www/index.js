// Minimal, hand-written test-page script -- no bundler, no npm, no build
// step beyond `wasm-bindgen` generating `./pkg/` (see ../README.md).
//
// Sequence: load the wasm module -> init a GPU device/surface for the
// canvas -> fetch the real KTLX fixture already used by radar-render's
// native harness -> decode it ONCE -> build elevation/moment pickers from
// the decoded volume's own metadata -> render whatever (elevation, moment)
// selection the pickers currently hold, re-rendering (never re-decoding)
// whenever either picker changes. Every step writes a line into #status
// (not just the console), so a human can tell at a glance whether it
// worked, partially worked, or failed without opening devtools.
//
// `window.__radarWebTest` exposes the renderer and a `select(sweepIndex,
// momentCode)` function directly, so an automated (e.g. CDP/puppeteer)
// check can drive elevation/moment switching without simulating real
// pointer events on the <select> elements.

import init, { initGpu } from "./pkg/radar_web.js";

// Fixed CSS/backing-buffer size, matched 1:1 (no devicePixelRatio scaling)
// -- see README.md's "canvas sizing" note.
const CANVAS_SIZE = 512;

// crates/radar-web/www/index.js -> repo root is three directories up.
const FIXTURE_URL =
  "../../../fixtures/nexrad-level2/KTLX20240601_000353_V06";

const statusEl = document.getElementById("status");
const elevationSelect = document.getElementById("elevation-select");
const momentSelect = document.getElementById("moment-select");

function logStatus(message, kind) {
  const line = document.createElement("div");
  line.textContent = message;
  if (kind) line.className = kind;
  statusEl.appendChild(line);
  (kind === "err" ? console.error : console.log)(message);
}

/** Populate `moment-select` with `renderer.momentWireCodesForSweep(sweepIndex)`,
 * preserving the previously-selected moment code if it is still offered on
 * this sweep, otherwise defaulting to the first one. */
function populateMomentsForSweep(renderer, sweepIndex) {
  const previous = momentSelect.value;
  const codes = renderer.momentWireCodesForSweep(sweepIndex);
  momentSelect.innerHTML = "";
  for (const code of codes) {
    const option = document.createElement("option");
    option.value = code;
    option.textContent = code;
    momentSelect.appendChild(option);
  }
  momentSelect.value = codes.includes(previous) ? previous : codes[0];
  momentSelect.disabled = false;
}

function currentSelection() {
  return {
    sweepIndex: Number(elevationSelect.value),
    momentCode: momentSelect.value,
  };
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

  logStatus("Decoding Archive II volume (once)...");
  const summary = renderer.decodeVolume(bytes);
  const elevationDegs = summary.elevationDegs();
  logStatus(
    `Decoded ${summary.sweepCount} sweeps for site ${summary.siteIcao}.`,
    "ok",
  );

  // Build the elevation picker from the volume's own metadata.
  elevationSelect.innerHTML = "";
  for (let i = 0; i < elevationDegs.length; i++) {
    const option = document.createElement("option");
    option.value = String(i);
    option.textContent = `${i}: ${elevationDegs[i].toFixed(2)} deg`;
    elevationSelect.appendChild(option);
  }
  // Default to the lowest-elevation sweep carrying REF, same convention
  // this crate's original S04 proof always used.
  const defaultSweepIndex = renderer.defaultSweepIndexForMoment("REF");
  elevationSelect.value = String(defaultSweepIndex ?? 0);
  elevationSelect.disabled = false;

  populateMomentsForSweep(renderer, Number(elevationSelect.value));

  function render() {
    const { sweepIndex, momentCode } = currentSelection();
    renderer.selectAndRender(sweepIndex, momentCode);
    const deg = elevationDegs[sweepIndex];
    logStatus(
      `Rendered sweep ${sweepIndex} (elevation ${deg.toFixed(2)} deg), moment ${momentCode}.`,
      "ok",
    );
  }

  elevationSelect.addEventListener("change", () => {
    populateMomentsForSweep(renderer, Number(elevationSelect.value));
    render();
  });
  momentSelect.addEventListener("change", render);

  render();
  logStatus("SUCCESS: end-to-end decode + initial render completed.", "ok");

  // Exposed for automated (CDP/puppeteer) verification of switching
  // elevation/moment without re-decoding -- see radar-web/README.md.
  window.__radarWebTest = {
    renderer,
    summary,
    elevationDegs,
    select(sweepIndex, momentCode) {
      elevationSelect.value = String(sweepIndex);
      populateMomentsForSweep(renderer, sweepIndex);
      if (momentCode) momentSelect.value = momentCode;
      render();
    },
  };
}

main().catch((err) => {
  logStatus(`FAILED: ${err && err.message ? err.message : err}`, "err");
  console.error(err);
});
