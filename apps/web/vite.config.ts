import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// RadarPro web development shell — S00 Foundation stage.
// No feature configuration (map, workers, etc.) yet; kept intentionally minimal.
export default defineConfig({
  plugins: [react()],
});
