// SPDX-License-Identifier: Apache-2.0
//! The fact sheet, and the bounded walk that fills it in.
//!
//! # What this is for
//!
//! Everything the public analyst may assert about one token, gathered from the
//! chain in a bounded number of calls, each figure carrying the slot it was read
//! at. Nothing here decides anything and nothing here is phrased: a
//! [`Dossier`] is *facts*, and Phase 2's verdict and voice sit on top of it.
//!
//! Keeping those apart is the same split `radar-signer` uses. The signer
//! re-reads the bytes it signs; the analyst re-reads the numbers it posts, and
//! it can only do that if the numbers exist as data before any sentence is
//! written about them.
//!
//! # Every field is optional, and that is the design
//!
//! A dossier for a mint whose curve could not be read is a dossier with no
//! curve, not a dossier with a curve of zero. AGENTS.md rule 9 — absent is not
//! zero, unknown is not safe — is why every fact below is an `Option` and why
//! [`Dossier::unavailable`] exists to say which ones are missing and why.
//! "Radar has no record" is a thing the analyst is expected to say plainly, and
//! it can only say it if the absence survives to the top.

use radar_pumpfun::curve::BondingCurve;
use radar_pumpfun::{Fees, pda};
use radar_types::{Address, Slot};

use crate::budget::{Budget, Count};
use crate::launch::{LaunchBlock, NotALaunch};
use crate::rpc::{RpcClient, RpcError, Transaction};

/// The impact budget capacity is measured at.
///
/// `Search::DEFAULT`'s 1%, so the number this reports is the same number the
/// rest of Radar reports. Research 0022 is why it must always be published *as*
/// a budget: the resulting figure is a Radar measurement at a chosen impact, and
/// **not a property of the token**. `STATE.md` and `GOAL.md` both described the
/// ~$31 it produces as a venue ceiling for weeks, and it is a setting.
pub const CAPACITY_IMPACT_BPS: u32 = 100;

/// A ceiling on the capacity search, in lamports.
///
/// Ten SOL. The search is a bisection and needs an upper bound; this one is far
/// above anything the curve supports pre-graduation, so it constrains the search
/// rather than the answer.
const CAPACITY_CEILING_LAMPORTS: u64 = 10_000_000_000;

/// Why a fact is missing.
///
/// Recorded per fact rather than as one global failure, because "the curve has
/// graduated" and "the endpoint timed out" lead to different replies and only
/// one of them is about the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    /// Which fact.
    pub fact: &'static str,
    /// Why, in a form safe to publish.
    pub why: String,
}

/// What the curve says right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurveFacts {
    /// Whether the curve has graduated to the AMM.
    ///
    /// A complete curve holds no reserves and prices nothing. One of three
    /// tokens sampled on 2026-09-01 was already complete, so this is the
    /// ordinary case rather than the exotic one.
    pub complete: bool,
    /// Lamports the curve actually holds.
    pub real_sol_reserves: u64,
    /// How much SOL can be spent before price moves by
    /// [`CAPACITY_IMPACT_BPS`].
    ///
    /// `None` means **cannot size into this at all**, never "no limit found"
    /// (rule 9). A complete curve is the common reason.
    pub capacity_lamports: Option<u64>,
    /// The venue fee, read from the on-chain schedule rather than assumed.
    ///
    /// Research 0023 measured it at 125 bps a side and found the program's own
    /// published IDL declares sixteen accounts for a buy where mainnet passes
    /// eighteen. A first-party reference is not the deployed program
    /// (LEARNINGS 25), so this is read from the chain every time.
    pub fees: Option<Fees>,
}

/// Everything the analyst may assert about one token.
#[derive(Clone, Debug)]
pub struct Dossier {
    /// The token asked about.
    pub mint: Address,
    /// The slot the read was taken at.
    ///
    /// **Every published figure carries this.** A number without the slot it was
    /// read at is unfalsifiable, and the account's entire claim is that its
    /// numbers can be checked on an explorer.
    pub read_at: Option<Slot>,
    /// What the launch block held.
    pub launch: Option<LaunchBlock>,
    /// What the curve says.
    pub curve: Option<CurveFacts>,
    /// How many successful transactions the creator's address has.
    ///
    /// **Transactions, not launches**, and named that way because the two are
    /// not the same number and the tempting one is the one this cannot measure.
    /// Counting launches from here would mean fetching and decoding every one of
    /// a prolific creator's transactions, which is exactly the unbounded work
    /// the budget exists to refuse.
    ///
    /// Always a [`Count::AtLeast`]: truncated when the walk hit its page bound,
    /// and still a lower bound when it did not, because a signature history is
    /// activity rather than launches. Phase 2 replaces this with the store's
    /// creator index -- 483,629 rows of `(creator, slot, mint)`, a hash lookup
    /// -- which is both cheaper and an actual launch count.
    pub creator_transactions: Option<Count>,
    /// Facts that could not be read, and why.
    pub unavailable: Vec<Unavailable>,
    /// RPC calls this dossier cost.
    pub calls: u32,
    /// How long it took, in milliseconds.
    pub elapsed_ms: u128,
}

impl Dossier {
    fn miss(&mut self, fact: &'static str, why: impl std::fmt::Display) {
        self.unavailable.push(Unavailable {
            fact,
            why: why.to_string(),
        });
    }
}

/// Builds a dossier for one mint.
///
/// Never returns `Err` for a fact it could not read — a partial dossier is the
/// product, and the missing halves are named in
/// [`Dossier::unavailable`]. It returns `Err` only when the mint itself cannot
/// be resolved, because a dossier about a token that does not exist is not a
/// partial answer, it is a wrong one.
///
/// # Errors
///
/// [`RpcError`] when the token's own signature history cannot be read at all.
pub fn build(client: &RpcClient, budget: &mut Budget, mint: &Address) -> Result<Dossier, RpcError> {
    let mint_key = mint.to_string();
    let mut dossier = Dossier {
        mint: *mint,
        read_at: None,
        launch: None,
        curve: None,
        creator_transactions: None,
        unavailable: Vec::new(),
        calls: 0,
        elapsed_ms: 0,
    };

    // 1. The launch block, from the oldest signature the mint has.
    let (signatures, truncated) = client.signatures_back_to_oldest(budget, mint)?;
    match oldest_launch(client, budget, &signatures, truncated, &mint_key) {
        Ok(block) => {
            dossier.read_at = Some(block.slot);
            dossier.launch = Some(block);
        }
        Err(why) => dossier.miss("launch block", why),
    }

    // 2. The curve, and the fee schedule it is priced under.
    match curve_facts(client, budget, mint) {
        Ok(facts) => dossier.curve = Some(facts),
        Err(why) => dossier.miss("curve", why),
    }

    // 3. The creator's activity, bounded.
    if let Some(creator) = dossier.launch.as_ref().map(|l| l.creator) {
        match client.signatures_back_to_oldest(budget, &creator) {
            Ok((sigs, _cut)) => {
                let n = u32::try_from(sigs.iter().filter(|s| s.err.is_none()).count())
                    .unwrap_or(u32::MAX);
                // `AtLeast` whether or not paging was cut short, and the reason
                // is not the budget: a signature history is transactions, and
                // the question anyone actually asks is about launches. Reporting
                // `Exactly` here would be exact about the wrong quantity, which
                // is LEARNINGS 22's shape -- an exemption whose reasoning named
                // the wrong number.
                dossier.creator_transactions = Some(Count::AtLeast(n));
            }
            Err(why) => dossier.miss("creator history", why),
        }
    }

    dossier.calls = budget.calls_made();
    dossier.elapsed_ms = budget.elapsed().as_millis();
    Ok(dossier)
}

/// Finds the launch transaction and rebuilds its block.
fn oldest_launch(
    client: &RpcClient,
    budget: &mut Budget,
    signatures: &[crate::rpc::SignatureInfo],
    truncated: bool,
    mint: &str,
) -> Result<LaunchBlock, String> {
    if truncated {
        // The oldest signature seen is not the oldest signature there is, so
        // reading it as the launch would invent a launch block out of an
        // ordinary trade. Refused rather than guessed -- AGENTS.md section 2:
        // when something is unknown, record it as unknown.
        return Err("this token has more history than the page budget allows, \
                    so its launch could not be reached"
            .to_owned());
    }
    let Some(oldest) = signatures.iter().rev().find(|s| s.err.is_none()) else {
        return Err("no successful transactions for this mint".to_owned());
    };

    let launch_slot = oldest.slot;
    let launch_tx = client
        .transaction(budget, &oldest.signature)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "the launch transaction could not be fetched".to_owned())?;

    // Every other transaction in the same slot that touched this mint. These
    // are the same-slot coordinated buys the recipient count is about.
    let mut block: Vec<Transaction> = vec![launch_tx.clone()];
    let mut stopped = false;
    for sig in signatures
        .iter()
        .filter(|s| s.slot == launch_slot && s.err.is_none() && s.signature != oldest.signature)
    {
        match client.transaction(budget, &sig.signature) {
            Ok(Some(tx)) => block.push(tx),
            Ok(None) => {}
            // Out of budget mid-block. The block is partial, so both counts
            // become "at least" rather than being published as complete.
            Err(RpcError::Stopped(_)) => {
                stopped = true;
                break;
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    crate::launch::assemble(&launch_tx, &block, mint, stopped)
        .map_err(|e: NotALaunch| e.to_string())
}

/// Reads the bonding curve and the fee schedule.
fn curve_facts(
    client: &RpcClient,
    budget: &mut Budget,
    mint: &Address,
) -> Result<CurveFacts, String> {
    let address = pda::bonding_curve(mint).ok_or_else(|| "no bonding-curve PDA".to_owned())?;
    let data = client
        .account(budget, &address)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no bonding-curve account: this is not a pump.fun token".to_owned())?;
    let curve = BondingCurve::parse(&data).map_err(|e| format!("{e:?}"))?;

    // A complete curve has no reserves. Asking it for capacity produces either a
    // division by zero or, worse, a plausible number out of stale fields -- so
    // the flag is checked before the arithmetic rather than after.
    let capacity = if curve.is_tradeable() {
        curve.buy_within_impact(CAPACITY_IMPACT_BPS, CAPACITY_CEILING_LAMPORTS)
    } else {
        None
    };

    Ok(CurveFacts {
        complete: curve.complete,
        real_sol_reserves: curve.real_sol_reserves,
        capacity_lamports: capacity,
        fees: fee_schedule(client, budget, &curve),
    })
}

/// Reads the fee schedule off the chain.
///
/// `None` rather than a constant when it cannot be read. 0023's whole finding is
/// that the fee is a *schedule* and that the program's published interface is
/// incomplete, so substituting a remembered 125 bps would be asserting a number
/// the chain was not asked for.
fn fee_schedule(client: &RpcClient, budget: &mut Budget, curve: &BondingCurve) -> Option<Fees> {
    let address = pda::fee_config()?;
    let data = client.account(budget, &address).ok()??;
    let config = radar_pumpfun::FeeConfig::parse(&data).ok()?;
    // The tier depends on where the token is on the curve, which is why this is
    // a schedule rather than a rate. Virtual SOL reserves are what
    // `the_fee_is_what_mainnet_charges` looks the tier up by -- 30 SOL, "a curve
    // at launch reserves" -- so this asks the schedule the same question the
    // crate's own mainnet test asks it.
    let market_cap = u128::from(curve.virtual_sol_reserves);
    config.fees_at_market_cap(market_cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dossier_names_what_it_could_not_read() {
        let mut dossier = Dossier {
            mint: Address::new([1u8; 32]),
            read_at: None,
            launch: None,
            curve: None,
            creator_transactions: None,
            unavailable: Vec::new(),
            calls: 0,
            elapsed_ms: 0,
        };
        dossier.miss("curve", "no bonding-curve account");
        assert_eq!(dossier.unavailable.len(), 1);
        assert_eq!(dossier.unavailable[0].fact, "curve");
        assert_eq!(dossier.unavailable[0].why, "no bonding-curve account");
        // And the fact itself stays absent rather than becoming a default.
        assert!(dossier.curve.is_none());
    }

    #[test]
    fn capacity_is_measured_at_the_budget_the_rest_of_radar_uses() {
        // 0022: the ~$31 figure this produces is the output of this setting, not
        // a property of the venue. If this constant and `Search::DEFAULT` drift
        // apart, the analyst and the kernel publish different numbers for the
        // same token.
        assert_eq!(CAPACITY_IMPACT_BPS, 100);
    }
}
