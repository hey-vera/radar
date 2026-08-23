// SPDX-License-Identifier: Apache-2.0
//! Instruments: one declaration, four surfaces.
//!
//! An instrument declares itself once — name, version, typed input and output,
//! cost model, latency class, freshness, determinism — and every surface is
//! derived from that single [`Spec`]:
//!
//! 1. the **internal call**, which strategies and the risk kernel use;
//! 2. the **HTTP endpoint**, routed by name;
//! 3. the **x402 price**, computed from the cost model rather than configured
//!    beside it;
//! 4. the **MCP tool**, whose schema and description are the same ones.
//!
//! Three surfaces each carrying their own copy of the price and the schema would
//! drift, and the one that drifted would be the paid one.
//!
//! Everything falls out of one more property: **every invocation is recorded**,
//! with arguments, result, watermark, latency and declared cost. That recording
//! *is* the research dataset — there is no separate instrumentation step to fall
//! behind, and no gap between what a decision saw and what a replay reads.
//!
//! ```
//! use radar_instruments::{Registry, CreatorHistory};
//!
//! let mut registry = Registry::new();
//! registry.register(CreatorHistory);
//!
//! // The MCP catalogue, the HTTP routes and the price list are all this one map.
//! let tools = registry.mcp_tools();
//! assert_eq!(tools.as_array().map(Vec::len), Some(1));
//! assert!(registry.get("creator_history").is_some());
//! ```

#![forbid(unsafe_code)]

mod creator_history;
mod creator_track_record;
mod registry;
pub mod simulate_exit;
mod spec;

pub use creator_history::CreatorHistory;
pub use creator_track_record::CreatorTrackRecord;
pub use registry::{
    Context, DEFAULT_MARGIN_PERCENT, Erased, Instrument, InstrumentError, Invocation, Registry,
};
pub use simulate_exit::SimulateExit;
pub use spec::{Cost, Determinism, Latency, MIN_PUBLIC_PRICE, Spec, Version};
