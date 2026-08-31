// SPDX-License-Identifier: Apache-2.0
//! The adversarial test, run against the real process.
//!
//! The unit tests prove the rules. This proves the *binary* — the thing that
//! will actually hold the key — enforces them: same executable, same argument
//! parsing, same environment handling, same stdin loop.
//!
//! The distinction matters because every failure mode this process exists to
//! prevent lives in the wiring. A library that refuses correctly and a binary
//! that never calls it is a system with no signer at all, and the library's own
//! tests are green either way.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, Command, Stdio};

/// The programs the test signer will accept.
const DEX: [u8; 32] = [0x11; 32];
const MINT: [u8; 32] = [0x22; 32];
const SYSTEM: [u8; 32] = [0u8; 32];

/// The secret half of the test wallet. Deterministic; not a real key.
const SEED: [u8; 32] = [0x5A; 32];

/// A signer process with a pipe to it.
struct Signer {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl Signer {
    /// Starts the binary with a key file and an allowlist, and no customer key.
    fn start(key_file: &std::path::Path, programs: &str) -> Self {
        Self::start_with(key_file, programs, None)
    }

    /// Starts the binary, optionally with a Privy authorization key.
    fn start_with(key_file: &std::path::Path, programs: &str, privy: Option<&str>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_radar-signer"));
        command
            .env("RADAR_SIGNER_KEY", key_file)
            .env("RADAR_SIGNER_PROGRAMS", programs);
        // Removed rather than left unset, so a variable in the developer's own
        // environment cannot make the no-key test pass.
        match privy {
            Some(material) => command.env("RADAR_PRIVY_AUTHORIZATION_KEY", material),
            None => command.env_remove("RADAR_PRIVY_AUTHORIZATION_KEY"),
        };
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the signer binary must start");
        let stdout = child.stdout.take().expect("piped");
        Self {
            child,
            reader: BufReader::new(stdout),
        }
    }

    /// Sends one request and reads one answer.
    fn ask(&mut self, request: &serde_json::Value) -> serde_json::Value {
        let stdin = self.child.stdin.as_mut().expect("piped");
        writeln!(stdin, "{request}").expect("write");
        stdin.flush().expect("flush");

        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .expect("the signer must answer");
        serde_json::from_str(&line).expect("the answer must be JSON")
    }
}

impl Drop for Signer {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

/// Writes a Solana keypair file and returns its path.
fn key_file(dir: &std::path::Path) -> std::path::PathBuf {
    use ed25519_dalek::SigningKey;

    let signing = SigningKey::from_bytes(&SEED);
    let mut bytes = SEED.to_vec();
    bytes.extend_from_slice(&signing.verifying_key().to_bytes());

    let path = dir.join("signer.json");
    std::fs::write(&path, serde_json::to_string(&bytes).expect("serialises")).expect("write");
    path
}

/// The wallet the test signer signs for.
fn wallet() -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(&SEED)
        .verifying_key()
        .to_bytes()
}

fn b58(bytes: &[u8; 32]) -> String {
    radar_types::Address::new(*bytes).to_string()
}

/// Builds a transaction with one signature slot.
///
/// `accounts[0]` is the fee payer. Instructions are `(program_index, indices, data)`.
fn transaction(accounts: &[[u8; 32]], instructions: &[(u8, Vec<u8>, Vec<u8>)]) -> String {
    let mut out = vec![1u8];
    out.extend_from_slice(&[0u8; 64]);
    out.push(1);
    out.push(0);
    out.push(0);
    out.push(u8::try_from(accounts.len()).expect("small"));
    for a in accounts {
        out.extend_from_slice(a);
    }
    out.extend_from_slice(&[0xAA; 32]);
    out.push(u8::try_from(instructions.len()).expect("small"));
    for (program, indices, data) in instructions {
        out.push(*program);
        out.push(u8::try_from(indices.len()).expect("small"));
        out.extend_from_slice(indices);
        out.push(u8::try_from(data.len()).expect("small"));
        out.extend_from_slice(data);
    }
    radar_types::b64::encode(&out)
}

/// The honest transaction: a swap on the allowed DEX, in the authorised mint.
fn honest() -> String {
    transaction(
        &[wallet(), MINT, DEX, SYSTEM],
        &[(2, vec![0, 1], vec![0xAB, 0xCD])],
    )
}

fn request(transaction: &str, mint: &[u8; 32], now_slot: u64) -> serde_json::Value {
    serde_json::json!({
        "sign": "local",
        "authorization": {
            "nonce": "test-nonce",
            "mint": b58(mint),
            "action": "buy",
            "max_notional": 50_000_000u64,
            "expires_after": 1_150u64,
            "needs_operator_signature": false,
        },
        "transaction": transaction,
        "now_slot": now_slot,
    })
}

/// A scratch directory that cleans itself up.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("radar-signer-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_process_signs_an_honest_transaction() {
    let scratch = Scratch::new("honest");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );

    let answer = signer.ask(&request(&honest(), &MINT, 1_000));
    assert_eq!(answer["outcome"], "signed", "got {answer}");
    assert_eq!(answer["wallet"], b58(&wallet()));

    // The returned transaction must carry the signature in its first slot and
    // leave the message untouched. A signer that re-serialised the message
    // would be signing bytes it verified and returning bytes it did not.
    let returned =
        radar_types::b64::decode(answer["transaction"].as_str().expect("string")).expect("base64");
    let original = radar_types::b64::decode(&honest()).expect("base64");
    assert_eq!(returned.len(), original.len());
    assert_eq!(
        &returned[65..],
        &original[65..],
        "the message must be unchanged"
    );
    assert_ne!(&returned[1..65], &[0u8; 64], "a signature must be present");
}

#[test]
fn the_process_refuses_a_substituted_mint() {
    // The attack the separate process exists for: a compromised executor holds
    // a valid authorization for one token and builds a transaction for another.
    let scratch = Scratch::new("substituted");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );

    let other = transaction(
        &[wallet(), [0x99; 32], DEX, SYSTEM],
        &[(2, vec![0, 1], vec![0xAB])],
    );
    let answer = signer.ask(&request(&other, &MINT, 1_000));
    assert_eq!(answer["outcome"], "refused", "got {answer}");
    assert!(
        answer["reasons"]
            .as_array()
            .expect("array")
            .iter()
            .any(|r| r
                .as_str()
                .unwrap_or_default()
                .contains("is not in the transaction")),
        "got {answer}"
    );
}

#[test]
fn the_process_refuses_an_unlisted_program() {
    let scratch = Scratch::new("program");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );

    let evil = transaction(
        &[wallet(), MINT, [0xEE; 32], SYSTEM],
        &[(2, vec![0, 1], vec![0xAB])],
    );
    assert_eq!(
        signer.ask(&request(&evil, &MINT, 1_000))["outcome"],
        "refused"
    );
}

#[test]
fn the_process_refuses_an_oversized_spend() {
    let scratch = Scratch::new("oversize");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );

    let mut data = 2u32.to_le_bytes().to_vec();
    data.extend_from_slice(&60_000_000u64.to_le_bytes());
    let big = transaction(
        &[wallet(), MINT, DEX, SYSTEM],
        &[(2, vec![0, 1], vec![0xAB]), (3, vec![0, 1], data)],
    );
    let answer = signer.ask(&request(&big, &MINT, 1_000));
    assert_eq!(answer["outcome"], "refused", "got {answer}");
}

#[test]
fn the_process_refuses_an_expired_authorization() {
    let scratch = Scratch::new("expired");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );
    assert_eq!(
        signer.ask(&request(&honest(), &MINT, 99_999))["outcome"],
        "refused"
    );
}

#[test]
fn an_unconfigured_signer_refuses_rather_than_dying() {
    // A misconfiguration that looked like a crash would have the executor retry
    // forever. It must answer, and every answer must be a refusal.
    let scratch = Scratch::new("unconfigured");
    let missing = scratch.0.join("no-such-key.json");
    let mut signer = Signer::start(&missing, &b58(&DEX));

    for _ in 0..3 {
        let answer = signer.ask(&request(&honest(), &MINT, 1_000));
        assert_eq!(answer["outcome"], "refused", "got {answer}");
    }
}

#[test]
fn an_empty_allowlist_refuses_everything() {
    // A signer with no allowlist signs anything. Of every misconfiguration
    // available here, that is the one with no upper bound on its cost, so it
    // must not start into a permissive state.
    let scratch = Scratch::new("noallowlist");
    let mut signer = Signer::start(&key_file(&scratch.0), "");
    assert_eq!(
        signer.ask(&request(&honest(), &MINT, 1_000))["outcome"],
        "refused"
    );
}

#[test]
fn garbage_does_not_stop_the_process_serving() {
    // A malformed request must not become a denial of service against the only
    // component that can stop a bad trade.
    let scratch = Scratch::new("garbage");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );

    for junk in [
        serde_json::json!("not an object"),
        serde_json::json!({"sign": "local", "authorization": 5}),
        serde_json::json!({"sign": "local", "authorization": {}, "transaction": "!!!", "now_slot": 1}),
    ] {
        assert_eq!(signer.ask(&junk)["outcome"], "refused");
    }
    // Still serving, and still correct, afterwards.
    assert_eq!(
        signer.ask(&request(&honest(), &MINT, 1_000))["outcome"],
        "signed"
    );
}

/// A transaction in a token the authorisation does not cover.
fn substituted_mint() -> String {
    transaction(
        &[wallet(), [0x99; 32], DEX, SYSTEM],
        &[(2, vec![0, 1], vec![0xAB, 0xCD])],
    )
}

/// A base64 PKCS#8 P-256 key, as Privy's dashboard would hand one over.
fn privy_key() -> String {
    let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
        &ring::rand::SystemRandom::new(),
    )
    .expect("a key pair");
    radar_types::b64::encode(pkcs8.as_ref())
}

/// A request for a Privy authorization signature.
fn privy_request(transaction: &str) -> serde_json::Value {
    serde_json::json!({
        "sign": "privy",
        "authorization": {
            "nonce": "test-nonce",
            "mint": b58(&MINT),
            "action": "buy",
            "max_notional": 50_000_000u64,
            "expires_after": 1_150u64,
            "needs_operator_signature": false,
        },
        "request": {
            "method": "POST",
            "url": "https://api.privy.io/v1/wallets/abc/rpc",
            "body": {
                "method": "signTransaction",
                "params": {"transaction": transaction, "encoding": "base64"},
            },
            "headers": {"privy-app-id": "cmthhkznr0a3u0cl86prxlb7x"},
        },
        "wallet": b58(&wallet()),
        "now_slot": 1_000u64,
    })
}

#[test]
fn an_untagged_request_is_refused_rather_than_assumed_to_be_a_local_one() {
    // The version-skew property, at the process boundary where it would happen.
    //
    // The signer answers two kinds of request now, and the tag has no default
    // on purpose: a deployment that updates the executor and not the signer, or
    // the other way round, must stop signing rather than guess which kind of
    // signature was wanted. Refusing is recoverable; guessing is not.
    let scratch = Scratch::new("untagged");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );

    let mut untagged = request(&honest(), &MINT, 1_000);
    untagged
        .as_object_mut()
        .expect("an object")
        .remove("sign")
        .expect("the tag was there to remove");

    let answer = signer.ask(&untagged);
    assert_eq!(
        answer["outcome"], "refused",
        "an untagged request must be refused, not assumed: {answer}"
    );
}

#[test]
fn a_privy_request_with_no_authorization_key_configured_is_refused_by_name() {
    // Rule 8, and the wording matters as much as the refusal. An instance with
    // no customer key must say which thing is missing, or an operator goes to
    // Privy's dashboard looking for a fault in their account.
    let scratch = Scratch::new("no-privy-key");
    let mut signer = Signer::start(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
    );

    let answer = signer.ask(&privy_request(&honest()));
    assert_eq!(answer["outcome"], "refused", "{answer}");
    assert!(
        answer["reasons"][0]
            .as_str()
            .unwrap_or_default()
            .contains("no Privy authorization key"),
        "the refusal must name what is missing: {answer}"
    );
}

#[test]
fn a_blank_authorization_key_is_absent_rather_than_a_key() {
    // `RADAR_PRIVY_AUTHORIZATION_KEY=` left in an env file, which is what a
    // half-finished deployment looks like.
    //
    // Treating an empty value as key material means trying to parse it, which
    // fails, which makes `Config::from_env` return an error -- and an error
    // there refuses *everything*, taking the local signing lane down with it.
    // A blank value must mean the same as an absent one.
    let scratch = Scratch::new("blank-privy-key");
    let mut signer = Signer::start_with(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
        Some("   "),
    );

    // The local lane still works, which is the half that would break.
    let local = signer.ask(&request(&honest(), &MINT, 1_000));
    assert_eq!(
        local["outcome"], "signed",
        "a blank customer key must not disable local signing: {local}"
    );

    let customer = signer.ask(&privy_request(&honest()));
    assert!(
        customer["reasons"][0]
            .as_str()
            .unwrap_or_default()
            .contains("no Privy authorization key"),
        "a blank value must read as absent, by name: {customer}"
    );
}

#[test]
fn the_running_process_checks_a_privy_request_before_it_authorises_one() {
    // ADR 0007's claim is that the Privy key lives in *this process* behind
    // *this check*. A library test does not establish that the binary wired the
    // two together, and the wiring is the part that could be missing.
    //
    // Both directions in one test, deliberately: a gate that refuses everything
    // is not a gate, so the honest request must be authorised for the refusal
    // below to mean anything.
    let scratch = Scratch::new("privy-gate");
    let mut signer = Signer::start_with(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
        Some(&privy_key()),
    );

    let honest = signer.ask(&privy_request(&honest()));
    assert_eq!(
        honest["outcome"], "authorised",
        "the honest request must be authorised, or this test proves nothing: {honest}"
    );
    assert!(
        !honest["signature"].as_str().unwrap_or_default().is_empty(),
        "an authorised answer carries a header value: {honest}"
    );

    let substituted = signer.ask(&privy_request(&substituted_mint()));
    assert_eq!(
        substituted["outcome"], "refused",
        "a request for a token the authorisation does not cover must be refused: {substituted}"
    );
}

#[test]
fn a_privy_request_whose_body_carries_no_transaction_is_refused() {
    // The most tempting place in the whole lane to write "nothing to check,
    // carry on". A request whose contents cannot be read is one nothing
    // inspected, and signing it would authorise bytes nobody looked at.
    let scratch = Scratch::new("privy-no-transaction");
    let mut signer = Signer::start_with(
        &key_file(&scratch.0),
        &format!("{},{}", b58(&DEX), b58(&SYSTEM)),
        Some(&privy_key()),
    );

    let mut request = privy_request(&honest());
    request["request"]["body"]["params"] = serde_json::json!({});

    let answer = signer.ask(&request);
    assert_eq!(answer["outcome"], "refused", "{answer}");
}
