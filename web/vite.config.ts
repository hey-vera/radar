// SPDX-License-Identifier: Apache-2.0
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

/**
 * Puts `dist/.gitkeep` back after `emptyOutDir` removes it.
 *
 * `web/dist` is build output and `.gitignore` excludes its contents — but the
 * **directory** is tracked, through that one file, because `radar-serve` embeds
 * it with `rust-embed` and the derive generates no `get` method when the folder
 * does not exist. A checkout without it fails to compile, and the error names a
 * missing method rather than a missing directory.
 *
 * `emptyOutDir: true` deletes the whole directory including `.gitkeep`, so
 * anyone who runs a build and commits with `git add -A` stages that deletion.
 * That happened, and it turned five CI jobs red with an error pointing at
 * `embed.rs`.
 *
 * Recreating it here rather than in a `postbuild` script keeps it next to the
 * `emptyOutDir` that causes it, which is where somebody reading either one will
 * look.
 */
function keepTheDirectory(): Plugin {
  return {
    name: "radar-keep-dist-directory",
    closeBundle() {
      writeFileSync(join(import.meta.dirname, "dist", ".gitkeep"), "");
    },
  };
}

// The build output is embedded into the `radar-serve` binary, so a deploy stays
// one file and the box needs no Node. Assets are content-hashed, which is what
// lets them be cached hard while the entry HTML is not cached at all.
export default defineConfig({
  plugins: [react(), tailwindcss(), keepTheDirectory()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Fail rather than silently ship an unminified bundle.
    minify: "esbuild",
    sourcemap: false,
  },
  server: {
    // Pinned, and not vite's default 5173. A sibling product in the same
    // checkout tree runs its own dev server, and two projects racing for one
    // port is how you end up reading the wrong application's page and believing
    // it is yours.
    port: 5273,
    strictPort: true,
    // Local development talks to a real radar-serve rather than a mock, so the
    // shapes the page renders are the shapes it will get in production.
    proxy: {
      "/v1": "http://127.0.0.1:8402",
      "/health": "http://127.0.0.1:8402",
    },
  },
});
