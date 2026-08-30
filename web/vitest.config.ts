// SPDX-License-Identifier: Apache-2.0
//! Test configuration, deliberately separate from the build config.
//!
//! The build has no test dependencies in it and the tests need a DOM. Merging
//! the two would put `jsdom` in the dependency graph of the bundle that gets
//! embedded in `radar-serve`, which is the one place in this repository where a
//! frontend dependency becomes a Rust binary's problem.

import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
