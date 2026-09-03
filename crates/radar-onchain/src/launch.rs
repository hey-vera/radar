// SPDX-License-Identifier: Apache-2.0
//! Reconstructing a launch block from transactions.
//!
//! Pure. Everything here takes decoded transactions and returns facts, so the
//! hard part is testable against captured bytes rather than against an endpoint
//! that is a different thing every hour.
//!
//! # Why this is possible at all
//!
//! The launch block is one Solana block, and it is on chain the instant it
//! happens. The store cannot answer this question — `Reader::read` decodes and
//! sorts an entire table, ten seconds at 167,987 launches against a store that
//! now holds 483,629 — and the recipient count was never a stored field, only
//! the label derived from it. So the store holds the verdict and not the number,
//! for 7,543 mints out of 483,629, and essentially never for the one somebody is
//! asking about.
//!
//! Reading the chain instead is faster, fresher and answers for any mint.
//!
//! # A live read is not a replay
//!
//! AGENTS.md rule 3 governs reads *out of the store*, and it is satisfied here
//! by not being one. An on-demand chain read is a live observation taken at a
//! slot it reports, which is why [`LaunchBlock`] carries that slot. It must
//! never be written into the store as though it had been recorded at the time —
//! doing so would put a value from the future into a table a replay reads.

use radar_decode::pumpfun;
use radar_types::{Address, Slot, Trust};

use crate::budget::Count;
use crate::rpc::Transaction;

/// What a token's launch block contained.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchBlock {
    /// The slot it launched in.
    pub slot: Slot,
    /// The creator, as the launch instruction recorded it.
    pub creator: Address,
    /// Distinct token accounts that received the token inside the launch block.
    ///
    /// **Recipients, not buyers, and never owners.** A destination is a token
    /// account — an `(owner, mint)` pair — so this counts accounts and not
    /// people. Research 0012 shows the obvious follow-up is not available:
    /// two mints cannot share a destination, so recipient sets cannot recur
    /// across launches. Nothing built on this may imply a cabal identity it
    /// cannot see.
    pub recipients: Count,
    /// Transactions in the launch block that touched this mint.
    pub transactions: Count,
    /// What the creator's own buy cost, when there was one.
    ///
    /// `None` means no dev buy was found, which is **not** the same as a dev buy
    /// of zero and must never be rendered as one (rule 9).
    pub dev_buy_lamports: Option<u64>,
    /// Creator-supplied name, symbol and URI.
    pub metadata: Metadata,
}

/// The strings a creator chose, which are arbitrary bytes.
///
/// Held in its own type so that nothing can pass one of these where a trusted
/// string is expected without saying so. AGENTS.md rule 4: token metadata is
/// `Trust::Untrusted` no matter how authoritative it sounds — storable,
/// hashable, displayable and analysable as data, never an instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    /// The token's name.
    pub name: String,
    /// The token's symbol.
    pub symbol: String,
    /// The metadata URI. **Never fetched automatically**: doing so would let a
    /// stranger's mint choose an outbound request from Radar's server.
    pub uri: String,
}

impl Metadata {
    /// The trust class of everything in here, stated rather than assumed.
    #[must_use]
    pub const fn trust(&self) -> Trust {
        Trust::Untrusted
    }
}

/// Why a launch block could not be reconstructed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum NotALaunch {
    /// No pump.fun `create` or `create_v2` instruction was found.
    #[error("no pump.fun launch instruction in this transaction")]
    NoLaunchInstruction,
    /// The instruction was found but its arguments would not decode.
    #[error("launch instruction arguments were unreadable")]
    UnreadableArguments,
    /// The transaction failed, so nothing in it happened.
    #[error("the transaction failed")]
    Failed,
}

/// The pump.fun program, as the node spells it.
fn pumpfun_program() -> String {
    pumpfun::PROGRAM_ID.to_string()
}

/// Reads the launch out of the transaction that created the token.
///
/// # Errors
///
/// [`NotALaunch`] when the transaction failed, carries no launch instruction, or
/// carries one whose arguments will not decode.
pub fn launch_from(tx: &Transaction) -> Result<(Address, Metadata), NotALaunch> {
    if tx.failed {
        // A failed transaction is not an event. 0006 found 35 of 97 migration
        // instructions in one hour were in failed transactions, and counting
        // them overstated the label by more than a third.
        return Err(NotALaunch::Failed);
    }
    let program = pumpfun_program();

    for ix in &tx.instructions {
        if ix.program != program {
            continue;
        }
        // Matched on discriminator bytes, never on a logged instruction name --
        // LEARNINGS 3, an exact-match on a name that had been versioned. And
        // `is_launch` covers `create` and `create_v2` both: checking for one
        // silently drops the other, and a launch Radar never sees is a launch
        // Radar reports as absent.
        let decoded = radar_decode::decode_pumpfun(&ix.data);
        let radar_decode::Decoded::Known(instruction) = decoded else {
            continue;
        };
        if !instruction.is_launch() {
            continue;
        }
        let Some(parsed) = radar_decode::decode_pumpfun_launch(&ix.data) else {
            return Err(NotALaunch::UnreadableArguments);
        };
        let launch = parsed.map_err(|_| NotALaunch::UnreadableArguments)?;
        return Ok((
            launch.creator,
            Metadata {
                name: launch.name.to_owned(),
                symbol: launch.symbol.to_owned(),
                uri: launch.uri.to_owned(),
            },
        ));
    }
    Err(NotALaunch::NoLaunchInstruction)
}

/// Counts the distinct token accounts that received `mint` across a block.
///
/// An account received the token if its balance of that mint rose. Accounts
/// absent from `pre` are treated as having started at zero, which is correct
/// here and only here: a token account that did not exist before the block
/// genuinely held none of a mint that did not exist before the block. This is
/// the one place a missing value is a zero rather than an unknown, and it is
/// worth saying why rather than letting it look like an oversight.
#[must_use]
pub fn recipients_in(transactions: &[Transaction], mint: &str) -> u32 {
    let mut destinations: Vec<String> = Vec::new();

    for tx in transactions {
        if tx.failed {
            continue;
        }
        for post in tx.post_token_balances.iter().filter(|b| b.mint == mint) {
            let before = tx
                .pre_token_balances
                .iter()
                .find(|b| b.account_index == post.account_index && b.mint == mint)
                .map_or(0, |b| b.amount);
            if post.amount <= before {
                continue;
            }
            // Keyed on the token account, which is what the account index names
            // within this transaction. Two transactions in the same block name
            // the same account at different indices, so the index alone would
            // both over- and under-count; the address is the identity.
            let Some(address) = tx.accounts.get(post.account_index) else {
                continue;
            };
            if !destinations.iter().any(|d| d == address) {
                destinations.push(address.clone());
            }
        }
    }
    u32::try_from(destinations.len()).unwrap_or(u32::MAX)
}

/// What the creator spent buying their own token in the launch block.
///
/// Returns `None` rather than zero when no buy is found. The difference is the
/// whole of rule 9 here: "the creator did not buy" and "we could not see what
/// the creator bought" are different claims, and only one of them is safe to
/// publish as a fact about a person.
#[must_use]
pub fn dev_buy_lamports(transactions: &[Transaction], creator: &Address) -> Option<u64> {
    let program = pumpfun_program();
    let creator_key = creator.to_string();
    let mut total: Option<u64> = None;

    for tx in transactions.iter().filter(|t| !t.failed) {
        // The fee payer is the first account key, and a dev buy in the launch
        // block is signed by the creator.
        if tx.accounts.first() != Some(&creator_key) {
            continue;
        }
        for ix in tx.instructions.iter().filter(|i| i.program == program) {
            let radar_decode::Decoded::Known(instruction) = radar_decode::decode_pumpfun(&ix.data)
            else {
                continue;
            };
            if !instruction.is_buy() {
                continue;
            }
            let Some(Ok(trade)) = radar_decode::decode_pumpfun_trade(&ix.data) else {
                continue;
            };
            // `exact_lamports` is `None` for a token-exact buy, where the SOL figure
            // is a bound rather than an outcome. Publishing a bound as a spend
            // would be stating a number the chain does not carry, so a
            // token-exact dev buy contributes nothing and stays unknown.
            if let Some(lamports) = trade.exact_lamports() {
                total = Some(total.unwrap_or(0).saturating_add(lamports));
            }
        }
    }
    total
}

/// Assembles the launch block from the transactions that make it up.
///
/// `truncated` says whether the caller stopped fetching before it had them all;
/// it turns both counts into [`Count::AtLeast`], because a partial block
/// produces a partial count and publishing it as complete is how a budget ends
/// up deciding a refusal.
///
/// # Errors
///
/// [`NotALaunch`] when `launch_tx` is not a launch.
pub fn assemble(
    launch_tx: &Transaction,
    block: &[Transaction],
    mint: &str,
    truncated: bool,
) -> Result<LaunchBlock, NotALaunch> {
    let (creator, metadata) = launch_from(launch_tx)?;
    let recipients = recipients_in(block, mint);
    let transactions =
        u32::try_from(block.iter().filter(|t| !t.failed).count()).unwrap_or(u32::MAX);

    let wrap = if truncated {
        Count::AtLeast
    } else {
        Count::Exactly
    };

    Ok(LaunchBlock {
        slot: launch_tx.slot,
        creator,
        recipients: wrap(recipients),
        transactions: wrap(transactions),
        dev_buy_lamports: dev_buy_lamports(block, &creator),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{RawInstruction, TokenBalance};

    fn tx(accounts: &[&str]) -> Transaction {
        Transaction {
            slot: Slot(441_040_080),
            accounts: accounts.iter().map(|a| (*a).to_owned()).collect(),
            instructions: Vec::new(),
            pre_token_balances: Vec::new(),
            post_token_balances: Vec::new(),
            failed: false,
        }
    }

    fn balance(index: usize, mint: &str, amount: u64) -> TokenBalance {
        TokenBalance {
            account_index: index,
            mint: mint.to_owned(),
            amount,
            owner: None,
        }
    }

    #[test]
    fn a_balance_that_rose_is_a_recipient_and_one_that_did_not_is_not() {
        let mut a = tx(&["Payer", "GotSome", "SoldSome"]);
        a.post_token_balances = vec![balance(1, "M", 100), balance(2, "M", 5)];
        a.pre_token_balances = vec![balance(2, "M", 50)];
        assert_eq!(recipients_in(&[a], "M"), 1);
    }

    #[test]
    fn an_account_absent_from_pre_started_at_zero() {
        let mut a = tx(&["Payer", "Fresh"]);
        a.post_token_balances = vec![balance(1, "M", 1)];
        assert_eq!(recipients_in(&[a], "M"), 1);
    }

    #[test]
    fn a_recipient_seen_twice_in_a_block_counts_once() {
        // The same token account at a different index in a second transaction.
        // Keying on the index rather than the address both over- and
        // under-counts, and the over-count is the direction that would inflate
        // a coordination signal.
        let mut a = tx(&["Payer", "Wallet"]);
        a.post_token_balances = vec![balance(1, "M", 10)];
        let mut b = tx(&["Payer", "Other", "Wallet"]);
        b.post_token_balances = vec![balance(2, "M", 20)];
        b.pre_token_balances = vec![balance(2, "M", 10)];
        assert_eq!(recipients_in(&[a, b], "M"), 1);
    }

    #[test]
    fn another_mints_recipients_are_not_counted() {
        // A launch transaction moves the subject token and, at a graduation,
        // an LP mint too -- 0006 is the record of what happens when two mints
        // in one transaction are not told apart.
        let mut a = tx(&["Payer", "SubjectHolder", "OtherHolder"]);
        a.post_token_balances = vec![balance(1, "M", 10), balance(2, "OTHER", 10)];
        assert_eq!(recipients_in(&[a], "M"), 1);
    }

    #[test]
    fn a_failed_transaction_contributes_no_recipients() {
        let mut a = tx(&["Payer", "Wallet"]);
        a.post_token_balances = vec![balance(1, "M", 10)];
        a.failed = true;
        assert_eq!(recipients_in(&[a], "M"), 0);
    }

    #[test]
    fn a_transaction_with_no_launch_instruction_is_refused() {
        let mut a = tx(&["Payer"]);
        a.instructions = vec![RawInstruction {
            program: "SomeOtherProgram".to_owned(),
            data: vec![0; 8],
            accounts: Vec::new(),
        }];
        assert_eq!(launch_from(&a), Err(NotALaunch::NoLaunchInstruction));
    }

    #[test]
    fn a_failed_launch_transaction_is_refused_before_its_instructions_are_read() {
        let mut a = tx(&["Payer"]);
        a.failed = true;
        assert_eq!(launch_from(&a), Err(NotALaunch::Failed));
    }

    #[test]
    fn a_truncated_block_produces_truncated_counts() {
        // The property that keeps a budget from deciding a refusal: the same
        // block read completely and incompletely must not produce the same
        // claim.
        let mut launch = tx(&["Payer"]);
        launch.instructions = vec![RawInstruction {
            program: pumpfun_program(),
            data: launch_data(),
            accounts: Vec::new(),
        }];
        let mut member = tx(&["Payer", "Wallet"]);
        member.post_token_balances = vec![balance(1, "M", 10)];

        let whole = assemble(&launch, std::slice::from_ref(&member), "M", false).expect("a launch");
        let cut = assemble(&launch, std::slice::from_ref(&member), "M", true).expect("a launch");

        assert_eq!(whole.recipients, Count::Exactly(1));
        assert_eq!(cut.recipients, Count::AtLeast(1));
        assert!(!whole.recipients.is_truncated());
        assert!(cut.recipients.is_truncated());
        assert_eq!(whole.recipients.exact(), Some(1));
        assert_eq!(cut.recipients.exact(), None);
    }

    #[test]
    fn no_dev_buy_is_none_rather_than_zero() {
        // Rule 9. "The creator did not buy" and "we could not see what the
        // creator bought" are different claims about a person.
        let member = tx(&["Creator"]);
        let creator = Address::new([7u8; 32]);
        assert_eq!(dev_buy_lamports(&[member], &creator), None);
    }

    /// A `create` payload: the discriminator, then three length-prefixed
    /// strings, then the creator.
    fn launch_data() -> Vec<u8> {
        let mut data = vec![0x18, 0x1e, 0xc8, 0x28, 0x05, 0x1c, 0x07, 0x77];
        for s in ["Name", "SYM", "uri"] {
            data.extend_from_slice(&u32::try_from(s.len()).expect("short").to_le_bytes());
            data.extend_from_slice(s.as_bytes());
        }
        data.extend_from_slice(&[9u8; 32]);
        data
    }

    #[test]
    fn a_launch_yields_its_creator_and_its_untrusted_metadata() {
        let mut a = tx(&["Payer"]);
        a.instructions = vec![RawInstruction {
            program: pumpfun_program(),
            data: launch_data(),
            accounts: Vec::new(),
        }];
        let (creator, metadata) = launch_from(&a).expect("a launch");
        assert_eq!(creator, Address::new([9u8; 32]));
        assert_eq!(metadata.name, "Name");
        assert_eq!(metadata.symbol, "SYM");
        assert_eq!(metadata.uri, "uri");
        assert_eq!(metadata.trust(), Trust::Untrusted);
    }
}
