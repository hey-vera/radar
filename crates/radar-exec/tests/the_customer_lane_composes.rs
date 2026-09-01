// SPDX-License-Identifier: Apache-2.0
//! The customer lane, end to end, with the key in another process.
//!
//! `lane_composes.rs` does this for the local wallet. This does it for a
//! customer's, where the shape is different in the way that matters: **no
//! process in Radar can produce a signature on its own.**
//!
//! Three parties, each holding one thing. The executor holds the application
//! credential and can ask. `radar-signer` holds the P-256 authorization key and
//! can authorise one request it has checked. Privy holds the wallet key and
//! signs what a valid authorisation asks for. That is ADR 0007 and ADR 0008
//! working together, and what these tests are for is confirming the pieces were
//! actually wired that way rather than merely written that way.
//!
//! The signer here is the **real** `verify::check`, not a stub. Stubbing it
//! would make every test below a statement about the fixture.

use std::sync::Mutex;

use radar_customer::{Allowance, Meter, Subject};
use radar_exec::customer_signing::{Authorising, CustomerSigner, PrivyTransport};
use radar_exec::pipeline::Signing;
use radar_risk::{Action, Address, Authorization, Autonomy, MicroUsd, Policy, Slot};
use radar_signer::privy::{AuthorizationKey, PrivyRequest, authorise};
use radar_signer::verify::Allowlist;

const SYSTEM: [u8; 32] = [0u8; 32];
const DEX: [u8; 32] = [0x11; 32];
const MINT: [u8; 32] = [0x22; 32];
const WALLET: [u8; 32] = [0x33; 32];
const APP: &str = "cmthhkznr0a3u0cl86prxlb7x";
const TODAY: u64 = 20_331;

fn wallet_address() -> String {
    Address::new(WALLET).to_string()
}

fn policy() -> Policy {
    Policy {
        autonomy: Autonomy::Capped,
        max_position: MicroUsd(50_000_000),
        max_canary: MicroUsd(50_000_000),
        max_input_staleness: radar_risk::SlotDelta(1_000),
        ..Policy::CLOSED
    }
}

fn authorization() -> Authorization {
    Authorization {
        nonce: "content-hash".to_owned(),
        mint: Address::new(MINT),
        action: Action::Buy,
        max_notional: MicroUsd(6_210_000),
        expires_after: Slot(1_150),
        needs_operator_signature: false,
    }
}

/// A legacy transaction whose fee payer is the customer's wallet.
fn transaction(accounts: &[[u8; 32]]) -> String {
    let mut out = vec![0u8, 1, 0, 0];
    out.push(u8::try_from(accounts.len()).expect("small"));
    for a in accounts {
        out.extend_from_slice(a);
    }
    out.extend_from_slice(&[0xAA; 32]);
    out.push(1);
    out.extend_from_slice(&[2, 2, 0, 1, 2, 0xAB, 0xCD]);
    radar_types::b64::encode(&out)
}

fn honest() -> String {
    transaction(&[WALLET, MINT, DEX, SYSTEM])
}

fn substituted_mint() -> String {
    transaction(&[WALLET, [0x99; 32], DEX, SYSTEM])
}

/// The real signer process's logic, in-process.
///
/// Not a stub of it. The point of these tests is the composition, and a stubbed
/// signer would make them statements about the stub.
struct RealSigner {
    key: AuthorizationKey,
    policy: Policy,
}

impl RealSigner {
    fn new(policy: Policy) -> Self {
        let pkcs8 = ring::signature::EcdsaKeyPair::generate_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING,
            &ring::rand::SystemRandom::new(),
        )
        .expect("a key pair");
        Self {
            key: AuthorizationKey::parse(&radar_types::b64::encode(pkcs8.as_ref()))
                .expect("parses"),
            policy,
        }
    }
}

impl Authorising for RealSigner {
    fn authorise(
        &self,
        authorization: &Authorization,
        request: &serde_json::Value,
        wallet: &str,
    ) -> Result<String, Vec<String>> {
        let request: PrivyRequest =
            serde_json::from_value(request.clone()).map_err(|e| vec![e.to_string()])?;
        let wallet: Address = wallet.parse().map_err(|_| vec!["bad wallet".to_owned()])?;
        authorise(
            &self.key,
            &request,
            authorization,
            &wallet,
            &Allowlist {
                programs: vec![DEX, SYSTEM],
            },
            &self.policy,
            Slot(1_000),
        )
        .map_err(|why| vec![why.to_string()])
    }
}

/// Privy, recording what it was actually sent.
#[derive(Default)]
struct FakePrivy {
    seen: Mutex<Vec<(String, String)>>,
}

impl PrivyTransport for FakePrivy {
    fn post(&self, url: &str, body: &str, signature: &str) -> Result<String, String> {
        self.seen
            .lock()
            .expect("not poisoned")
            .push((body.to_owned(), signature.to_owned()));
        assert!(
            url.contains("/wallets/"),
            "the wallet-scoped endpoint: {url}"
        );
        Ok(r#"{"method":"signTransaction","data":{"signed_transaction":"c2lnbmVk","encoding":"base64"}}"#.to_owned())
    }
}

fn meter(allowance: u32) -> Mutex<Meter> {
    let subject = Subject::derive("did:privy:someone", &[0xAB; 32]).expect("a subject");
    Mutex::new(Meter::new(subject, Allowance::per_day(allowance), TODAY))
}

#[test]
fn a_customer_trade_the_kernel_authorised_reaches_privy_signed() {
    // The permitting half. A lane that refuses everything is not a lane, and
    // every refusal below would prove nothing without this.
    let signer = RealSigner::new(policy());
    let privy = FakePrivy::default();
    let meter = meter(10);
    let customer = CustomerSigner::new("sol-1", wallet_address(), APP, &signer, &privy, &meter);

    let returned = customer
        .sign(&authorization(), &honest())
        .expect("an authorised trade signs");
    assert_eq!(returned, "c2lnbmVk");

    let seen = privy.seen.lock().expect("not poisoned");
    assert_eq!(seen.len(), 1, "exactly one request reached Privy");
    assert!(
        seen[0].0.contains("signTransaction"),
        "the body Privy received is the signing request: {}",
        seen[0].0
    );
    assert!(
        !seen[0].1.is_empty(),
        "and it carried an authorization signature"
    );
}

#[test]
fn a_trade_for_another_token_never_reaches_privy_at_all() {
    // The order is the property. The signer refuses before anything is sent, so
    // a substituted mint costs no network call and no vendor-side rejection --
    // it stops inside Radar, at the process holding the key.
    let signer = RealSigner::new(policy());
    let privy = FakePrivy::default();
    let meter = meter(10);
    let customer = CustomerSigner::new("sol-1", wallet_address(), APP, &signer, &privy, &meter);

    let refusal = customer
        .sign(&authorization(), &substituted_mint())
        .expect_err("a substituted mint must not be signed");
    assert!(
        refusal
            .iter()
            .any(|r| r.contains("outside the authorisation")),
        "the signer's reason should survive to the caller: {refusal:?}"
    );
    assert!(
        privy.seen.lock().expect("not poisoned").is_empty(),
        "nothing should have been sent to Privy"
    );
}

#[test]
fn a_closed_policy_stops_the_customer_lane_at_the_signer() {
    // `Policy::CLOSED` enforced at the key, on a customer's wallet.
    //
    // Everything else here is identical to the passing case: the same
    // authorisation, the same transaction, the same wallet. The only difference
    // is the policy file the signer process loaded, which no caller controls.
    let signer = RealSigner::new(Policy::CLOSED);
    let privy = FakePrivy::default();
    let meter = meter(10);
    let customer = CustomerSigner::new("sol-1", wallet_address(), APP, &signer, &privy, &meter);

    assert!(
        customer.sign(&authorization(), &honest()).is_err(),
        "a closed policy must not sign a customer's wallet"
    );
    assert!(
        privy.seen.lock().expect("not poisoned").is_empty(),
        "and must not reach Privy"
    );
}

#[test]
fn the_body_privy_receives_is_the_body_the_signer_authorised() {
    // The property the whole scheme rests on, and the one that could break
    // silently.
    //
    // The signature covers a canonicalisation of the request. If the body were
    // rebuilt between signing and sending -- a re-serialisation, a re-ordered
    // key, an added field -- the signature would no longer match, and the
    // failure would arrive from Privy as an authentication error that looks like
    // a credential problem.
    //
    // Asserted by checking the transaction survives intact, since that is the
    // field a rebuild would most plausibly disturb.
    let signer = RealSigner::new(policy());
    let privy = FakePrivy::default();
    let meter = meter(10);
    let customer = CustomerSigner::new("sol-1", wallet_address(), APP, &signer, &privy, &meter);

    customer.sign(&authorization(), &honest()).expect("signs");

    let seen = privy.seen.lock().expect("not poisoned");
    assert!(
        seen[0].0.contains(&honest()),
        "the exact transaction must reach Privy: {}",
        seen[0].0
    );
}

#[test]
fn a_spent_allowance_refuses_before_anything_is_signed() {
    // The meter is a cap, not only a count. An unbounded signer is what
    // invariant 1 exists to prevent, and a runaway loop should cost a bounded
    // number of signatures rather than a bill.
    let signer = RealSigner::new(policy());
    let privy = FakePrivy::default();
    let meter = meter(2);
    let customer = CustomerSigner::new("sol-1", wallet_address(), APP, &signer, &privy, &meter);

    assert!(customer.sign(&authorization(), &honest()).is_ok());
    assert!(customer.sign(&authorization(), &honest()).is_ok());
    assert!(
        customer.sign(&authorization(), &honest()).is_err(),
        "the third is past the allowance"
    );
    assert_eq!(
        privy.seen.lock().expect("not poisoned").len(),
        2,
        "the refused attempt must not have reached Privy"
    );
}

#[test]
fn a_refused_signature_still_costs_its_allowance() {
    // Deliberate, and the direction is worth stating because it looks unfair.
    //
    // The meter is charged before the signature is asked for. A process that
    // dies mid-call cannot know whether Privy signed, and counting afterwards
    // undercounts exactly the calls that went wrong -- which is what a runaway
    // loop is made of. Paying for a signature that never happened is the
    // recoverable direction; the other one is a bill.
    let signer = RealSigner::new(policy());
    let privy = FakePrivy::default();
    let meter = meter(10);
    let customer = CustomerSigner::new("sol-1", wallet_address(), APP, &signer, &privy, &meter);

    assert!(
        customer
            .sign(&authorization(), &substituted_mint())
            .is_err()
    );
    assert_eq!(
        meter.lock().expect("not poisoned").today(),
        1,
        "the attempt was charged even though nothing was signed"
    );
}

#[test]
fn privy_answering_without_a_signed_transaction_is_a_failure_not_a_pass() {
    // Privy reports errors in the same envelope as successes. An answer with no
    // `signed_transaction` must never be read as one, because the value that
    // would flow onward is whatever the caller defaulted to.
    struct Unhelpful;
    impl PrivyTransport for Unhelpful {
        fn post(&self, _: &str, _: &str, _: &str) -> Result<String, String> {
            Ok(r#"{"error":"wallet not found","data":{}}"#.to_owned())
        }
    }

    let signer = RealSigner::new(policy());
    let meter = meter(10);
    let unhelpful = Unhelpful;
    let customer = CustomerSigner::new("sol-1", wallet_address(), APP, &signer, &unhelpful, &meter);

    let refusal = customer
        .sign(&authorization(), &honest())
        .expect_err("an answer with no signature is not a signature");
    assert!(
        refusal.iter().any(|r| r.contains("wallet not found")),
        "the vendor's reason should reach the caller: {refusal:?}"
    );
}
