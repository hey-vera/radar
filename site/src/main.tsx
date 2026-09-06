// SPDX-License-Identifier: Apache-2.0
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
// The display face, before the stylesheet that names it. Five `@font-face`
// rules arrive, each gated by `unicode-range`; a reader of this English page
// fetches exactly one of them, the 29.4 kB latin subset. The other four are
// emitted and never requested.
import "@fontsource-variable/geist/wght.css";
import "./index.css";

const root = document.getElementById("root");
if (!root) throw new Error("no #root");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
