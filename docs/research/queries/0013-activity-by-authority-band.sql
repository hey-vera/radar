-- SPDX-License-Identifier: Apache-2.0
--
-- 0013: later activity by launch-block authority band.
--
-- Kept because a research note that describes a query and does not carry it is
-- one nobody can re-run, and re-running is the only way a number in a note gets
-- checked. LEARNINGS 1 is the record of a design documented as canonical while
-- its source existed nowhere.
--
-- Window B in the note is this statement with every interval shifted back by
-- 180 minutes: 150 -> 330, 140 -> 320, 60 -> 240.
--
-- Ran in under nine seconds against the public endpoint on 2026-08-30.

WITH
first_slots AS (
  SELECT mint AS m, min(block_slot) AS launch_slot, min(block_timestamp) AS t0
  FROM solana.token_transfers
  WHERE block_timestamp >= now() - INTERVAL 150 MINUTE AND mint != ''
  GROUP BY mint
),
launched AS (
  SELECT m, launch_slot, t0 FROM first_slots
  WHERE t0 >= now() - INTERVAL 140 MINUTE AND t0 <= now() - INTERVAL 60 MINUTE
),
prevalence AS (
  SELECT t.authority AS a, uniqExact(t.mint) AS launch_blocks
  FROM solana.token_transfers t
  INNER JOIN first_slots f ON t.mint = f.m AND t.block_slot = f.launch_slot
  WHERE t.block_timestamp >= now() - INTERVAL 150 MINUTE AND t.authority != ''
  GROUP BY t.authority
),
per_launch AS (
  SELECT l.m AS mint,
         max(multiIf(p.launch_blocks >= 100, 0, p.launch_blocks >= 3, 2, 1)) AS band
  FROM launched l
  INNER JOIN solana.token_transfers t ON t.mint = l.m AND t.block_slot = l.launch_slot
  LEFT JOIN prevalence p ON t.authority = p.a
  WHERE t.block_timestamp >= now() - INTERVAL 150 MINUTE AND t.authority != ''
  GROUP BY l.m
),
later AS (
  SELECT t.mint AS mint, count() AS transfers, uniqExact(t.authority) AS participants
  FROM solana.token_transfers t
  INNER JOIN launched l ON t.mint = l.m
  WHERE t.block_timestamp >= now() - INTERVAL 150 MINUTE AND t.block_slot > l.launch_slot
  GROUP BY t.mint
)
SELECT
  multiIf(band = 2, 'repeat', band = 0, 'infrastructure', 'ordinary') AS cohort,
  count() AS launches,
  round(avg(coalesce(later.transfers, 0)), 1) AS mean_later_transfers,
  median(coalesce(later.transfers, 0)) AS median_later_transfers,
  round(100 * countIf(coalesce(later.transfers, 0) = 0) / count(), 1) AS pct_dead_after_launch_block,
  round(100 * countIf(coalesce(later.participants, 0) >= 10) / count(), 1) AS pct_ten_plus_participants
FROM per_launch LEFT JOIN later ON per_launch.mint = later.mint
GROUP BY cohort ORDER BY launches DESC FORMAT TSV
