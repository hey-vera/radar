// SPDX-License-Identifier: Apache-2.0
//! Test configuration, separate from the build config for the reason `web`'s
//! is: the tests need a DOM and the shipped bundle must not carry `jsdom`.

import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
