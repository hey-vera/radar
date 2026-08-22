// SPDX-License-Identifier: Apache-2.0
//! A stateless MCP server over the instrument registry.
//!
//! Implements the **2026-07-28** specification, which is stateless-first: there
//! are no protocol sessions, no `Mcp-Session-Id`, and no `initialize` handshake.
//! Every request carries its own protocol version and client capabilities in
//! `_meta`, and `server/discover` advertises what this server supports.
//!
//! Three properties of that revision matter commercially rather than
//! aesthetically, and together they are why Radar's paid surface is MCP:
//!
//! 1. **`Mcp-Name` is a required header.** A paywall can price a call from the
//!    header alone, without deserialising the body. The registry's price table
//!    becomes the middleware's configuration directly.
//! 2. **No sessions** means the surface sits behind a plain load balancer with
//!    no affinity, and a cold tool call is one round trip rather than three.
//! 3. **`ttlMs` and `cacheScope`** on list results map onto each instrument's
//!    declared freshness, so a shared intermediary may cache exactly what is
//!    safe to cache.
//!
//! Roots, Sampling and Logging are deprecated in this revision and are not
//! implemented.

use radar_asof::AsOf;
use radar_instruments::{Context, Registry};
use radar_store::Reader;
use serde_json::{Value, json};

/// The protocol revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2026-07-28";

/// How long a client may cache the tool list.
///
/// The catalogue changes only when Radar is redeployed, so this is generous. It
/// is `public` scope because the list carries no caller-specific content.
const TOOLS_TTL_MS: u64 = 300_000;

/// JSON-RPC error codes. The `-32020..=-32099` band is reserved for MCP by the
/// 2026-07-28 error-code allocation policy.
mod code {
    /// The request was not valid JSON-RPC.
    pub const INVALID_REQUEST: i32 = -32600;
    /// No such method.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// Arguments were wrong.
    pub const INVALID_PARAMS: i32 = -32602;
    /// The client asked for a protocol revision this server does not speak.
    pub const UNSUPPORTED_PROTOCOL_VERSION: i32 = -32022;
}

/// What the server needs to answer a call.
pub struct Server<'a> {
    /// The instruments on offer.
    pub registry: &'a Registry,
    /// The recorded store the instruments read.
    pub store: &'a Reader,
}

impl Server<'_> {
    /// Handles one JSON-RPC request and returns the response body.
    ///
    /// Notifications — requests with no `id` — return `None`, because JSON-RPC
    /// forbids replying to them.
    #[must_use]
    pub fn handle(&self, request: &Value) -> Option<Value> {
        // A notification -- a request with no id -- gets no reply even when it
        // is malformed. JSON-RPC forbids one, and answering anyway desynchronises
        // a client that is not expecting it.
        let id = Some(request.get("id")?.clone());
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();

        if let Some(version) = client_protocol_version(request)
            && version != PROTOCOL_VERSION
        {
            return Some(error(
                id,
                code::UNSUPPORTED_PROTOCOL_VERSION,
                &format!("this server speaks {PROTOCOL_VERSION}, not {version}"),
            ));
        }

        match method {
            "server/discover" => Some(ok(id, Self::discover())),
            "tools/list" => Some(ok(id, self.tools_list())),
            "tools/call" => Some(self.tools_call(id, request)),
            "" => Some(error(id, code::INVALID_REQUEST, "no method")),
            other => Some(error(
                id,
                code::METHOD_NOT_FOUND,
                &format!("no method `{other}`"),
            )),
        }
    }

    /// `server/discover` — required by the 2026-07-28 revision.
    fn discover() -> Value {
        json!({
            "resultType": "complete",
            "protocolVersions": [PROTOCOL_VERSION],
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": "radar",
                "title": "Radar — Solana research intelligence",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })
    }

    fn tools_list(&self) -> Value {
        json!({
            "resultType": "complete",
            "tools": self.registry.mcp_tools(),
            // Required on list results by this revision. Public because the
            // catalogue is identical for every caller.
            "ttlMs": TOOLS_TTL_MS,
            "cacheScope": "public",
        })
    }

    fn tools_call(&self, id: Option<Value>, request: &Value) -> Value {
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return error(id, code::INVALID_PARAMS, "params.name is required");
        };
        if self.registry.get(name).is_none() {
            return error(id, code::INVALID_PARAMS, &format!("no tool named `{name}`"));
        }

        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        // The watermark is the store's own, not the caller's. Letting a caller
        // choose it would let them ask what Radar knew before it knew it, which
        // is the point-in-time guarantee working in reverse.
        let watermark = match Reader::watermark(self.store) {
            Ok(Some(slot)) => slot,
            Ok(None) => {
                return error(
                    id,
                    code::INVALID_PARAMS,
                    "the store is empty and cannot answer",
                );
            }
            Err(e) => return error(id, code::INVALID_PARAMS, &e.to_string()),
        };

        let ctx = Context {
            as_of: AsOf::at(watermark),
            store: self.store,
        };
        match self.registry.invoke(name, arguments, &ctx) {
            Ok(record) => {
                let failed = record.error.is_some();
                let payload = record
                    .output
                    .clone()
                    .unwrap_or_else(|| json!({ "error": record.error }));
                ok(
                    id,
                    json!({
                        "resultType": "complete",
                        // Text first: some clients render only this.
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&payload)
                                .unwrap_or_else(|_| payload.to_string()),
                        }],
                        "structuredContent": payload,
                        "isError": failed,
                        "_meta": {
                            "org.heyvera.radar/asOfSlot": record.as_of,
                            "org.heyvera.radar/instrumentVersion": record.version,
                            "org.heyvera.radar/latencyMicros": record.latency_micros,
                        },
                    }),
                )
            }
            Err(e) => error(id, code::INVALID_PARAMS, &e.to_string()),
        }
    }
}

/// The protocol version a client declared, if it declared one.
fn client_protocol_version(request: &Value) -> Option<&str> {
    request
        .get("params")?
        .get("_meta")?
        .get("io.modelcontextprotocol/protocolVersion")?
        .as_str()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "both arguments are moved into the JSON value the macro builds"
)]
fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "the id is moved into the JSON value the macro builds"
)]
fn error(id: Option<Value>, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_instruments::CreatorHistory;

    fn fixture() -> (tempfile::TempDir, Registry) {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut registry = Registry::new();
        registry.register(CreatorHistory);
        (dir, registry)
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "params is moved into the JSON value the macro builds"
    )]
    fn request(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    #[test]
    fn discover_advertises_the_revision_this_server_speaks() {
        let (dir, registry) = fixture();
        let store = Reader::open(dir.path());
        let server = Server {
            registry: &registry,
            store: &store,
        };

        let out = server
            .handle(&request("server/discover", json!({})))
            .expect("a reply");
        assert_eq!(out["result"]["protocolVersions"][0], PROTOCOL_VERSION);
        assert_eq!(out["result"]["resultType"], "complete");
        assert_eq!(out["result"]["serverInfo"]["name"], "radar");
    }

    #[test]
    fn the_tool_list_carries_the_cache_hints_this_revision_requires() {
        let (dir, registry) = fixture();
        let store = Reader::open(dir.path());
        let server = Server {
            registry: &registry,
            store: &store,
        };

        let out = server
            .handle(&request("tools/list", json!({})))
            .expect("a reply");
        assert_eq!(out["result"]["ttlMs"], TOOLS_TTL_MS);
        assert_eq!(out["result"]["cacheScope"], "public");
        assert_eq!(out["result"]["tools"][0]["name"], "creator_history");
    }

    #[test]
    fn a_notification_gets_no_reply() {
        // JSON-RPC forbids replying to a request with no id. Replying anyway
        // desynchronises a client that is not expecting one.
        let (dir, registry) = fixture();
        let store = Reader::open(dir.path());
        let server = Server {
            registry: &registry,
            store: &store,
        };
        let notification = json!({ "jsonrpc": "2.0", "method": "tools/list" });
        assert!(server.handle(&notification).is_none());
    }

    #[test]
    fn a_client_on_another_revision_is_told_so_rather_than_guessed_at() {
        let (dir, registry) = fixture();
        let store = Reader::open(dir.path());
        let server = Server {
            registry: &registry,
            store: &store,
        };

        let req = request(
            "tools/list",
            json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "2025-03-26" } }),
        );
        let out = server.handle(&req).expect("a reply");
        assert_eq!(out["error"]["code"], code::UNSUPPORTED_PROTOCOL_VERSION);
    }

    #[test]
    fn an_unknown_method_is_an_error_not_a_silent_empty_result() {
        let (dir, registry) = fixture();
        let store = Reader::open(dir.path());
        let server = Server {
            registry: &registry,
            store: &store,
        };
        let out = server
            .handle(&request("resources/list", json!({})))
            .expect("a reply");
        assert_eq!(out["error"]["code"], code::METHOD_NOT_FOUND);
    }

    #[test]
    fn an_unknown_tool_is_refused_before_the_store_is_touched() {
        let (dir, registry) = fixture();
        let store = Reader::open(dir.path());
        let server = Server {
            registry: &registry,
            store: &store,
        };
        let out = server
            .handle(&request("tools/call", json!({ "name": "no_such_tool" })))
            .expect("a reply");
        assert_eq!(out["error"]["code"], code::INVALID_PARAMS);
        assert!(
            out["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("no_such_tool"))
        );
    }

    #[test]
    fn an_empty_store_says_so_rather_than_answering_zero() {
        // "I have no data" and "the answer is nothing" mean opposite things to a
        // caller deciding whether to trust a result.
        let (dir, registry) = fixture();
        let store = Reader::open(dir.path());
        let server = Server {
            registry: &registry,
            store: &store,
        };
        let out = server
            .handle(&request(
                "tools/call",
                json!({ "name": "creator_history", "arguments": { "creator": "x" } }),
            ))
            .expect("a reply");
        assert!(
            out["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("empty"))
        );
    }
}
