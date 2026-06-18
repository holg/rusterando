-- Per-shop kitchen-printer (Pi) state, so the admin can SEE which version
-- each Pi runs vs. the server's target — turning the version-skew failure
-- (a 0.1.0 Pi silently not printing against a 1.0 server) into something
-- visible, and gating the in-band self-update.
--
-- One row per shop_slug. Upserted on every Hello.
CREATE TABLE IF NOT EXISTS kitchen_clients (
    shop_slug       TEXT PRIMARY KEY,
    -- Running binary version the Pi reported (CARGO_PKG_VERSION).
    version         TEXT NOT NULL DEFAULT '',
    -- Target triple the Pi binary was built for, e.g.
    -- 'aarch64-unknown-linux-gnu'. Picks the matching update artifact.
    arch            TEXT NOT NULL DEFAULT '',
    -- Unix seconds of the last Hello/heartbeat — "last seen".
    last_seen_at    INTEGER NOT NULL DEFAULT 0,
    -- Admin-gated update: when 1, the server offers the current target
    -- binary on the Pi's next Hello. Cleared on a successful UpdateResult.
    update_armed    INTEGER NOT NULL DEFAULT 0,
    -- Last self-update outcome, for the admin panel.
    last_update_to       TEXT NOT NULL DEFAULT '',
    last_update_ok       INTEGER NOT NULL DEFAULT 1,
    last_update_error    TEXT NOT NULL DEFAULT '',
    last_update_at       INTEGER NOT NULL DEFAULT 0
);
