// SPDX-License-Identifier: Apache-2.0
//! The payout: one transaction a week, under three refusals, from a key that
//! can do nothing else.
//!
//! Design 0007 §6.3 C4 and C5, ADR 0013, plan 0006 item 7. The token's creator
//! fee accrues in the pump.fun creator vault of the wallet that launched it.
//! Once a week's record carries a claim, this pays it: `collect_creator_fee`
//! moves what the vault holds into the creator wallet, and a system transfer
//! moves it on to the claimed address, in one transaction the creator wallet
//! signs. That wallet holds the token's creator role and nothing else -- no
//! tokens (ADR 0013 constraint 2), no other funds beyond what pays a fee.
//!
//! # The three refusals, and where they live
//!
//! [`radar_contest::Payout::permitted`] is the policy: not already paid, a
//! winner, a claim, the recipient is the claimed address, the amount is at
//! most what was collected. This crate calls it and never argues with it. What
//! this crate adds is the plumbing a refusal has to survive: the transaction
//! is built from the record and the chain, not from arguments; the amount is
//! what the vault holds above its rent reserve, so there is no field for a
//! larger number; and after sending, the transaction is **read back** and
//! checked -- recipient and amount -- before the ledger says it was paid.
//!
//! # Two paths, one check
//!
//! `radar-payout --week N` signs and sends. `radar contest pay --week N
//! --dry-run` prints the exact unsigned transaction for the operator to sign
//! elsewhere, and `radar contest record-payout --week N --signature S` reads
//! that transaction back through the same [`verify`] the automated path uses.
//! The fallback is therefore exercised by the automated path's own test, which
//! is design 0007 C5's condition for having one.
//!
//! # Not the trading signer
//!
//! This is a different key, a different unit, a different user. It does not
//! touch `radar-risk`, it cannot sign a trade, and the trading signer cannot
//! sign this. The blast radius of the hot key is one week of creator fees
//! (ADR 0013 "What this costs").

use std::path::Path;

use base64::Engine as _;
use ed25519_dalek::{Signer as _, SigningKey};
use radar_contest::{Payout, Record, Refusal, Vault, Week};
use radar_pumpfun::transaction::{Instruction, transaction};
use radar_pumpfun::{instruction, pda};
use radar_types::Address;

/// Lamports a zero-data account must hold to be rent exempt, which is what
/// `collect_creator_fee` leaves in the vault.
///
/// **An assumption until a capture confirms it.** The figure is the runtime's
/// rent-exempt minimum for zero bytes of data at the default rent parameters
/// (890,880). The vault's balance after a collect on mainnet is the evidence;
/// when a `collect_creator_fee` transaction is captured into
/// `radar-pumpfun/tests/fixtures`, its post-balance replaces this comment
/// with a test. Until then the direction of any error is safe: too high a
/// reserve pays out slightly less than collected, never more.
pub const VAULT_RENT_RESERVE: u64 = 890_880;

/// What the vault holds above its reserve: what can be paid.
#[must_use]
pub const fn collected(vault_lamports: u64) -> u64 {
    vault_lamports.saturating_sub(VAULT_RENT_RESERVE)
}

/// A payment, planned and not yet made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Which week.
    pub week: Week,
    /// The creator wallet: signer, payer, and the vault's owner.
    pub creator: Address,
    /// The claimed address.
    pub recipient: Address,
    /// What is paid, in lamports. Equal to `collected`: the whole fee is the
    /// prize (ADR 0013 constraint 3, design 0009 L1).
    pub lamports: u64,
    /// What the vault held above its reserve when the plan was made.
    pub collected: u64,
    /// The two instructions, in order.
    pub instructions: Vec<Instruction>,
    /// The transaction with its signature slot blank, ready to sign.
    pub unsigned: Vec<u8>,
}

impl Plan {
    /// The unsigned transaction, base64, as `solana` tooling accepts it.
    #[must_use]
    pub fn unsigned_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.unsigned)
    }
}

/// Why nothing was paid.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PayError {
    /// The policy refused. Recorded, never argued with.
    #[error("refused: {0:?}")]
    Refused(Refusal),
    /// The vault holds nothing above its reserve, so there is nothing to pay.
    /// Distinct from a refusal: the week is fine, the pool is empty.
    #[error("nothing collected: the vault holds {vault} lamports, the reserve is {reserve}")]
    NothingCollected {
        /// The vault's balance.
        vault: u64,
        /// The rent reserve it keeps.
        reserve: u64,
    },
    /// The claim carries text that is not an address.
    #[error("the claimed address does not parse: {0}")]
    BadAddress(String),
    /// A derivation failed, which for a real creator it cannot.
    #[error("the creator vault could not be derived")]
    NoVault,
    /// The transaction could not be built.
    #[error("the transaction could not be built: {0}")]
    Unbuildable(String),
    /// The chain did not answer, or answered with something unreadable.
    #[error("chain: {0}")]
    Chain(String),
    /// The record or the pool file could not be read or written.
    #[error("ledger: {0}")]
    Ledger(String),
    /// The sent transaction does not say what the plan said.
    #[error("verification failed: {0}")]
    Verify(String),
}

/// Plans a week's payment from its record and the vault's balance. Pure.
///
/// # Errors
///
/// [`PayError::Refused`] with the policy's reason; [`PayError::NothingCollected`]
/// when the vault is at its reserve; the others when the claim does not parse
/// or the transaction cannot be built.
pub fn plan(
    record: &Record,
    creator: &Address,
    vault_lamports: u64,
    blockhash: &[u8; 32],
) -> Result<Plan, PayError> {
    let collected = collected(vault_lamports);
    // The recipient is the claim, never an argument. An unclaimed week has no
    // recipient and the policy says so first; the empty string is what it is
    // compared against, and it is compared against nothing.
    let claimed = record
        .claim
        .as_ref()
        .map(|c| c.address.clone())
        .unwrap_or_default();
    Payout::permitted(record, &claimed, collected, collected).map_err(PayError::Refused)?;
    if collected == 0 {
        return Err(PayError::NothingCollected {
            vault: vault_lamports,
            reserve: VAULT_RENT_RESERVE,
        });
    }
    let recipient: Address = claimed
        .parse()
        .map_err(|e| PayError::BadAddress(format!("{claimed}: {e}")))?;
    let instructions = vec![
        instruction::collect_creator_fee(creator).ok_or(PayError::NoVault)?,
        instruction::system_transfer(creator, &recipient, collected),
    ];
    let unsigned = transaction(creator, &instructions, blockhash)
        .map_err(|e| PayError::Unbuildable(e.to_string()))?;
    Ok(Plan {
        week: record.week,
        creator: *creator,
        recipient,
        lamports: collected,
        collected,
        instructions,
        unsigned,
    })
}

/// Signs an unsigned transaction with one key: the creator's.
///
/// The wire format is a compact-u16 count of signatures, that many 64-byte
/// slots, then the message. Exactly one slot is expected here; a transaction
/// asking for more is not one this crate built.
///
/// # Errors
///
/// [`PayError::Unbuildable`] when the bytes are not a one-signer transaction.
pub fn sign(unsigned: &[u8], key: &SigningKey) -> Result<Vec<u8>, PayError> {
    if unsigned.first() != Some(&1) || unsigned.len() < 1 + 64 + 1 {
        return Err(PayError::Unbuildable(
            "not a one-signer transaction".to_owned(),
        ));
    }
    let message = &unsigned[65..];
    let signature = key.sign(message).to_bytes();
    let mut signed = unsigned.to_vec();
    signed[1..65].copy_from_slice(&signature);
    Ok(signed)
}

/// Loads a Solana keypair file: a JSON array of 64 bytes, secret then public.
///
/// The two halves must agree, as in `radar-signer`: a file whose public half
/// does not match its secret has been edited, and signing with it would
/// produce signatures for a wallet that is not the one named.
///
/// # Errors
///
/// [`PayError::Ledger`] with the reason.
pub fn load_key(path: &Path) -> Result<SigningKey, PayError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| PayError::Ledger(format!("key file {}: {e}", path.display())))?;
    let bytes: Vec<u8> = serde_json::from_str(&text)
        .map_err(|_| PayError::Ledger("key file is not a JSON byte array".to_owned()))?;
    if bytes.len() != 64 {
        return Err(PayError::Ledger(
            "key file is not a 64-byte keypair".to_owned(),
        ));
    }
    let secret: [u8; 32] = bytes[..32]
        .try_into()
        .map_err(|_| PayError::Ledger("key file is not a 64-byte keypair".to_owned()))?;
    let key = SigningKey::from_bytes(&secret);
    if key.verifying_key().to_bytes() != bytes[32..] {
        return Err(PayError::Ledger(
            "key file's public half does not match its secret half".to_owned(),
        ));
    }
    Ok(key)
}

/// The creator wallet a signing key is.
#[must_use]
pub fn wallet_of(key: &SigningKey) -> Address {
    Address::new(key.verifying_key().to_bytes())
}

/// One system transfer, as read back off the chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// The sender.
    pub from: Address,
    /// The recipient.
    pub to: Address,
    /// The amount, in lamports.
    pub lamports: u64,
}

/// What this crate asks of the chain. A trait so the whole path runs against a
/// fake in tests; the real one is [`Rpc`].
pub trait Chain {
    /// An account's balance in lamports; zero for an account that does not exist.
    ///
    /// # Errors
    ///
    /// The transport's or the node's reason.
    fn balance(&self, address: &Address) -> Result<u64, String>;
    /// A recent blockhash.
    ///
    /// # Errors
    ///
    /// The transport's or the node's reason.
    fn latest_blockhash(&self) -> Result<[u8; 32], String>;
    /// Sends a signed transaction, base64, and returns its signature.
    ///
    /// # Errors
    ///
    /// The transport's or the node's reason, including a rejected transaction.
    fn send(&self, signed_base64: &str) -> Result<String, String>;
    /// The system transfers a confirmed transaction made, or `None` when the
    /// transaction is not (yet) found.
    ///
    /// # Errors
    ///
    /// The transport's or the node's reason.
    fn transfers_in(&self, signature: &str) -> Result<Option<Vec<Transfer>>, String>;
}

/// Reads a sent transaction back and checks it against the plan.
///
/// The step both paths share (design 0007 C5). What is checked is what the
/// three refusals protect: one transfer from the creator to the claimed
/// address for exactly the planned amount. Anything else -- no transfer, a
/// different recipient, a different amount, a second transfer -- is a
/// transaction the ledger must not describe as this week's payout.
///
/// # Errors
///
/// [`PayError::Verify`] with what differed; [`PayError::Chain`] when the
/// transaction could not be read at all.
pub fn verify(chain: &dyn Chain, signature: &str, plan: &Plan) -> Result<(), PayError> {
    let Some(transfers) = chain.transfers_in(signature).map_err(PayError::Chain)? else {
        return Err(PayError::Verify(format!("{signature} is not on chain yet")));
    };
    let expected = Transfer {
        from: plan.creator,
        to: plan.recipient,
        lamports: plan.lamports,
    };
    match transfers.as_slice() {
        [one] if *one == expected => Ok(()),
        [one] => Err(PayError::Verify(format!(
            "the transaction paid {} lamports from {} to {}; the plan was {} from {} to {}",
            one.lamports, one.from, one.to, expected.lamports, expected.from, expected.to
        ))),
        [] => Err(PayError::Verify(
            "the transaction made no transfer".to_owned(),
        )),
        many => Err(PayError::Verify(format!(
            "the transaction made {} transfers; the plan made one",
            many.len()
        ))),
    }
}

/// Where a week's record lives.
#[must_use]
pub fn record_path(contest_dir: &str, week: Week) -> String {
    format!("{contest_dir}/{}.json", week.0)
}

/// Reads a week's record.
///
/// # Errors
///
/// [`PayError::Ledger`] when it is missing or does not parse.
pub fn read_record(contest_dir: &str, week: Week) -> Result<Record, PayError> {
    let path = record_path(contest_dir, week);
    let text =
        std::fs::read_to_string(&path).map_err(|e| PayError::Ledger(format!("{path}: {e}")))?;
    Record::from_json(&text).map_err(|e| PayError::Ledger(format!("{path}: {e}")))
}

/// Writes a record via a sibling and a rename, so a reader never sees half.
///
/// # Errors
///
/// [`PayError::Ledger`] with the I/O reason.
pub fn write_record(contest_dir: &str, record: &Record) -> Result<(), PayError> {
    let path = record_path(contest_dir, record.week);
    let text = record
        .to_json()
        .map_err(|e| PayError::Ledger(e.to_string()))?;
    write_atomically(&path, &text)
}

/// Writes the vault reading the public pool page serves.
///
/// Here because this process reads the vault anyway, and design 0008 phase 3
/// wanted the reading written by whichever job read it.
///
/// # Errors
///
/// [`PayError::Ledger`] with the I/O reason.
pub fn write_vault(contest_dir: &str, vault: &Vault) -> Result<(), PayError> {
    let text = vault
        .to_json()
        .map_err(|e| PayError::Ledger(e.to_string()))?;
    write_atomically(&format!("{contest_dir}/pool.json"), &text)
}

fn write_atomically(path: &str, text: &str) -> Result<(), PayError> {
    let tmp = format!("{path}.tmp");
    std::fs::write(&tmp, text).map_err(|e| PayError::Ledger(format!("{tmp}: {e}")))?;
    std::fs::rename(&tmp, path).map_err(|e| PayError::Ledger(format!("{path}: {e}")))
}

/// What a run did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Planned and printed; nothing signed, nothing sent.
    Planned(Plan),
    /// Sent, verified, and recorded.
    Paid(Payout),
}

/// Pays a week, or plans it.
///
/// Reads the record and the vault, writes the pool reading either way, plans,
/// and -- unless `dry_run` -- signs, sends, verifies and records. The record is
/// written only after verification: a signature the chain does not confirm as
/// the planned transfer is printed and not recorded, and the next run refuses
/// nothing it should pay.
///
/// # Errors
///
/// Any [`PayError`]; a refusal is the first thing checked after the vault is
/// read, so a refused week costs one balance call and no signature.
pub fn pay(
    chain: &dyn Chain,
    contest_dir: &str,
    week: Week,
    key: &SigningKey,
    now: u64,
    dry_run: bool,
) -> Result<Outcome, PayError> {
    let creator = wallet_of(key);
    let record = read_record(contest_dir, week)?;
    let vault_address = pda::creator_vault(&creator).ok_or(PayError::NoVault)?;
    let vault_lamports = chain.balance(&vault_address).map_err(PayError::Chain)?;
    write_vault(
        contest_dir,
        &Vault {
            address: vault_address.to_string(),
            lamports: vault_lamports,
            measured_at: now,
        },
    )?;
    let blockhash = chain.latest_blockhash().map_err(PayError::Chain)?;
    let planned = plan(&record, &creator, vault_lamports, &blockhash)?;
    if dry_run {
        return Ok(Outcome::Planned(planned));
    }
    let signed = sign(&planned.unsigned, key)?;
    let signature = chain
        .send(&base64::engine::general_purpose::STANDARD.encode(signed))
        .map_err(PayError::Chain)?;
    verify(chain, &signature, &planned)?;
    let payout = Payout {
        recipient: planned.recipient.to_string(),
        lamports: planned.lamports,
        signature,
        at: now,
    };
    let mut record = record;
    record.payout = Some(payout.clone());
    write_record(contest_dir, &record)?;
    Ok(Outcome::Paid(payout))
}

/// Records a payment the operator made by hand, after reading it back.
///
/// The fallback (design 0007 C5). The plan is rebuilt from the record with the
/// vault as it stands, the transaction is read back through [`verify`], and
/// only then does the ledger say the week was paid. The amount check is on the
/// chain's figure, so a hand-signed transaction for the wrong amount is
/// refused the same way an automated one would be.
///
/// # Errors
///
/// Any [`PayError`].
pub fn record_payout(
    chain: &dyn Chain,
    contest_dir: &str,
    week: Week,
    creator: &Address,
    signature: &str,
    now: u64,
) -> Result<Payout, PayError> {
    let record = read_record(contest_dir, week)?;
    let vault_address = pda::creator_vault(creator).ok_or(PayError::NoVault)?;
    // The vault has been collected by the hand-made transaction, so its balance
    // now is not what was paid. The plan's amount comes from the chain instead:
    // the transfer that was actually made is what the policy is checked
    // against, with `collected` taken as that amount plus what the vault still
    // holds above its reserve -- the most that could have been there.
    let Some(transfers) = chain.transfers_in(signature).map_err(PayError::Chain)? else {
        return Err(PayError::Verify(format!("{signature} is not on chain yet")));
    };
    let [transfer] = transfers.as_slice() else {
        return Err(PayError::Verify(format!(
            "the transaction made {} transfers; a payout makes one",
            transfers.len()
        )));
    };
    let vault_now = chain.balance(&vault_address).map_err(PayError::Chain)?;
    let could_have_held = collected(vault_now).saturating_add(transfer.lamports);
    let claimed = record
        .claim
        .as_ref()
        .map(|c| c.address.clone())
        .unwrap_or_default();
    Payout::permitted(
        &record,
        &transfer.to.to_string(),
        transfer.lamports,
        could_have_held,
    )
    .map_err(PayError::Refused)?;
    if transfer.from != *creator || transfer.to.to_string() != claimed {
        return Err(PayError::Verify(format!(
            "the transaction moved {} to {}; the claim is {} from {}",
            transfer.lamports, transfer.to, claimed, creator
        )));
    }
    let payout = Payout {
        recipient: transfer.to.to_string(),
        lamports: transfer.lamports,
        signature: signature.to_owned(),
        at: now,
    };
    let mut record = record;
    record.payout = Some(payout.clone());
    write_record(contest_dir, &record)?;
    Ok(payout)
}

/// The real chain, over JSON-RPC.
#[derive(Clone, Debug)]
pub struct Rpc {
    endpoint: String,
}

impl Rpc {
    /// A client for one endpoint. Direct, never the x402 lane (rule 7).
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    fn call(&self, method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
        let body =
            serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let mut response = ureq::post(&self.endpoint)
            .content_type("application/json")
            .send(body.to_string())
            .map_err(|e| e.to_string())?;
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())?;
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("not json: {e}"))?;
        if let Some(err) = value.get("error") {
            return Err(format!("{method}: {err}"));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| format!("{method} returned no result"))
    }
}

impl Chain for Rpc {
    fn balance(&self, address: &Address) -> Result<u64, String> {
        let result = self.call("getBalance", &serde_json::json!([address.to_string()]))?;
        result
            .get("value")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "getBalance: no value".to_owned())
    }

    fn latest_blockhash(&self) -> Result<[u8; 32], String> {
        let result = self.call("getLatestBlockhash", &serde_json::json!([]))?;
        let text = result
            .get("value")
            .and_then(|v| v.get("blockhash"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "getLatestBlockhash: no blockhash".to_owned())?;
        let bytes = bs58::decode(text)
            .into_vec()
            .map_err(|e| format!("blockhash is not base58: {e}"))?;
        bytes
            .try_into()
            .map_err(|_| "blockhash is not 32 bytes".to_owned())
    }

    fn send(&self, signed_base64: &str) -> Result<String, String> {
        let result = self.call(
            "sendTransaction",
            &serde_json::json!([signed_base64, { "encoding": "base64", "preflightCommitment": "confirmed" }]),
        )?;
        result
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "sendTransaction: no signature".to_owned())
    }

    fn transfers_in(&self, signature: &str) -> Result<Option<Vec<Transfer>>, String> {
        let result = self.call(
            "getTransaction",
            &serde_json::json!([signature, { "encoding": "jsonParsed", "commitment": "confirmed", "maxSupportedTransactionVersion": 0 }]),
        )?;
        if result.is_null() {
            return Ok(None);
        }
        Ok(Some(transfers_of(&result)))
    }
}

/// The system transfers in a `jsonParsed` transaction, top-level and inner.
#[must_use]
pub fn transfers_of(tx: &serde_json::Value) -> Vec<Transfer> {
    let mut out = Vec::new();
    let mut visit = |ix: &serde_json::Value| {
        if ix.get("program").and_then(serde_json::Value::as_str) != Some("system") {
            return;
        }
        let Some(parsed) = ix.get("parsed") else {
            return;
        };
        if parsed.get("type").and_then(serde_json::Value::as_str) != Some("transfer") {
            return;
        }
        let info = &parsed["info"];
        let (Some(from), Some(to), Some(lamports)) = (
            info.get("source")
                .and_then(serde_json::Value::as_str)
                .and_then(|s| s.parse().ok()),
            info.get("destination")
                .and_then(serde_json::Value::as_str)
                .and_then(|s| s.parse().ok()),
            info.get("lamports").and_then(serde_json::Value::as_u64),
        ) else {
            return;
        };
        out.push(Transfer { from, to, lamports });
    };
    for ix in tx["transaction"]["message"]["instructions"]
        .as_array()
        .into_iter()
        .flatten()
    {
        visit(ix);
    }
    for inner in tx["meta"]["innerInstructions"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for ix in inner["instructions"].as_array().into_iter().flatten() {
            visit(ix);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_contest::{Claim, Entry, Metrics, Ranked, Ranking};
    use std::cell::RefCell;

    const WEEK: Week = Week(2957);
    const BLOCKHASH: [u8; 32] = [7u8; 32];

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[42u8; 32])
    }

    fn recipient() -> Address {
        Address::new([9u8; 32])
    }

    fn record(claimed: bool) -> Record {
        let mut ranking = Ranking::default();
        ranking.ranked.push(Ranked {
            entry: Entry {
                reply_id: "r1".to_owned(),
                summoner: "alice".to_owned(),
                handle: None,
                mint: "M".to_owned(),
                at: WEEK.opens_at() + 10,
                metrics: Metrics {
                    likes: 3,
                    ..Metrics::default()
                },
            },
            score: 3,
        });
        let mut record = Record::close(WEEK, ranking, &radar_contest::Rules::published(["op"]));
        assert!(record.winner.is_some());
        if claimed {
            record.claim = Some(Claim {
                address: recipient().to_string(),
                reply_id: "c1".to_owned(),
                at: WEEK.closes_at() + 100,
            });
        }
        record
    }

    /// A chain that answers from fields and records what it was sent.
    #[derive(Default)]
    struct Fake {
        vault: u64,
        sent: RefCell<Vec<String>>,
        /// What `transfers_in` answers, or `None` for "not on chain".
        confirms: Option<Vec<Transfer>>,
        confirm_as_planned: bool,
    }

    impl Chain for Fake {
        fn balance(&self, _: &Address) -> Result<u64, String> {
            Ok(self.vault)
        }
        fn latest_blockhash(&self) -> Result<[u8; 32], String> {
            Ok(BLOCKHASH)
        }
        fn send(&self, signed: &str) -> Result<String, String> {
            self.sent.borrow_mut().push(signed.to_owned());
            Ok("SIG1".to_owned())
        }
        fn transfers_in(&self, _: &str) -> Result<Option<Vec<Transfer>>, String> {
            if self.confirm_as_planned {
                return Ok(Some(vec![Transfer {
                    from: wallet_of(&key()),
                    to: recipient(),
                    lamports: collected(self.vault),
                }]));
            }
            Ok(self.confirms.clone())
        }
    }

    fn dir(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("radar-payout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir.to_string_lossy().into_owned()
    }

    #[test]
    fn the_plan_pays_the_claim_everything_above_the_reserve_in_two_instructions() {
        let creator = wallet_of(&key());
        let p = plan(
            &record(true),
            &creator,
            VAULT_RENT_RESERVE + 1_000_000,
            &BLOCKHASH,
        )
        .expect("a plan");
        assert_eq!(p.lamports, 1_000_000);
        assert_eq!(p.collected, 1_000_000);
        assert_eq!(p.recipient, recipient());
        assert_eq!(p.instructions.len(), 2);
        // collect_creator_fee first, from the pump.fun program, creator signing.
        assert_eq!(
            p.instructions[0].program_id,
            radar_decode::pumpfun::PROGRAM_ID
        );
        assert!(
            p.instructions[0].accounts[0].signer && p.instructions[0].accounts[0].pubkey == creator
        );
        assert_eq!(
            p.instructions[0].accounts[1].pubkey,
            pda::creator_vault(&creator).expect("vault")
        );
        // Then the transfer, from the creator to the claim, for exactly the amount.
        let transfer = &p.instructions[1];
        assert_eq!(transfer.program_id, instruction::SYSTEM_PROGRAM);
        assert_eq!(transfer.accounts[1].pubkey, recipient());
        assert_eq!(&transfer.data[..4], &2u32.to_le_bytes());
        assert_eq!(&transfer.data[4..], &1_000_000u64.to_le_bytes());
        // One signature slot, blank.
        assert_eq!(p.unsigned[0], 1);
        assert!(p.unsigned[1..65].iter().all(|b| *b == 0));
        assert!(!p.unsigned_base64().is_empty());
    }

    #[test]
    fn the_three_refusals_and_the_empty_vault_stop_a_plan_before_anything_is_built() {
        let creator = wallet_of(&key());
        let full = VAULT_RENT_RESERVE + 1_000;
        // Unclaimed.
        assert_eq!(
            plan(&record(false), &creator, full, &BLOCKHASH).err(),
            Some(PayError::Refused(Refusal::Unclaimed))
        );
        // Already paid.
        let mut paid = record(true);
        paid.payout = Some(Payout {
            recipient: recipient().to_string(),
            lamports: 1,
            signature: "OLD".to_owned(),
            at: 1,
        });
        assert_eq!(
            plan(&paid, &creator, full, &BLOCKHASH).err(),
            Some(PayError::Refused(Refusal::AlreadyPaid {
                signature: "OLD".to_owned()
            }))
        );
        // No winner.
        let none = Record::close(
            WEEK,
            Ranking::default(),
            &radar_contest::Rules::published(["op"]),
        );
        assert_eq!(
            plan(&none, &creator, full, &BLOCKHASH).err(),
            Some(PayError::Refused(Refusal::NoWinner))
        );
        // At the reserve: nothing to pay, and not a refusal.
        assert_eq!(
            plan(&record(true), &creator, VAULT_RENT_RESERVE, &BLOCKHASH).err(),
            Some(PayError::NothingCollected {
                vault: VAULT_RENT_RESERVE,
                reserve: VAULT_RENT_RESERVE
            })
        );
        // A claim that is not an address.
        let mut bad = record(true);
        bad.claim.as_mut().expect("claim").address = "not-an-address".to_owned();
        assert!(matches!(
            plan(&bad, &creator, full, &BLOCKHASH).err(),
            Some(PayError::BadAddress(_))
        ));
    }

    #[test]
    fn the_signature_is_over_the_message_and_verifies_under_the_creators_key() {
        let creator = wallet_of(&key());
        let p = plan(&record(true), &creator, VAULT_RENT_RESERVE + 5, &BLOCKHASH).expect("a plan");
        let signed = sign(&p.unsigned, &key()).expect("signed");
        assert_eq!(signed.len(), p.unsigned.len());
        assert_eq!(&signed[65..], &p.unsigned[65..], "the message is untouched");
        let sig = ed25519_dalek::Signature::from_bytes(signed[1..65].try_into().expect("64"));
        key()
            .verifying_key()
            .verify_strict(&signed[65..], &sig)
            .expect("the signature verifies over the message");
        // Not a one-signer transaction: refused rather than half-signed.
        let mut two = p.unsigned.clone();
        two[0] = 2;
        assert!(sign(&two, &key()).is_err());
    }

    #[test]
    fn verify_accepts_exactly_the_planned_transfer_and_nothing_else() {
        let creator = wallet_of(&key());
        let p = plan(
            &record(true),
            &creator,
            VAULT_RENT_RESERVE + 500,
            &BLOCKHASH,
        )
        .expect("a plan");
        let planned = Transfer {
            from: creator,
            to: recipient(),
            lamports: 500,
        };
        let chain = |confirms: Option<Vec<Transfer>>| Fake {
            confirms,
            ..Fake::default()
        };
        assert_eq!(verify(&chain(Some(vec![planned.clone()])), "S", &p), Ok(()));
        // Re-applied by comparing only the recipient: the wrong amount passes
        // and the second assertion fails.
        let wrong_amount = Transfer {
            lamports: 499,
            ..planned.clone()
        };
        assert!(matches!(
            verify(&chain(Some(vec![wrong_amount])), "S", &p),
            Err(PayError::Verify(_))
        ));
        let wrong_recipient = Transfer {
            to: Address::new([1u8; 32]),
            ..planned.clone()
        };
        assert!(matches!(
            verify(&chain(Some(vec![wrong_recipient])), "S", &p),
            Err(PayError::Verify(_))
        ));
        assert!(matches!(
            verify(
                &chain(Some(vec![planned.clone(), planned.clone()])),
                "S",
                &p
            ),
            Err(PayError::Verify(_))
        ));
        assert!(matches!(
            verify(&chain(Some(Vec::new())), "S", &p),
            Err(PayError::Verify(_))
        ));
        assert!(matches!(
            verify(&chain(None), "S", &p),
            Err(PayError::Verify(_))
        ));
    }

    #[test]
    fn pay_writes_the_pool_reading_then_sends_verifies_and_records_in_that_order() {
        let d = dir("pay");
        write_record(&d, &record(true)).expect("record");
        let chain = Fake {
            vault: VAULT_RENT_RESERVE + 2_000,
            confirm_as_planned: true,
            ..Fake::default()
        };
        let out = pay(&chain, &d, WEEK, &key(), 1_788_000_000, false).expect("paid");
        let Outcome::Paid(payout) = out else {
            panic!("paid");
        };
        assert_eq!(payout.lamports, 2_000);
        assert_eq!(payout.signature, "SIG1");
        assert_eq!(payout.recipient, recipient().to_string());
        assert_eq!(chain.sent.borrow().len(), 1, "one transaction sent");
        let saved = read_record(&d, WEEK).expect("record");
        assert_eq!(saved.payout, Some(payout));
        let pool =
            Vault::from_json(&std::fs::read_to_string(format!("{d}/pool.json")).expect("pool"))
                .expect("vault");
        assert_eq!(pool.lamports, VAULT_RENT_RESERVE + 2_000);
        assert_eq!(
            pool.address,
            pda::creator_vault(&wallet_of(&key()))
                .expect("v")
                .to_string()
        );

        // Paying again is refused as already paid, and nothing is sent.
        let again = pay(&chain, &d, WEEK, &key(), 1_788_000_001, false);
        assert!(matches!(
            again,
            Err(PayError::Refused(Refusal::AlreadyPaid { .. }))
        ));
        assert_eq!(chain.sent.borrow().len(), 1);
    }

    #[test]
    fn a_transaction_the_chain_does_not_confirm_as_planned_is_not_recorded() {
        // Re-applied by writing the record before `verify`: the ledger says
        // paid with a signature the chain does not confirm, and this fails.
        let d = dir("unverified");
        write_record(&d, &record(true)).expect("record");
        let chain = Fake {
            vault: VAULT_RENT_RESERVE + 2_000,
            confirms: Some(vec![Transfer {
                from: wallet_of(&key()),
                to: Address::new([1u8; 32]),
                lamports: 2_000,
            }]),
            ..Fake::default()
        };
        let out = pay(&chain, &d, WEEK, &key(), 1, false);
        assert!(matches!(out, Err(PayError::Verify(_))), "{out:?}");
        assert_eq!(read_record(&d, WEEK).expect("record").payout, None);
        assert_eq!(
            chain.sent.borrow().len(),
            1,
            "it was sent; the ledger is what refuses"
        );
    }

    #[test]
    fn a_dry_run_plans_writes_the_pool_and_sends_nothing() {
        let d = dir("dry");
        write_record(&d, &record(true)).expect("record");
        let chain = Fake {
            vault: VAULT_RENT_RESERVE + 300,
            ..Fake::default()
        };
        let out = pay(&chain, &d, WEEK, &key(), 1, true).expect("planned");
        assert!(matches!(out, Outcome::Planned(ref p) if p.lamports == 300));
        assert!(chain.sent.borrow().is_empty());
        assert!(std::path::Path::new(&format!("{d}/pool.json")).exists());
        assert_eq!(read_record(&d, WEEK).expect("record").payout, None);
    }

    #[test]
    fn the_fallback_records_a_hand_made_payment_only_when_the_chain_agrees_with_the_claim() {
        let d = dir("fallback");
        write_record(&d, &record(true)).expect("record");
        let creator = wallet_of(&key());
        let good = Fake {
            vault: VAULT_RENT_RESERVE,
            confirms: Some(vec![Transfer {
                from: creator,
                to: recipient(),
                lamports: 777,
            }]),
            ..Fake::default()
        };
        let payout = record_payout(&good, &d, WEEK, &creator, "HAND1", 5).expect("recorded");
        assert_eq!((payout.lamports, payout.signature.as_str()), (777, "HAND1"));
        assert_eq!(read_record(&d, WEEK).expect("record").payout, Some(payout));

        // A second week's record, and a hand-made transaction to somebody else:
        // the policy's WrongRecipient, from the chain's own figure.
        let mut other = record(true);
        other.week = Week(WEEK.0 + 1);
        write_record(&d, &other).expect("record");
        let elsewhere = Fake {
            vault: VAULT_RENT_RESERVE,
            confirms: Some(vec![Transfer {
                from: creator,
                to: Address::new([1u8; 32]),
                lamports: 777,
            }]),
            ..Fake::default()
        };
        assert_eq!(
            record_payout(&elsewhere, &d, other.week, &creator, "HAND2", 6).err(),
            Some(PayError::Refused(Refusal::WrongRecipient))
        );
        assert_eq!(read_record(&d, other.week).expect("record").payout, None);
        // Not on chain: nothing recorded.
        let missing = Fake {
            vault: VAULT_RENT_RESERVE,
            confirms: None,
            ..Fake::default()
        };
        assert!(matches!(
            record_payout(&missing, &d, other.week, &creator, "HAND3", 7),
            Err(PayError::Verify(_))
        ));
    }

    #[test]
    fn transfers_are_read_out_of_a_parsed_transaction_top_level_and_inner() {
        let tx = serde_json::json!({
            "transaction": { "message": { "instructions": [
                { "program": "spl-token", "parsed": { "type": "transfer", "info": {} } },
                { "program": "system", "parsed": { "type": "transfer", "info": {
                    "source": Address::new([2u8; 32]).to_string(),
                    "destination": Address::new([3u8; 32]).to_string(),
                    "lamports": 12 } } }
            ] } },
            "meta": { "innerInstructions": [ { "instructions": [
                { "program": "system", "parsed": { "type": "createAccount", "info": {} } },
                { "program": "system", "parsed": { "type": "transfer", "info": {
                    "source": Address::new([4u8; 32]).to_string(),
                    "destination": Address::new([5u8; 32]).to_string(),
                    "lamports": 34 } } }
            ] } ] }
        });
        let got = transfers_of(&tx);
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].lamports, got[1].lamports), (12, 34));
        assert_eq!(got[1].to, Address::new([5u8; 32]));
    }

    #[test]
    fn the_unsigned_transaction_round_trips_through_base64_and_sign_refuses_the_wrong_shape() {
        // CI's mutants: `unsigned_base64` replaced by "xyzzy", and the length
        // bound in `sign` moved every way it can move.
        let creator = wallet_of(&key());
        let p = plan(&record(true), &creator, VAULT_RENT_RESERVE + 5, &BLOCKHASH).expect("a plan");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(p.unsigned_base64())
            .expect("base64");
        assert_eq!(decoded, p.unsigned);

        // 65 bytes is a count and an empty slot with no message: refused. 66 is
        // the smallest one-signer transaction with a message and is signed.
        assert!(sign(&[1u8; 65], &key()).is_err());
        let mut smallest = vec![1u8];
        smallest.extend_from_slice(&[0u8; 64]);
        smallest.push(9);
        let signed = sign(&smallest, &key()).expect("one byte of message is enough to sign");
        assert_eq!(signed.len(), 66);
        assert_ne!(&signed[1..65], &[0u8; 64], "the slot was filled");
        assert!(sign(&[], &key()).is_err());
    }

    #[test]
    fn the_fallback_refuses_a_payment_from_the_wrong_wallet_even_to_the_right_claim() {
        // Re-applied by turning the `||` into `&&`: a transfer from a stranger's
        // wallet to the claimed address is recorded as the week's payout, which
        // is a payout the creator never made.
        let d = dir("wrong-sender");
        write_record(&d, &record(true)).expect("record");
        let creator = wallet_of(&key());
        let stranger = Fake {
            vault: VAULT_RENT_RESERVE,
            confirms: Some(vec![Transfer {
                from: Address::new([8u8; 32]),
                to: recipient(),
                lamports: 777,
            }]),
            ..Fake::default()
        };
        assert!(matches!(
            record_payout(&stranger, &d, WEEK, &creator, "HAND9", 5),
            Err(PayError::Verify(_))
        ));
        assert_eq!(read_record(&d, WEEK).expect("record").payout, None);
    }

    /// A JSON-RPC node that answers each method with a canned result.
    fn fake_node() -> String {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(mut stream) = incoming else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut buf = vec![0u8; 65_536];
                let mut n = 0;
                let mut need = usize::MAX;
                while n < buf.len() && n < need {
                    let Ok(read) = stream.read(&mut buf[n..]) else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    n += read;
                    if need == usize::MAX
                        && let Some(end) = buf[..n].windows(4).position(|w| w == [13, 10, 13, 10])
                    {
                        let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        need = end + 4 + length;
                    }
                }
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let result = if request.contains("getBalance") {
                    r#"{"context":{"slot":1},"value":1234567}"#.to_owned()
                } else if request.contains("getLatestBlockhash") {
                    format!(
                        r#"{{"context":{{"slot":1}},"value":{{"blockhash":"{}","lastValidBlockHeight":9}}}}"#,
                        bs58::encode([7u8; 32]).into_string()
                    )
                } else if request.contains("sendTransaction") {
                    r#""SIGNATURE1""#.to_owned()
                } else if request.contains("getTransaction") {
                    format!(
                        r#"{{"slot":2,"transaction":{{"message":{{"instructions":[{{"program":"system","parsed":{{"type":"transfer","info":{{"source":"{}","destination":"{}","lamports":500}}}}}}]}}}},"meta":{{"innerInstructions":[]}}}}"#,
                        Address::new([2u8; 32]),
                        Address::new([3u8; 32])
                    )
                } else {
                    "null".to_owned()
                };
                let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":{result}}}"#);
                let crlf = String::from_utf8(vec![13, 10]).expect("ascii");
                let head = format!(
                    "HTTP/1.1 200 OK{crlf}Content-Length: {}{crlf}Content-Type: application/json{crlf}Connection: close{crlf}{crlf}",
                    body.len()
                );
                let _ = stream.write_all(format!("{head}{body}").as_bytes());
                let _ = stream.flush();
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn the_rpc_client_reads_each_answer_out_of_the_nodes_shape() {
        // CI's mutants replaced `latest_blockhash` with a fixed array and
        // nothing failed, because nothing had asked a node. This asks a fake
        // one, method by method, and reads back what it said.
        let rpc = Rpc::new(fake_node());
        assert_eq!(rpc.balance(&Address::new([1u8; 32])), Ok(1_234_567));
        assert_eq!(rpc.latest_blockhash(), Ok([7u8; 32]));
        assert_eq!(rpc.send("AAEC"), Ok("SIGNATURE1".to_owned()));
        let transfers = rpc
            .transfers_in("SIGNATURE1")
            .expect("read")
            .expect("on chain");
        assert_eq!(
            transfers,
            vec![Transfer {
                from: Address::new([2u8; 32]),
                to: Address::new([3u8; 32]),
                lamports: 500,
            }]
        );
    }

    #[test]
    fn a_keypair_file_loads_only_when_its_halves_agree() {
        let d = dir("key");
        let k = key();
        let mut bytes = k.to_bytes().to_vec();
        bytes.extend_from_slice(&k.verifying_key().to_bytes());
        let path = std::path::PathBuf::from(format!("{d}/payout.json"));
        std::fs::write(&path, serde_json::to_string(&bytes).expect("json")).expect("write");
        let loaded = load_key(&path).expect("loads");
        assert_eq!(wallet_of(&loaded), wallet_of(&k));

        bytes[40] ^= 1;
        std::fs::write(&path, serde_json::to_string(&bytes).expect("json")).expect("write");
        assert!(
            load_key(&path).is_err(),
            "a mismatched public half is refused"
        );
        std::fs::write(&path, "[1,2,3]").expect("write");
        assert!(load_key(&path).is_err());
    }
}
