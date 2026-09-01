// SPDX-License-Identifier: Apache-2.0
//! Reading a customer's wallet, and the two ways that read could lie.
//!
//! The response bodies here are shaped like Privy's, not like Radar's
//! understanding of Privy's — a `linked_accounts` array holding several kinds of
//! account at once, which is what a real customer who logged in with Phantom and
//! was given an embedded wallet actually looks like.
//!
//! Two properties, and both are about *not* acting:
//!
//! 1. A missing `delegated` flag is not a grant (rule 9).
//! 2. The wallet returned must be the **embedded Solana** one, never an external
//!    wallet the customer connected in order to log in.

use radar_serve::privy::{Client, Credentials, Transport, Unavailable};

const DID: &str = "did:privy:cmthhkznr0a3u0cl86prxlb7x";

/// A transport that answers with a fixed body.
struct Canned(Result<String, String>);

impl Transport for Canned {
    fn get(&self, url: &str, credentials: &Credentials) -> Result<String, String> {
        assert!(
            url.ends_with(DID),
            "the DID must come from the verified token and reach the URL: {url}"
        );
        assert!(
            credentials.basic().starts_with("Basic "),
            "Privy authenticates the application with Basic auth"
        );
        self.0.clone()
    }
}

fn client(body: &str) -> Client {
    Client::with_transport(
        Credentials::new("app", "secret"),
        Box::new(Canned(Ok(body.to_owned()))),
    )
}

/// A customer with a Phantom wallet connected for login, an Ethereum embedded
/// wallet, and the Solana embedded wallet Radar cares about.
fn a_realistic_customer(delegated: Option<bool>) -> String {
    let flag = delegated.map_or(String::new(), |d| format!(r#","delegated":{d}"#));
    format!(
        r#"{{
          "id": "{DID}",
          "linked_accounts": [
            {{"type":"email","address":"someone@example.com"}},
            {{"type":"wallet","chain_type":"solana","connector_type":"phantom",
              "address":"PhantomAddressTheCustomerLoggedInWith","id":"ext-1"}},
            {{"type":"wallet","chain_type":"ethereum","connector_type":"embedded",
              "address":"0xEthereumEmbedded","id":"eth-1","delegated":true}},
            {{"type":"wallet","chain_type":"solana","connector_type":"embedded",
              "address":"SoLanaEmbeddedWalletAddress","id":"sol-1"{flag}}}
          ]
        }}"#
    )
}

#[test]
fn a_missing_delegation_flag_is_not_a_grant() {
    // Rule 9, on the field that decides whether Radar may trade someone's money.
    //
    // Privy omits `delegated` in some responses. Reading a missing value as
    // permission is the shape of mistake that ends with a wallet being traded
    // without consent, and it is one `unwrap_or(true)` away at all times.
    let wallet = client(&a_realistic_customer(None))
        .wallet_for(DID)
        .expect("the wallet is found");
    assert!(
        !wallet.delegated,
        "a field Privy did not send must not read as a grant"
    );
}

#[test]
fn an_explicit_refusal_is_carried_and_an_explicit_grant_is_too() {
    // Both directions. A reader that always returned `false` would pass the test
    // above and make the product permanently unable to trade — which is safe,
    // and would be discovered late and blamed on Privy.
    let refused = client(&a_realistic_customer(Some(false)))
        .wallet_for(DID)
        .expect("found");
    assert!(!refused.delegated);

    let granted = client(&a_realistic_customer(Some(true)))
        .wallet_for(DID)
        .expect("found");
    assert!(granted.delegated, "an explicit grant must be carried");
}

#[test]
fn the_wallet_returned_is_the_embedded_solana_one_and_not_a_connected_phantom() {
    // The customer in this fixture logged in with Phantom. That address appears
    // first in `linked_accounts`, is a wallet, and is on Solana — so a lookup
    // matching on any two of the three conditions returns it.
    //
    // Radar can never be a signer on it. Showing it as a deposit destination
    // would send the customer's funds somewhere Radar cannot trade from, and
    // returning it as *the* wallet would make every later step wrong.
    let wallet = client(&a_realistic_customer(Some(true)))
        .wallet_for(DID)
        .expect("found");

    assert_eq!(wallet.address, "SoLanaEmbeddedWalletAddress");
    assert_eq!(wallet.id, "sol-1");
}

#[test]
fn an_ethereum_embedded_wallet_is_not_mistaken_for_a_solana_one() {
    // The other near-miss in the same fixture: right connector, wrong chain, and
    // it even carries `delegated: true`.
    let body = r#"{"linked_accounts":[
        {"type":"wallet","chain_type":"ethereum","connector_type":"embedded",
         "address":"0xEthereumEmbedded","id":"eth-1","delegated":true}
    ]}"#;
    assert_eq!(client(body).wallet_for(DID), Err(Unavailable::NoWallet));
}

#[test]
fn a_customer_with_no_embedded_wallet_is_told_so_rather_than_given_nothing() {
    // A real answer, not a failure: someone who just signed up legitimately has
    // no wallet yet, and that reads differently from Privy being down.
    let body = r#"{"id":"x","linked_accounts":[{"type":"email","address":"a@b.c"}]}"#;
    assert_eq!(client(body).wallet_for(DID), Err(Unavailable::NoWallet));
}

#[test]
fn privy_being_unreachable_is_never_reported_as_an_absent_wallet() {
    // The distinction rule 8 turns on. "Radar could not ask" and "the customer
    // has no wallet" are indistinguishable to a caller that collapses them, and
    // only one of the two is safe to act on.
    let unreachable = Client::with_transport(
        Credentials::new("app", "secret"),
        Box::new(Canned(Err("connection refused".to_owned()))),
    );
    assert!(matches!(
        unreachable.wallet_for(DID),
        Err(Unavailable::Unreachable(_))
    ));

    // Reachable but answering nonsense is the same category, not a missing
    // wallet: a proxy returning an HTML error page must not read as "no wallet".
    let nonsense = client("<html>502 Bad Gateway</html>");
    assert!(matches!(
        nonsense.wallet_for(DID),
        Err(Unavailable::Unreachable(_))
    ));
}

#[test]
fn a_half_configured_credential_is_refused_rather_than_treated_as_absent() {
    // An id with no secret is a half-finished deployment. Reporting it as
    // "unconfigured" would hide the mistake behind a message about a feature
    // nobody turned on.
    let both = |k: &str| match k {
        "RADAR_PRIVY_APP_ID" => Some("app".to_owned()),
        "RADAR_PRIVY_APP_SECRET" => Some("secret".to_owned()),
        _ => None,
    };
    assert!(Credentials::from_vars(&both).is_ok());

    for present in ["RADAR_PRIVY_APP_ID", "RADAR_PRIVY_APP_SECRET"] {
        let half = |k: &str| (k == present).then(|| "value".to_owned());
        assert_eq!(
            Credentials::from_vars(&half),
            Err(Unavailable::NotConfigured),
            "{present} alone is not a credential"
        );
    }

    let blank = |k: &str| match k {
        "RADAR_PRIVY_APP_ID" => Some("app".to_owned()),
        "RADAR_PRIVY_APP_SECRET" => Some("   ".to_owned()),
        _ => None,
    };
    assert_eq!(
        Credentials::from_vars(&blank),
        Err(Unavailable::NotConfigured),
        "a blank secret is not a secret"
    );
}

#[test]
fn the_secret_never_reaches_a_debug_string() {
    // `Debug` output ends up in error messages and error messages end up in
    // logs. A derived `Debug` here would put the application secret in every
    // one of them, and the secret is what authenticates Radar to Privy.
    let credentials = Credentials::new("cmthhkznr0a3u0cl86prxlb7x", "the-actual-secret");
    let rendered = format!("{credentials:?}");

    assert!(
        !rendered.contains("the-actual-secret"),
        "the secret reached a Debug string: {rendered}"
    );
    assert!(
        rendered.contains("cmthhkznr0a3u0cl86prxlb7x"),
        "the application id should still be there, since it identifies which \
         application an operator is looking at: {rendered}"
    );
}
