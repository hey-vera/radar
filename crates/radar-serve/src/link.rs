// SPDX-License-Identifier: Apache-2.0
//! Linking the model credential from the interface.
//!
//! Device authorisation prints a verification URL and a short code and waits for
//! a human to complete the flow in a browser. **Neither string is a
//! credential** — that is the whole design of the flow, and it is why they can
//! be rendered in a web page. The code is useless without somebody able to sign
//! in to the account, and it expires in minutes.
//!
//! So the button is not a shortcut around the boundary in
//! [ADR 0004](../../../docs/adr/0004-radar-spawns-the-vendor-cli-rather-than-holding-an-oauth-token.md):
//! the CLI still owns `auth.json`, still owns refresh, and Radar still has no
//! code that reads a token. What the button removes is an SSH session, which
//! matters most for the case nobody plans for — the credential lapsing after a
//! fortnight of inactivity, at which point re-linking should be a click rather
//! than a procedure somebody has to remember.
//!
//! # Three things this has to get right
//!
//! **Only the URL and the code leave.** The same subprocess goes on to print
//! success or failure, and on failure a vendor CLI will say more than it should.
//! [`radar_model::codex::parse_link`] extracts two fields and the route returns
//! those two fields.
//!
//! **One at a time, with a deadline.** The login process sits waiting for
//! minutes. A refreshed browser tab must not spawn a second one, and an
//! abandoned flow must not hold a process forever.
//!
//! **Behind the identity check.** A public button that spawns a login process on
//! somebody's server is a different thing entirely. [`crate::access`] is why
//! this could be built at all.

use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use radar_model::codex::Linking;
use serde::Serialize;
use serde_json::json;

use crate::AppState;

/// How long an unfinished flow is held before the process is reclaimed.
///
/// Generous, because the human in the middle may be finding their phone. Finite,
/// because a browser tab closed halfway through must not leave a process waiting
/// until the next restart.
pub const ABANDON_AFTER: core::time::Duration = core::time::Duration::from_secs(15 * 60);

/// Whether a flow of this age has been given up on.
///
/// A pure function rather than an inline comparison, because the comparison is
/// the whole reclaim and inlined it needs fifteen minutes to observe. Inverted,
/// it reclaims every *fresh* flow instead — killing the login the operator is
/// halfway through, and leaving the abandoned one in place forever.
#[must_use]
pub const fn is_abandoned(age: core::time::Duration) -> bool {
    age.as_nanos() >= ABANDON_AFTER.as_nanos()
}

/// A flow in progress.
#[derive(Debug)]
pub struct InFlight {
    child: std::process::Child,
    started: std::time::Instant,
    prompt: Linking,
}

/// The one login that may be running.
///
/// A `Mutex<Option<..>>` rather than a queue: two device-authorisation flows
/// against one credential is not a thing to support, it is a thing to refuse.
/// The credential is single-writer, and two flows would race to be the writer.
#[derive(Debug, Default)]
pub struct Linker {
    inner: Mutex<Option<InFlight>>,
}

/// What the interface is told.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Progress {
    /// A flow is open; show these to the operator.
    Waiting {
        /// Where to go.
        verification_url: String,
        /// What to type there.
        user_code: String,
        /// How long it has been open, so the page can say so.
        seconds_elapsed: u64,
    },
    /// The CLI finished and the credential is written.
    Linked,
    /// The CLI finished unsuccessfully.
    ///
    /// Carries the exit status and nothing else. A vendor CLI failing an
    /// authentication says more on stderr than belongs in a browser.
    Failed {
        /// What the process exited with.
        status: String,
    },
    /// Nothing is running.
    Idle,
}

impl Linker {
    /// An empty linker.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Whether a flow is already open, reclaiming an abandoned one first.
    ///
    /// Called before starting, so an abandoned flow does not block the next
    /// attempt forever — which would make the failure of this feature
    /// permanent rather than transient, and only fixable by a restart.
    fn reclaim_if_abandoned(slot: &mut Option<InFlight>) {
        let abandoned = slot
            .as_ref()
            .is_some_and(|f| is_abandoned(f.started.elapsed()));
        if abandoned && let Some(mut flight) = slot.take() {
            let _ = flight.child.kill();
            let _ = flight.child.wait();
        }
    }

    /// Where the current flow has got to.
    ///
    /// Reaps the process when it finishes, so a completed login does not stay
    /// reported as waiting.
    pub fn progress(&self) -> Progress {
        let Ok(mut slot) = self.inner.lock() else {
            return Progress::Failed {
                status: "the linker is poisoned".to_owned(),
            };
        };
        Self::reclaim_if_abandoned(&mut slot);

        let Some(flight) = slot.as_mut() else {
            return Progress::Idle;
        };

        match flight.child.try_wait() {
            Ok(Some(status)) => {
                let done = if status.success() {
                    Progress::Linked
                } else {
                    Progress::Failed {
                        status: status.to_string(),
                    }
                };
                *slot = None;
                done
            }
            Ok(None) => Progress::Waiting {
                verification_url: flight.prompt.verification_url.clone(),
                user_code: flight.prompt.user_code.clone(),
                seconds_elapsed: flight.started.elapsed().as_secs(),
            },
            Err(e) => {
                *slot = None;
                Progress::Failed {
                    status: format!("could not wait on the CLI: {e}"),
                }
            }
        }
    }
}

/// Starts a device-authorisation flow, or reports the one already running.
///
/// # Errors
///
/// Never returns `Err`; every failure is a status and a JSON body, because this
/// is answering a browser.
pub async fn begin(State(state): State<Arc<AppState>>) -> Response {
    let Some(chat) = state.chat.as_ref() else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response();
    };
    let Some(codex) = chat.linkable.as_ref() else {
        // The API-key path has nothing to link. Saying so beats a generic
        // failure, because the button should not have been shown at all.
        return (
            StatusCode::CONFLICT,
            Json(json!({ "error": "this provider is configured with a key, not a subscription" })),
        )
            .into_response();
    };

    // Asking again while one is open returns the same prompt rather than
    // starting a second. The credential is single-writer; two flows would race
    // to be the writer, and the loser corrupts the winner.
    if let Progress::Waiting { .. } = state.linker.progress() {
        return Json(state.linker.progress()).into_response();
    }

    let started = tokio::task::block_in_place(|| codex.begin_link());
    match started {
        Ok((child, prompt)) => {
            let Ok(mut slot) = state.linker.inner.lock() else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({ "error": "the linker is poisoned" })),
                )
                    .into_response();
            };
            *slot = Some(InFlight {
                child,
                started: std::time::Instant::now(),
                prompt: prompt.clone(),
            });
            Json(Progress::Waiting {
                verification_url: prompt.verification_url,
                user_code: prompt.user_code,
                seconds_elapsed: 0,
            })
            .into_response()
        }
        Err(why) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": why.to_string() })),
        )
            .into_response(),
    }
}

/// Reports the current flow.
pub async fn status(State(state): State<Arc<AppState>>) -> Response {
    if state.chat.is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response();
    }
    Json(state.linker.progress()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_linker_says_so_rather_than_pretending_to_wait() {
        // The interface renders a code when it is told `waiting`. A linker that
        // reported `waiting` with nothing running would show a stale code that
        // has already expired, and the operator would type it and be rejected.
        let linker = Linker::new();
        assert!(matches!(linker.progress(), Progress::Idle));
    }

    #[test]
    fn progress_serialises_with_the_state_the_interface_switches_on() {
        // The tag is the contract. A renamed variant would leave the page with a
        // shape it does not recognise, which renders as nothing at all.
        let waiting = serde_json::to_value(Progress::Waiting {
            verification_url: "https://auth.openai.com/device".to_owned(),
            user_code: "WDJB-MJHT".to_owned(),
            seconds_elapsed: 12,
        })
        .expect("serialises");
        assert_eq!(waiting["state"], "waiting");
        assert_eq!(waiting["user_code"], "WDJB-MJHT");

        assert_eq!(
            serde_json::to_value(Progress::Linked).expect("serialises")["state"],
            "linked"
        );
        assert_eq!(
            serde_json::to_value(Progress::Idle).expect("serialises")["state"],
            "idle"
        );
    }

    #[test]
    fn a_failure_carries_the_status_and_not_the_clis_complaint() {
        // A vendor CLI failing an authentication says more on stderr than
        // belongs in a browser, and this is the field a browser renders.
        let failed = serde_json::to_value(Progress::Failed {
            status: "exit status: 1".to_owned(),
        })
        .expect("serialises");
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["status"], "exit status: 1");
        assert_eq!(
            failed.as_object().map(serde_json::Map::len),
            Some(2),
            "two fields, so nothing rides along: {failed}"
        );
    }

    #[test]
    fn an_abandoned_flow_is_reclaimed_rather_than_blocking_the_next_one() {
        // Without this the first abandoned login makes the feature permanently
        // broken until a restart -- a transient failure turned into a permanent
        // one, which is the worse of the two.
        assert!(
            ABANDON_AFTER >= core::time::Duration::from_secs(300),
            "long enough for somebody to find their phone"
        );
        assert!(
            ABANDON_AFTER <= core::time::Duration::from_secs(3600),
            "short enough that an abandoned flow is not effectively forever"
        );

        // Inverted, this reclaims every *fresh* flow -- killing the login the
        // operator is halfway through -- and leaves the abandoned one in place
        // forever. Both directions asserted, and the boundary with them.
        assert!(!is_abandoned(core::time::Duration::ZERO));
        assert!(!is_abandoned(core::time::Duration::from_secs(60)));
        assert!(
            !is_abandoned(
                ABANDON_AFTER
                    .checked_sub(core::time::Duration::from_nanos(1))
                    .expect("the window is not zero")
            ),
            "a flow one nanosecond short of the window is still live"
        );
        assert!(is_abandoned(ABANDON_AFTER), "at the window, reclaim");
        assert!(is_abandoned(core::time::Duration::from_secs(86_400)));

        // With nothing in flight, reclaiming is a no-op rather than a panic.
        let mut empty = None;
        Linker::reclaim_if_abandoned(&mut empty);
        assert!(empty.is_none());
    }
}
