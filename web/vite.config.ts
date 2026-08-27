// SPDX-License-Identifier: Apache-2.0
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The build output is embedded into the `radar-serve` binary, so a deploy stays
// one file and the box needs no Node. Assets are content-hashed, which is what
// lets them be cached hard while the entry HTML is not cached at all.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Fail rather than silently ship an unminified bundle.
    minify: "esbuild",
    sourcemap: false,
  },
  server: {
    // Local development talks to a real radar-serve rather than a mock, so the
    // shapes the page renders are the shapes it will get in production.
    proxy: {
      "/v1": "http://127.0.0.1:8402",
      "/health": "http://127.0.0.1:8402",
    },
  },
});
