-- Push notification device tokens for the iOS staff app.
--
-- One row per (device, role) pair. A single physical device can
-- register multiple roles (e.g. David's iPad listening for both admin
-- and driver alerts) by calling /api/register_push_token once per role.
--
-- The role gates which server-side triggers fan out to which tokens:
--   - Admin   → payment failures, out-of-area attempts, daily totals.
--   - Kitchen → new orders, order changes, "fertig in 5 Min" warnings.
--   - Driver  → new tour starts, "Bestellung abholbereit" pings.
--
-- We also persist the platform up-front so an Android client could
-- register the same way (FCM tokens) without a schema change.

CREATE TABLE push_devices (
    token         TEXT PRIMARY KEY,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'kitchen', 'driver')),
    platform      TEXT NOT NULL CHECK (platform IN ('ios', 'android')),
    -- Free-text label so the admin UI can show "Davids iPad" / "Kitchen Tablet"
    -- when listing registered devices. Set by the app on registration.
    label         TEXT,
    -- Bumped every time the app re-registers (every launch). The server
    -- TTLs out devices that haven't checked in for ~30d to keep the list
    -- clean after a phone gets swapped.
    last_seen_at  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at    TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Apple flags some tokens as invalid via the APNs response
    -- ("Unregistered", "BadDeviceToken"). We mark them rather than
    -- deleting so we can tell "never registered" from "registered then
    -- invalidated" in admin diagnostics.
    invalidated_at TIMESTAMP
);

CREATE INDEX idx_push_devices_role  ON push_devices(role) WHERE invalidated_at IS NULL;
CREATE INDEX idx_push_devices_seen  ON push_devices(last_seen_at);
