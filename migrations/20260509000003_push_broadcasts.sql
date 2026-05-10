-- Audit trail for admin-initiated push broadcasts. Each row is one
-- "fan out to every device" event triggered from /admin/broadcast.
--
-- Recipient count is captured at send-time so we know how many devices
-- the message *actually* went to (may differ from `SELECT COUNT(*) FROM
-- push_devices` if some tokens get invalidated mid-send).

CREATE TABLE push_broadcasts (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    body            TEXT NOT NULL,
    -- "admin" / "kitchen" / "driver" — who sent it (from current_role()).
    -- Always "admin" today since only the admin page exposes it, but
    -- recorded per-row so we can grow.
    sent_by_role    TEXT NOT NULL,
    -- Number of active push tokens at fan-out time. APNs delivery is
    -- best-effort beyond that.
    recipient_count INTEGER NOT NULL DEFAULT 0,
    sent_at         TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_push_broadcasts_sent_at ON push_broadcasts(sent_at DESC);
