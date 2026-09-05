// SPDX-License-Identifier: Apache-2.0
//! `radar seven-days-later`: the join the analyst is not allowed to make.
//!
//! Design 0009 M4. The analyst daemon does not read the store -- its unit says
//! so, and the recorder's crash must never take the bot with it -- so the one
//! join the daily post needs runs here, on the box, on a timer, the way the
//! creator index is built: replies from the analyst's log, outcomes from the
//! store, written as one file the daemon reads and posts at noon UTC.
//!
//! The rows are for the day **seven days before** the run: what the bot was
//! asked about a week ago, and what the chain has done since.

use std::collections::BTreeMap;

use radar_analyst::daily::{Graduation, Row, Rows};
use radar_asof::AsOf;
use radar_store::{GraduationMode, Outcome};

/// Seconds in a day.
const DAY: u64 = 86_400;

/// Runs the command.
///
/// `--store <dir>`, `--analyst-dir <dir>` (default `data/analyst`), and
/// `--today <seconds>` for a test that wants a fixed clock.
///
/// # Errors
///
/// A message when the store cannot be read or the file cannot be written.
pub fn run(args: &[String]) -> Result<(), String> {
    let reader = crate::store_of(args)?;
    let analyst_dir =
        crate::flag(args, "--analyst-dir").unwrap_or_else(|| "data/analyst".to_owned());
    let today = match crate::flag(args, "--today") {
        Some(t) => t.parse::<u64>().map_err(|e| format!("--today: {e}"))?,
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };

    let log =
        radar_analyst::log::latest(&format!("{analyst_dir}/replies.jsonl")).unwrap_or_default();
    let watermark = reader
        .watermark()
        .map_err(|e| format!("cannot read the watermark: {e}"))?
        .ok_or("the store holds no events, so there is nothing to look back on")?;
    let outcomes = reader
        .read_outcomes(AsOf::at(watermark))
        .map_err(|e| format!("cannot read outcomes: {e}"))?;

    let rows = build(&log, &outcomes, today, watermark.get());
    let daily_dir = format!("{analyst_dir}/daily");
    std::fs::create_dir_all(&daily_dir).map_err(|e| format!("cannot create {daily_dir}: {e}"))?;
    let (path, _) =
        radar_analyst::daily::paths_for(&daily_dir, &radar_analyst::daily::date_of(today));
    rows.write(&path)
        .map_err(|e| format!("cannot write {path}: {e}"))?;
    println!(
        "{} replies from {} joined against {} outcomes at slot {}, written to {path}",
        rows.rows.len(),
        rows.asked_on,
        outcomes.len(),
        watermark.get()
    );
    Ok(())
}

/// The rows for the day seven days before `today`. Pure.
///
/// Only **published** X replies are looked back on: a dry-run answer was never
/// a public call and has nothing to age. The latest outcome per mint is used;
/// a mint the store has not measured gets a row with every field unknown,
/// which the post counts as a coin asked about and nothing else.
#[must_use]
pub fn build(
    log: &[radar_analyst::Entry],
    outcomes: &[Outcome],
    today: u64,
    watermark_slot: u64,
) -> Rows {
    let asked_day = (today / DAY).saturating_sub(7);
    let (from, to) = (asked_day * DAY, (asked_day + 1) * DAY);

    let mut latest: BTreeMap<String, &Outcome> = BTreeMap::new();
    for o in outcomes {
        let key = o.mint.to_string();
        if latest
            .get(&key)
            .is_none_or(|have| o.measured_at > have.measured_at)
        {
            latest.insert(key, o);
        }
    }

    let rows = log
        .iter()
        .filter(|e| e.at >= from && e.at < to && e.reply_id.is_some())
        .filter_map(|e| {
            let mint = e.mint.clone()?;
            let outcome = latest.get(&mint);
            Some(Row {
                graduation: outcome.and_then(|o| o.graduation_mode()).map(|m| match m {
                    GraduationMode::Instant => Graduation::Instant,
                    GraduationMode::Organic => Graduation::Organic,
                }),
                // No transfer at or after the slot the reply was read at. Both
                // sides must be known: a reply with no slot, or an outcome with
                // no transfer slot, is "cannot say", not "quiet".
                quiet_since_reply: match (
                    e.read_at_slot,
                    outcome.and_then(|o| o.last_transfer_slot),
                ) {
                    (Some(read), Some(last)) => Some(last.get() <= read),
                    _ => None,
                },
                held_bps: outcome.and_then(|o| o.held_to_end_gain_bps()),
                mint,
                asked_at: e.at,
                reply_id: e.reply_id.clone(),
            })
        })
        .collect();

    Rows {
        asked_on: radar_analyst::daily::date_of(from),
        built_at: today,
        watermark_slot,
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use radar_types::{Address, Slot};

    fn entry(
        mint: [u8; 32],
        at: u64,
        reply: Option<&str>,
        slot: Option<u64>,
    ) -> radar_analyst::Entry {
        radar_analyst::Entry {
            at,
            mention_id: format!("m-{}", mint[0]),
            summoner: "s".to_owned(),
            mint: Some(Address::new(mint).to_string()),
            read_at_slot: slot,
            fact_sheet: String::new(),
            reply: "measured".to_owned(),
            fellback: None,
            reply_id: reply.map(str::to_owned),
            signals: None,
        }
    }

    fn outcome(
        mint: [u8; 32],
        measured_at: u64,
        graduated_after: Option<u64>,
        last_transfer: Option<u64>,
        first: Option<u64>,
        last: Option<u64>,
    ) -> Outcome {
        Outcome {
            mint: Address::new(mint),
            measured_at: Slot(measured_at),
            launch_slot: Slot(1_000),
            first_transfer_slot: Some(Slot(1_000)),
            last_transfer_slot: last_transfer.map(Slot),
            transfers: 3,
            unique_senders: 2,
            unique_receivers: 2,
            graduated_at: graduated_after.map(|d| Slot(1_000 + d)),
            first_price: first,
            last_price: last,
            peak_price: None,
            trough_price: None,
            window_peak_price: None,
            window_trough_price: None,
            vwap: None,
            fills: 0,
        }
    }

    const TODAY: u64 = 20_701 * DAY + 11 * 3_600; // 2026-09-05 11:00 UTC

    #[test]
    fn the_rows_are_last_weeks_published_replies_joined_to_the_latest_outcome() {
        let week_ago = TODAY - 7 * DAY;
        let log = vec![
            entry([1; 32], week_ago, Some("r1"), Some(2_000)), // organic, quiet
            entry([2; 32], week_ago + 10, Some("r2"), Some(2_000)), // instant, traded since
            entry([3; 32], week_ago + 20, None, Some(2_000)),  // dry run: not public
            entry([4; 32], week_ago + 30, Some("r4"), None),   // no slot: cannot say quiet
            entry([5; 32], week_ago - DAY, Some("r5"), Some(2_000)), // eight days ago
            entry([6; 32], TODAY - DAY, Some("r6"), Some(2_000)), // yesterday
            // The day's edges: the first second of the next day is out, the
            // last second of the day is in. Re-applied `<` as `<=` and r7 is
            // counted.
            entry([7; 32], (week_ago / DAY + 1) * DAY, Some("r7"), Some(2_000)),
            entry(
                [8; 32],
                (week_ago / DAY + 1) * DAY - 1,
                Some("r8"),
                Some(2_000),
            ),
        ];
        let outcomes = vec![
            outcome([1; 32], 5_000, Some(900), Some(1_900), Some(100), Some(50)),
            // Two measurements of mint 2: the later one has the later transfer.
            outcome([2; 32], 4_000, Some(1), Some(1_500), Some(100), Some(120)),
            outcome([2; 32], 6_000, Some(1), Some(2_500), Some(100), Some(130)),
            outcome([4; 32], 5_000, None, Some(1_700), None, None),
        ];
        let rows = build(&log, &outcomes, TODAY, 6_000);
        assert_eq!(rows.asked_on, "2026-08-29");
        assert_eq!(rows.watermark_slot, 6_000);
        let mints: Vec<&str> = rows
            .rows
            .iter()
            .map(|r| r.reply_id.as_deref().unwrap_or(""))
            .collect();
        assert_eq!(mints, ["r1", "r2", "r4", "r8"], "{rows:?}");

        let r1 = &rows.rows[0];
        assert_eq!(r1.graduation, Some(Graduation::Organic));
        assert_eq!(r1.quiet_since_reply, Some(true));
        assert_eq!(r1.held_bps, Some(-5_000));

        let r2 = &rows.rows[1];
        assert_eq!(r2.graduation, Some(Graduation::Instant));
        assert_eq!(
            r2.quiet_since_reply,
            Some(false),
            "the later measurement wins"
        );
        assert_eq!(r2.held_bps, Some(3_000));

        let r4 = &rows.rows[2];
        assert_eq!(r4.graduation, None);
        assert_eq!(r4.quiet_since_reply, None, "no reply slot: cannot say");
        assert_eq!(r4.held_bps, None);
    }

    #[test]
    fn a_mint_the_store_never_measured_is_a_row_with_nothing_known() {
        // Re-applied by dropping unmeasured mints: the row disappears and the
        // day under-counts what the bot was asked about.
        let week_ago = TODAY - 7 * DAY;
        let log = vec![entry([9; 32], week_ago, Some("r9"), Some(2_000))];
        let rows = build(&log, &[], TODAY, 1);
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0].graduation, None);
        assert_eq!(rows.rows[0].quiet_since_reply, None);
        assert_eq!(rows.rows[0].held_bps, None);
    }
}
