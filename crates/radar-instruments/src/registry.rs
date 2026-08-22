// SPDX-License-Identifier: Apache-2.0
//! The instrument registry: one declaration, four surfaces.

use std::collections::BTreeMap;

use radar_asof::AsOf;
use radar_store::Reader;
use radar_types::{MicroUsd, SourceId};
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::spec::Spec;

/// What an instrument is given besides its arguments.
///
/// The watermark is here rather than in each instrument's arguments so it cannot
/// be forgotten, and the store is behind a shared reference so an instrument
/// cannot write. An instrument that could record would be able to influence its
/// own replay.
pub struct Context<'a> {
    /// The point-in-time watermark. Nothing observed after this may inform the
    /// answer.
    pub as_of: AsOf,
    /// The recorded event log, read-only.
    pub store: &'a Reader,
}

/// Why an instrument could not answer.
#[derive(Debug, thiserror::Error)]
pub enum InstrumentError {
    /// The arguments did not match the declared input schema.
    #[error("invalid arguments for `{instrument}`: {detail}")]
    BadArguments {
        /// Which instrument.
        instrument: &'static str,
        /// What was wrong.
        detail: String,
    },
    /// No instrument by that name is registered.
    #[error("no instrument named `{0}`")]
    NotFound(String),
    /// The store could not answer as of the requested watermark.
    ///
    /// Distinct from an empty result: "I have no data for that slot" and
    /// "nothing happened in that slot" mean opposite things.
    #[error("store cannot answer as of {as_of}: {detail}")]
    OutOfRange {
        /// The watermark asked for.
        as_of: String,
        /// What the store said.
        detail: String,
    },
    /// Something else failed.
    #[error("{instrument}: {detail}")]
    Failed {
        /// Which instrument.
        instrument: &'static str,
        /// What happened.
        detail: String,
    },
}

/// An analytical instrument.
///
/// Implementors declare their contract once in [`Spec`]; the registry derives the
/// HTTP route, the x402 price, the MCP tool definition and the recorded schema
/// from that single declaration.
pub trait Instrument: Send + Sync {
    /// Arguments.
    type Input: DeserializeOwned + JsonSchema;
    /// Result.
    type Output: Serialize + JsonSchema;

    /// What this instrument declares about itself.
    fn spec(&self) -> Spec;

    /// Answers.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError`] if the question cannot be answered as asked.
    fn run(&self, input: Self::Input, ctx: &Context<'_>) -> Result<Self::Output, InstrumentError>;
}

/// A type-erased instrument, so the registry can hold instruments with different
/// input and output types in one map.
pub trait Erased: Send + Sync {
    /// The declaration.
    fn spec(&self) -> Spec;
    /// JSON Schema for the arguments, for MCP and for HTTP validation.
    fn input_schema(&self) -> Value;
    /// JSON Schema for the result.
    fn output_schema(&self) -> Value;
    /// Runs with JSON arguments.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError`] if the arguments do not deserialise or the
    /// instrument fails.
    fn call(&self, args: Value, ctx: &Context<'_>) -> Result<Value, InstrumentError>;
}

impl<T: Instrument> Erased for T {
    fn spec(&self) -> Spec {
        Instrument::spec(self)
    }

    fn input_schema(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(T::Input)).unwrap_or(Value::Null)
    }

    fn output_schema(&self) -> Value {
        serde_json::to_value(schemars::schema_for!(T::Output)).unwrap_or(Value::Null)
    }

    fn call(&self, args: Value, ctx: &Context<'_>) -> Result<Value, InstrumentError> {
        let name = Instrument::spec(self).name;
        let input: T::Input =
            serde_json::from_value(args).map_err(|e| InstrumentError::BadArguments {
                instrument: name,
                detail: e.to_string(),
            })?;
        let output = self.run(input, ctx)?;
        serde_json::to_value(output).map_err(|e| InstrumentError::Failed {
            instrument: name,
            detail: e.to_string(),
        })
    }
}

/// One recorded invocation.
///
/// Every call is recorded, and that recording *is* the research dataset. There
/// is no separate instrumentation step to fall behind, and no gap between what a
/// decision saw and what a replay reads.
#[derive(Clone, Debug, Serialize)]
pub struct Invocation {
    /// Which instrument.
    pub instrument: &'static str,
    /// Which version of its contract.
    pub version: String,
    /// The arguments, verbatim.
    pub arguments: Value,
    /// The watermark it answered as of.
    pub as_of: u64,
    /// The result, or `None` if it failed.
    pub output: Option<Value>,
    /// The error, if it failed.
    pub error: Option<String>,
    /// How long it took.
    pub latency_micros: u64,
    /// What it was expected to cost.
    pub declared_cost: MicroUsd,
    /// Which source answered.
    pub source: SourceId,
}

/// Instruments, by name.
#[derive(Default)]
pub struct Registry {
    by_name: BTreeMap<&'static str, Box<dyn Erased>>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an instrument.
    ///
    /// # Panics
    ///
    /// If an instrument with the same name is already registered. Two
    /// instruments answering to one name would make the HTTP route, the price
    /// and the MCP tool ambiguous, and which one answered would depend on
    /// registration order.
    pub fn register<T: Instrument + 'static>(&mut self, instrument: T) {
        let spec = Instrument::spec(&instrument);
        assert!(
            !self.by_name.contains_key(spec.name),
            "two instruments registered as `{}`",
            spec.name
        );
        self.by_name.insert(spec.name, Box::new(instrument));
    }

    /// Every instrument, in name order.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Erased> {
        self.by_name.values().map(AsRef::as_ref)
    }

    /// How many instruments are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// Looks one up.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn Erased> {
        self.by_name.get(name).map(AsRef::as_ref)
    }

    /// Calls an instrument and returns both its result and the record of the
    /// call.
    ///
    /// The record is produced whether or not the call succeeded. A failure is
    /// research data too — an instrument that fails on a class of token is
    /// telling you something about that class.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::NotFound`] if no instrument answers to `name`.
    /// A failure inside the instrument is reported in the returned
    /// [`Invocation`] rather than as an error, so the caller always gets a
    /// record.
    pub fn invoke(
        &self,
        name: &str,
        args: Value,
        ctx: &Context<'_>,
    ) -> Result<Invocation, InstrumentError> {
        let instrument = self
            .get(name)
            .ok_or_else(|| InstrumentError::NotFound(name.to_owned()))?;
        let spec = instrument.spec();
        let started = std::time::Instant::now();
        let result = instrument.call(args.clone(), ctx);
        let latency_micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);

        let (output, error) = match result {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e.to_string())),
        };

        Ok(Invocation {
            instrument: spec.name,
            version: spec.version.to_string(),
            arguments: args,
            as_of: ctx.as_of.slot().get(),
            output,
            error,
            latency_micros,
            declared_cost: spec.cost.total(),
            source: SourceId::local_decode(),
        })
    }

    /// The MCP tool catalogue, derived from the specs.
    ///
    /// The 2026-07-28 MCP specification is stateless and requires an `Mcp-Name`
    /// header on every call, so a paywall can price per tool from the header
    /// without deserialising the body — which is why the price belongs on the
    /// same declaration as the schema.
    #[must_use]
    pub fn mcp_tools(&self) -> Value {
        Value::Array(
            self.iter()
                .map(|i| {
                    let spec = i.spec();
                    serde_json::json!({
                        "name": spec.name,
                        "description": spec.summary,
                        "inputSchema": i.input_schema(),
                        "outputSchema": i.output_schema(),
                        "_meta": {
                            "org.heyvera.radar/version": spec.version.to_string(),
                            "org.heyvera.radar/latency": spec.latency,
                            "org.heyvera.radar/priceMicroUsd": spec.public_price(DEFAULT_MARGIN_PERCENT).get(),
                        }
                    })
                })
                .collect(),
        )
    }
}

/// Margin over cost on the public surface.
pub const DEFAULT_MARGIN_PERCENT: u64 = 50;
