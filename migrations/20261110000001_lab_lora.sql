-- LoRa lab traffic for the admin page's LoRa tab (/admin/lora).
-- The tab5-lab Tabs (crates/tab5-lab/src/report.rs) post every LoRa packet
-- they send, receive or miss an answer to (POST /api/lab/lora); the admin
-- page shows them as a chat and can queue messages that a Tab then sends
-- over LoRa.
-- Additive only: nothing existing is changed by this file.

-- One row per event as one Tab saw it. A message sent by one Tab and heard
-- by another appears twice: `tx` at the sender, `rx` at the receiver.
CREATE TABLE IF NOT EXISTS lab_lora_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    received_at INTEGER NOT NULL,          -- unix seconds, server clock
    device      TEXT    NOT NULL,          -- the reporting Tab, e.g. tab5-e2d5a3
    dir         TEXT    NOT NULL,          -- tx | rx | lost
    kind        TEXT    NOT NULL,          -- MSG | ACK | PING | PONG | RAW
    seq         INTEGER,                   -- the packet's sequence number (0–255)
    sender      TEXT,                      -- the sender named in the packet
    text        TEXT    NOT NULL DEFAULT '',
    rssi        INTEGER,                   -- signal at the reporting Tab (dBm)
    snr         REAL,                      -- (dB)
    peer_rssi   INTEGER,                   -- signal at the other end, from its ACK/PONG
    peer_snr    REAL,
    rtt_ms      INTEGER,                   -- MSG/PING → ACK/PONG round trip
    admin_id    INTEGER,                   -- lab_lora_outbox.id when typed on the admin page
    uptime_ms   INTEGER                    -- the Tab's uptime (it has no clock)
);
CREATE INDEX IF NOT EXISTS lab_lora_events_received ON lab_lora_events (received_at);

-- Messages typed on the admin page, waiting for their Tab to pick them up.
CREATE TABLE IF NOT EXISTS lab_lora_outbox (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at    INTEGER NOT NULL,        -- unix seconds
    device        TEXT    NOT NULL,        -- the Tab that should send it
    text          TEXT    NOT NULL,
    handed_out_at INTEGER                  -- when the Tab took it (NULL: waiting)
);
CREATE INDEX IF NOT EXISTS lab_lora_outbox_waiting ON lab_lora_outbox (device, handed_out_at);

-- The Tabs that report, and when they last asked (every few seconds while
-- on; written at most every 30 s per Tab): the admin page sends only
-- through a Tab that is there.
CREATE TABLE IF NOT EXISTS lab_lora_devices (
    device    TEXT    PRIMARY KEY,
    last_seen INTEGER NOT NULL             -- unix seconds
);
