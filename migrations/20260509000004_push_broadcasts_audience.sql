-- Track which audience each /admin/broadcast went to. Two values today:
--   'staff' — admin + kitchen + driver only (operational pings)
--   'all'   — every active token regardless of role (staff + customers)
-- Existing rows predate the split; backfill them as 'staff' since that
-- was the only audience the page actually had at the time.

ALTER TABLE push_broadcasts
    ADD COLUMN audience TEXT NOT NULL DEFAULT 'staff';
