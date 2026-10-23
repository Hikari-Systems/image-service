-- Index for the background transcode sweep's claim query.
--
-- The claim scans for rows that are eligible (no lease, or an expired one) and
-- incomplete, ordered by age. Without an index that is a full scan of `image` on
-- every pass, on every replica, forever — cheap at demo size and not at all cheap
-- once the table is real.
--
-- Partial, on the two predicates that actually narrow it:
--   * `downloaded_s3_path IS NOT NULL` — a row with no source cannot be
--     transcoded, so it can never be claimed and does not belong in the index.
--   * `original_s3_path IS NULL` — the dominant incomplete case now that uploads
--     defer the original. Rows that are fully transcoded drop out of the index
--     entirely, which is the point: the index shrinks as the backlog clears
--     rather than growing with the table.
--
-- `avoid_resize_until NULLS FIRST` matches the query's eligibility test (never
-- leased sorts before leased), and `created_at` matches its ORDER BY, so the
-- claim can walk the index in order and stop at LIMIT.
CREATE INDEX IF NOT EXISTS image_transcode_sweep_idx
    ON image (avoid_resize_until NULLS FIRST, created_at NULLS FIRST)
    WHERE downloaded_s3_path IS NOT NULL
      AND original_s3_path IS NULL;
