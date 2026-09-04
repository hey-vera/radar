// SPDX-License-Identifier: Apache-2.0
//! One poll of the daemon's loop, against a platform that is not X.
//!
//! # What this establishes that the unit tests do not
//!
//! Every piece of this loop is tested where it lives: the parser, the gate, the
//! meter, the cursor arithmetic, the log's ordering. None of that says the
//! **loop** puts them in the right order, and the orderings are where the cost
//! is — a mention refused after the chain was read has already been paid for, a
//! reply posted before it was logged is a public statement with no record, and a
//! cursor advanced past a mention nobody answered is a question silently
//! dropped.
//!
//! So this runs `tick` exactly once against a fake platform and reads what
//! happened to the three files it owns.
//!
//! # It deliberately does not reach a chain
//!
//! The RPC client points at a closed port, so a mention naming a real mint comes
//! back `Unreadable` and is not published. That is the honest boundary of this
//! test: it proves the loop's plumbing and the money decisions, not the dossier,
//! which `radar-onchain` owns and tests against real captures.

use std::io::{Read as _, Write as _};
use std::time::Duration;

use radar_analyst::admission::{Gate, Limits};
use radar_analyst::daemon::{Paths, tick};
use radar_analyst::publish::DryRun;
use radar_analyst::spend::{Prices, Spend};
use radar_analyst::x::X;
use radar_provider::Budget;
use radar_types::MicroUsd;

/// The end of an HTTP header block.
const CRLF_CRLF: [u8; 4] = [13, 10, 13, 10];

/// A platform that answers one request with a canned page.
///
/// Returns its base URL and a receiver carrying the request it saw, so the test
/// can assert that the cursor and the credential actually reached the wire.
fn platform(body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("an address").port();
    let (tx, rx) = std::sync::mpsc::channel();
    let body = body.to_owned();

    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = vec![0_u8; 8192];
        let mut n = 0;
        while n < buf.len() {
            let Ok(read) = stream.read(&mut buf[n..]) else {
                break;
            };
            if read == 0 {
                break;
            }
            n += read;
            if buf[..n].windows(4).any(|w| w == CRLF_CRLF) {
                break;
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());

        let crlf = String::from_utf8(vec![13, 10]).expect("ascii is utf-8");
        let head = format!(
            "HTTP/1.1 200 OK{crlf}Content-Length: {len}{crlf}Content-Type: application/json{crlf}Connection: close{crlf}{crlf}",
            len = body.len()
        );
        let _ = stream.write_all(format!("{head}{body}").as_bytes());
        let _ = stream.flush();
    });

    (format!("http://127.0.0.1:{port}"), rx)
}

fn workspace(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("radar-loop-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp dir");
    dir.to_str().expect("a path").to_owned()
}

fn prices() -> Prices {
    Prices {
        mention_read: MicroUsd(1_000),
        post_read: MicroUsd(5_000),
        reply: MicroUsd(10_000),
        model_call: MicroUsd(2_000),
    }
}

fn open_limits() -> Limits {
    Limits {
        per_summoner_daily: 5,
        global_daily: 50,
        dedupe_seconds: 3_600,
    }
}

/// A client that cannot reach a chain, on purpose. See the module note.
fn no_chain() -> radar_onchain::RpcClient {
    radar_onchain::RpcClient::new("http://127.0.0.1:1".to_owned())
}

#[test]
fn one_poll_reads_answers_and_advances_the_cursor() {
    // Two mentions: one naming a symbol, which is answerable without a chain,
    // and one naming nothing. Neither reaches RPC, so what is being tested is
    // the loop rather than the dossier.
    let page = r#"{"data":[
        {"id":"1001","author_id":"alice","text":"@radar what about $ABC"},
        {"id":"1003","author_id":"bob","text":"@radar hello there"},
        {"id":"1002","author_id":"carol","text":"@radar and this one"}
    ]}"#;
    let (base, seen) = platform(page);

    let dir = workspace("advance");
    let paths = Paths::under(&dir);
    let mut gate = Gate::new(open_limits(), vec!["radar".to_owned()]);
    let mut spend = Spend::open(
        Budget {
            per_call_max: MicroUsd(50_000),
            daily_max: MicroUsd(1_000_000),
        },
        prices(),
        paths.ledger.clone(),
        1,
    );
    let x = X::at(base, "test-token", "u42");

    let answered = tick(
        Some(&x),
        &DryRun,
        &mut gate,
        &mut spend,
        &no_chain(),
        None,
        None,
        &paths,
    );

    // Nothing is published, because the publisher is the dry run.
    assert_eq!(answered, 0, "a dry run publishes nothing");

    let request = seen.recv().expect("the platform saw a request");
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer test-token"),
        "the credential must reach the wire: {request}"
    );

    // **The cursor is the largest id, not the last in the page.** The page is
    // deliberately out of order: 1001, 1003, 1002. Taking the last would set it
    // to 1002 and re-read 1003 forever.
    let cursor = radar_analyst::read_cursor(&paths.cursor);
    assert_eq!(
        cursor.as_deref(),
        Some("1003"),
        "the cursor must be the largest id seen"
    );

    // The read was charged even though nothing was answered: the page arrived,
    // and that is what a read is.
    assert_eq!(spend.spent_today(), MicroUsd(1_000));

    // The ledger is on disk, so a restart does not hand back a fresh allowance.
    assert!(
        std::path::Path::new(&paths.ledger).exists(),
        "the ledger must survive the tick"
    );
}

#[test]
fn a_platform_that_refuses_costs_nothing_and_does_not_move_the_cursor() {
    // The two properties that matter when the account's access breaks: no
    // charge for a page that never arrived, and no cursor movement past
    // mentions nobody has answered. Getting the second wrong loses the
    // questions permanently.
    let (base, _seen) = platform(r#"{"errors":[{"title":"Unauthorized"}]}"#);

    let dir = workspace("refused");
    let paths = Paths::under(&dir);
    let mut gate = Gate::new(open_limits(), vec!["radar".to_owned()]);
    let mut spend = Spend::open(
        Budget {
            per_call_max: MicroUsd(50_000),
            daily_max: MicroUsd(1_000_000),
        },
        prices(),
        paths.ledger.clone(),
        1,
    );
    let x = X::at(base, "stale-token", "u42");

    let answered = tick(
        Some(&x),
        &DryRun,
        &mut gate,
        &mut spend,
        &no_chain(),
        None,
        None,
        &paths,
    );

    assert_eq!(answered, 0);
    assert_eq!(
        spend.spent_today(),
        MicroUsd::ZERO,
        "a page that never arrived is not a read"
    );
    assert_eq!(
        radar_analyst::read_cursor(&paths.cursor),
        None,
        "a failed poll must not advance past unanswered mentions"
    );
}

#[test]
fn an_exhausted_budget_stops_the_poll_before_it_costs_anything() {
    // Rule 8's shape at the top of the loop: the read is authorised before it
    // happens, so an account out of budget does not poll at all rather than
    // polling and discovering it cannot answer.
    let (base, _seen) = platform(r#"{"data":[{"id":"1","author_id":"a","text":"@radar hi"}]}"#);

    let dir = workspace("broke");
    let paths = Paths::under(&dir);
    let mut gate = Gate::new(open_limits(), vec!["radar".to_owned()]);
    let mut spend = Spend::open(Budget::CLOSED, prices(), paths.ledger.clone(), 1);
    let x = X::at(base, "tok", "u42");

    let answered = tick(
        Some(&x),
        &DryRun,
        &mut gate,
        &mut spend,
        &no_chain(),
        None,
        None,
        &paths,
    );

    assert_eq!(answered, 0);
    assert_eq!(spend.refusals(), 1, "the read itself was refused");
    assert_eq!(
        radar_analyst::read_cursor(&paths.cursor),
        None,
        "a poll that never happened cannot move the cursor"
    );
}

#[test]
fn with_no_credential_the_loop_does_nothing_at_all() {
    // The resting state, and the one that ships: no source, no publisher, no
    // spend, no files. A daemon in this state is running and harmless.
    let dir = workspace("silent");
    let paths = Paths::under(&dir);
    let mut gate = Gate::new(open_limits(), vec!["radar".to_owned()]);
    let mut spend = Spend::open(
        Budget {
            per_call_max: MicroUsd(50_000),
            daily_max: MicroUsd(1_000_000),
        },
        prices(),
        paths.ledger.clone(),
        1,
    );

    let answered = tick(
        None,
        &DryRun,
        &mut gate,
        &mut spend,
        &no_chain(),
        None,
        None,
        &paths,
    );

    assert_eq!(answered, 0);
    assert_eq!(spend.spent_today(), MicroUsd::ZERO);
    assert!(!std::path::Path::new(&paths.log).exists());
    assert!(!std::path::Path::new(&paths.cursor).exists());
}
