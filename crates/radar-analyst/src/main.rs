// SPDX-License-Identifier: Apache-2.0
//! The summoned-reply daemon.
//!
//! Four lines, on purpose. Everything it does is in
//! [`daemon`](radar_analyst::daemon), where one poll can be driven by a test
//! against a fake platform rather than only by systemd against the real one.
//!
//! Its caller is `radar-analyst.service`. AGENTS.md section 5 asks for a
//! layer's caller to be named before it is built; a binary's is systemd, which
//! is why the loop lives in one rather than in a library function whose only
//! caller would be a test.

#![forbid(unsafe_code)]

fn main() {
    radar_analyst::daemon::run();
}
