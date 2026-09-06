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
    /// Who launched the token, read from the curve account itself.
    ///
    /// **The only way to get a creator for a graduated coin.** The launch
    /// block is the other route, and `oldest_launch` refuses it when the
    /// signature walk truncates -- which it does for any coin with real
    /// history, so exactly the coins people ask about. This account carries it
    /// regardless of age, and the dossier already reads it.
    pub creator: Address,
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
        Ok((facts, slot)) => {
            // The curve read is the *only* slot a graduated coin has, because
            // its launch block is past the signature-page budget and
            // `oldest_launch` rightly refuses to guess one. Without this the
            // coins people actually ask about published every figure with no
            // slot beside it -- unfalsifiable, which is the one thing this
            // account may not be. Set only when the launch block did not
            // already supply one, so the earlier read still wins.
            if dossier.read_at.is_none() {
                dossier.read_at = slot;
            }
            dossier.curve = Some(facts);
        }
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
        .filter(|s| also_in_block(s, launch_slot, &oldest.signature))
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

/// Whether a signature belongs to the launch block, other than the launch
/// itself.
///
/// Extracted from the loop so each of its three conditions can be tested. All
/// three were surviving mutants: `just mutants` flipped `&&` to `||`, `==` to
/// `!=` and `!=` to `==` here and every test still passed, which meant the
/// filter was doing nothing any assertion could see.
///
/// Each condition is load-bearing in a different way. The **slot** is what makes
/// this the launch *block* rather than the token's whole history. The **error**
/// check keeps failed transactions out, which 0006 records as worth a third of
/// a label. And excluding the **launch signature** stops the launch transaction
/// being fetched and counted twice.
fn also_in_block(sig: &crate::rpc::SignatureInfo, launch_slot: u64, launch_sig: &str) -> bool {
    sig.slot == launch_slot && sig.err.is_none() && sig.signature != launch_sig
}

/// Reads the bonding curve and the fee schedule.
fn curve_facts(
    client: &RpcClient,
    budget: &mut Budget,
    mint: &Address,
) -> Result<(CurveFacts, Option<Slot>), String> {
    let address = pda::bonding_curve(mint).ok_or_else(|| "no bonding-curve PDA".to_owned())?;
    let read = client
        .account(budget, &address)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no bonding-curve account: this is not a pump.fun token".to_owned())?;
    let curve = BondingCurve::parse(&read.data).map_err(|e| format!("{e:?}"))?;

    // A complete curve has no reserves. Asking it for capacity produces either a
    // division by zero or, worse, a plausible number out of stale fields -- so
    // the flag is checked before the arithmetic rather than after.
    let capacity = if curve.is_tradeable() {
        curve.buy_within_impact(CAPACITY_IMPACT_BPS, CAPACITY_CEILING_LAMPORTS)
    } else {
        None
    };

    Ok((
        CurveFacts {
            complete: curve.complete,
            real_sol_reserves: curve.real_sol_reserves,
            capacity_lamports: capacity,
            creator: curve.creator,
            fees: fee_schedule(client, budget, &curve),
        },
        read.slot,
    ))
}

/// Reads the fee schedule off the chain.
///
/// `None` rather than a constant when it cannot be read. 0023's whole finding is
/// that the fee is a *schedule* and that the program's published interface is
/// incomplete, so substituting a remembered 125 bps would be asserting a number
/// the chain was not asked for.
fn fee_schedule(client: &RpcClient, budget: &mut Budget, curve: &BondingCurve) -> Option<Fees> {
    let address = pda::fee_config()?;
    let data = client.account(budget, &address).ok()??.data;
    let config = radar_pumpfun::FeeConfig::parse(&data).ok()?;
    fees_for(&config, curve)
}

/// Which tier of the schedule this curve pays.
///
/// The pure half of [`fee_schedule`], split out so it can be tested — the outer
/// function is three network calls deep and mutating it to return `None` or a
/// default survived, because nothing without an endpoint could observe it.
///
/// The tier depends on where the token sits on the curve, which is the whole of
/// 0023's finding: **the fee is a schedule, not a rate.** Virtual SOL reserves
/// are what `radar-pumpfun`'s own mainnet test looks the tier up by — 30 SOL,
/// "a curve at launch reserves" — so this asks the schedule the same question
/// that test asks it.
///
/// `None` when no tier covers the curve, and never a remembered 125 bps: 0023
/// also found the program's published interface incomplete, so substituting a
/// constant here would be asserting a number the chain was not asked for.
fn fees_for(config: &radar_pumpfun::FeeConfig, curve: &BondingCurve) -> Option<Fees> {
    config.fees_at_market_cap(u128::from(curve.virtual_sol_reserves))
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

    fn sig(signature: &str, slot: u64, failed: bool) -> crate::rpc::SignatureInfo {
        crate::rpc::SignatureInfo {
            signature: signature.to_owned(),
            slot,
            err: failed.then(|| serde_json::json!("boom")),
        }
    }

    #[test]
    fn the_launch_block_filter_needs_all_three_of_its_conditions() {
        // Every one of these was a surviving mutant. Asserted separately so a
        // failure names which condition stopped working rather than only that
        // the filter did.
        assert!(also_in_block(&sig("other", 100, false), 100, "launch"));
        // A different slot is a different block -- this is what keeps the read
        // to the launch block rather than the token's whole history.
        assert!(!also_in_block(&sig("other", 101, false), 100, "launch"));
        // A failed transaction is not an event (0006).
        assert!(!also_in_block(&sig("other", 100, true), 100, "launch"));
        // The launch itself is already in the block; including it again would
        // fetch and count it twice.
        assert!(!also_in_block(&sig("launch", 100, false), 100, "launch"));
    }

    fn curve_at(virtual_sol: u64) -> BondingCurve {
        BondingCurve {
            virtual_token_reserves: 889_566_950_293_959,
            virtual_sol_reserves: virtual_sol,
            real_token_reserves: 609_666_950_293_959,
            real_sol_reserves: 6_186_150_833,
            token_total_supply: 1_000_000_000_000_000,
            complete: false,
            creator: Address::new([1u8; 32]),
        }
    }

    #[test]
    fn the_fee_comes_from_the_tier_the_curve_is_in() {
        // Two tiers with different fees, so the lookup has something to get
        // wrong. Returning `None` or a default here survived mutation because
        // the only caller is three network calls deep.
        let config = radar_pumpfun::FeeConfig {
            flat: Fees {
                lp_bps: 0,
                protocol_bps: 0,
                creator_bps: 0,
            },
            tiers: vec![
                radar_pumpfun::fees::Tier {
                    threshold_lamports: 0,
                    fees: Fees {
                        lp_bps: 0,
                        protocol_bps: 95,
                        creator_bps: 30,
                    },
                },
                radar_pumpfun::fees::Tier {
                    threshold_lamports: 100_000_000_000,
                    fees: Fees {
                        lp_bps: 0,
                        protocol_bps: 10,
                        creator_bps: 5,
                    },
                },
            ],
        };

        // A curve at launch reserves pays the low tier: 125 bps a side, which
        // is exactly what 0023 measured off mainnet.
        let low = fees_for(&config, &curve_at(30_130_000_000)).expect("a tier");
        assert_eq!(low.total_bps(), 125);
        assert_eq!(low.round_trip_bps(), 250);

        // A curve further along pays the other one, so the schedule is being
        // read rather than the first row returned.
        let high = fees_for(&config, &curve_at(200_000_000_000)).expect("a tier");
        assert_eq!(high.total_bps(), 15);
        assert_ne!(low, high);
    }

    /// A fee-config account, built to the layout `FeeConfig::parse` reads.
    ///
    /// Synthesised rather than captured, because what is under test here is
    /// that [`fee_schedule`] reads the chain at all — `radar-pumpfun` already
    /// asserts the parse against real mainnet bytes, and duplicating that
    /// capture would be two places to update when the layout moves.
    fn fee_config_account(threshold: u128, protocol_bps: u64, creator_bps: u64) -> Vec<u8> {
        let mut data = radar_pumpfun::fees::FEE_CONFIG_DISCRIMINATOR.to_vec();
        data.push(0); // bump
        data.extend_from_slice(&[0u8; 32]); // admin
        // The flat fees, three u64.
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes()); // one tier
        data.extend_from_slice(&threshold.to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes()); // lp
        data.extend_from_slice(&protocol_bps.to_le_bytes());
        data.extend_from_slice(&creator_bps.to_le_bytes());
        data
    }

    /// A transport that answers every call with the same body.
    struct Always(String);

    impl crate::rpc::Transport for Always {
        fn post(&self, _: &str, _: String) -> Result<String, String> {
            Ok(self.0.clone())
        }
    }

    fn account_response(data: &[u8]) -> String {
        // The node returns account data base64-encoded.
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::new();
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    encoded.push(ALPHABET[((n >> (18 - i * 6)) & 0x3F) as usize] as char);
                } else {
                    encoded.push('=');
                }
            }
        }
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"value":{{"data":["{encoded}","base64"]}}}}}}"#
        )
    }

    #[test]
    fn the_fee_schedule_is_read_from_the_chain_rather_than_assumed() {
        // Mutating `fee_schedule` to `None` or a default survived until the
        // transport became injectable. Both consequences are bad in opposite
        // directions: `None` silently drops the fee from every reply, and a
        // default asserts a fee nobody read.
        let response = account_response(&fee_config_account(0, 95, 30));
        let client = RpcClient::with_transport("http://test.invalid", Box::new(Always(response)));
        let mut budget = Budget::new(60, 3, std::time::Duration::from_secs(30));

        let fees =
            fee_schedule(&client, &mut budget, &curve_at(30_130_000_000)).expect("a fee schedule");
        // 125 bps a side is what 0023 measured off mainnet.
        assert_eq!(fees.total_bps(), 125);
        assert_eq!(fees.round_trip_bps(), 250);
    }

    #[test]
    fn a_missing_fee_config_account_is_no_fee_rather_than_a_remembered_one() {
        // Rule 9 through the whole path: a fee that could not be read is a
        // refusal to price, never a constant recalled from a research note.
        let client = RpcClient::with_transport(
            "http://test.invalid",
            Box::new(Always(
                r#"{"jsonrpc":"2.0","id":1,"result":{"value":null}}"#.to_owned(),
            )),
        );
        let mut budget = Budget::new(60, 3, std::time::Duration::from_secs(30));
        assert_eq!(
            fee_schedule(&client, &mut budget, &curve_at(30_130_000_000)),
            None
        );
    }

    #[test]
    fn a_schedule_that_covers_nothing_is_no_fee_rather_than_a_free_trade() {
        // Rule 9. A fee that could not be read is a refusal to price, never a
        // zero -- a trade priced at no fee is a trade that looks profitable.
        let config = radar_pumpfun::FeeConfig {
            flat: Fees {
                lp_bps: 0,
                protocol_bps: 0,
                creator_bps: 0,
            },
            tiers: Vec::new(),
        };
        assert_eq!(fees_for(&config, &curve_at(30_130_000_000)), None);
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
