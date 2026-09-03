-- SPDX-License-Identifier: Apache-2.0
--
-- 0024: distinct recipients inside a pump.fun launch block, for every launch in
-- a window.
--
-- Carried rather than described, for the reason 0013 and 0019 give: a note whose
-- query exists only in prose is one nobody can re-run, and re-running is the only
-- way a number in a note gets checked. 0019 exists *because* the measurement
-- behind `assumed_round_trip_bps = 850` could not be checked. 0008 -- the note
-- this one re-measures -- described its query and did not carry it, which is why
-- re-running it meant rebuilding it from the note's prose.
--
-- # What changed from 0008, and why it matters
--
-- 0008 sampled eighty launches per population and took the launch block as "the
-- slot of a mint's first observed token transfer". That heuristic is wrong in a
-- way that inflates the population by about four times: a token launched days
-- earlier whose first transfer *inside the window* falls in some later slot is
-- counted as a launch in that slot, and its "launch block" is an ordinary
-- trading block. Measured over the same hour, 2026-09-02 00:30-01:30:
--
--   first-transfer heuristic, any mint          11,019
--   first-transfer heuristic, mints ending pump  5,285
--   create/create_v2 discriminator join          1,928
--
-- 0006 measured the store's recorded launch rate at 1,283/hour against a chain
-- rate of 1,328/hour on 2026-08-23. The discriminator join is the only one of
-- the three in that neighbourhood.
--
-- So this query identifies a launch the way the recorder does: a transaction
-- carrying the pump.fun `create` or `create_v2` **discriminator bytes**, never a
-- logged instruction name (LEARNINGS 3), and never a first-seen heuristic.
--
--   create     181ec828051c0777
--   create_v2  d6904cec5f8b31b4
--
-- Both, not just `create_v2`: `radar_decode::pumpfun::Instruction::is_launch`
-- matches both, and checking for one silently drops the other.
--
-- # Recipients, not buyers
--
-- `destination` is a **token account**, an `(owner, mint)` pair. This counts
-- distinct token accounts that received the token inside its own launch block.
-- Resolving them to owners is a join this does NOT do, and 0012 shows why the
-- obvious follow-up is not available: two mints cannot share a destination, so
-- recipient sets cannot recur across launches. Nothing built on this number may
-- imply a cabal identity it cannot see.
--
-- # Why it is sliced by time rather than paged
--
-- `solana.token_transfers` prunes on `block_timestamp` and nothing else
-- (ADR 0002), so a mint filter over a wide window does not prune and the public
-- endpoint's 60-second limit kills it -- measured: a 200-mint `IN` list over
-- fifteen days times out. Deep `OFFSET` paging re-runs the whole query per page
-- and times out by the third. A 20-minute slice returns 600-900 rows, inside the
-- endpoint's thousand-row cap, and each one is a single bounded query.
--
-- **If a slice ever returns 1,000 rows it has been truncated silently** -- treat
-- that as a failure and shrink the slice, do not read the histogram. LEARNINGS 5
-- and 26: a check must fail differently when it did not run than when it found
-- nothing.
--
-- Ran in about 75 seconds per two hours of chain against the public endpoint on
-- 2026-09-03, with `user=crypto`.
--
-- Substitute the slice bounds for :FROM and :TO. Graduation labels are NOT in
-- this query: they come from Radar's own store, joined on the mint afterwards,
-- because CryptoHouse cannot distinguish an instant graduation from an organic
-- one without re-implementing the resolver 0006 fixed.

WITH
launch_tx AS (
  SELECT DISTINCT tx_signature
  FROM solana.instructions
  WHERE program_id = '6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P'
    AND block_timestamp >= :FROM AND block_timestamp < :TO
    AND lower(hex(substring(base58Decode(data), 1, 8)))
        IN ('181ec828051c0777', 'd6904cec5f8b31b4')
),
launched AS (
  -- The subject mint of each launch, and the slot it launched in. The quote
  -- assets are excluded because counting recipients of wrapped SOL would measure
  -- the market rather than the token.
  SELECT t.mint AS m, min(t.block_slot) AS launch_slot
  FROM solana.token_transfers t
  INNER JOIN launch_tx x ON t.tx_signature = x.tx_signature
  WHERE t.block_timestamp >= :FROM AND t.block_timestamp < :TO
    AND t.mint != ''
    AND t.mint NOT IN ('So11111111111111111111111111111111111111112',
                       'So11111111111111111111111111111111111111111')
  GROUP BY t.mint
)
SELECT l.m AS m, uniqExact(t.destination) AS recipients
FROM solana.token_transfers t
INNER JOIN launched l ON t.mint = l.m AND t.block_slot = l.launch_slot
WHERE t.block_timestamp >= :FROM AND t.block_timestamp < :TO
GROUP BY l.m
ORDER BY l.m
