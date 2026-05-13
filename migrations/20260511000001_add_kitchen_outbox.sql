-- Kitchen channel outbox. Persists every KitchenEvent (postcard-encoded
-- in `payload`) until the printer client has acknowledged it. Survives
-- server restart; on Pi reconnect, anything with `acked_at IS NULL` and
-- id > the Pi's last_seen_seq is replayed.
--
-- Only populated when KITCHEN_LISTEN_ADDR is set in .env — printerless
-- deploys (rusterando demo, etc.) never insert here. See
-- crates/rusterando-server/src/kitchen.rs.

CREATE TABLE IF NOT EXISTS kitchen_outbox (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    payload     BLOB    NOT NULL,         -- postcard-encoded KitchenEvent
    created_at  INTEGER NOT NULL,         -- unix seconds
    acked_at    INTEGER                   -- unix seconds; NULL = unacked
);

-- Partial index over unacked rows — the only query path is "give me
-- everything still pending, ordered by id". Acked rows live forever
-- (or until GC trims them) for audit but never need an index scan.
CREATE INDEX IF NOT EXISTS idx_kitchen_outbox_unacked
    ON kitchen_outbox (id)
    WHERE acked_at IS NULL;
