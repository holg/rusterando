-- Generic key/value settings the admin can tune from the UI without
-- a code deploy. v1 use case: free-delivery threshold. Future use:
-- max-orders-per-hour, holiday-mode banner, etc.
--
-- Values are stored as TEXT for flexibility (cents go in as decimal
-- strings). Per-key parsing happens in Rust where each setting is
-- read.

CREATE TABLE app_settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    label_de   TEXT NOT NULL,
    -- Free-text help shown next to the field in /admin/settings.
    hint_de    TEXT,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO app_settings (key, value, label_de, hint_de) VALUES
    ('free_delivery_threshold_cents', '3500',
     'Kostenlose Lieferung ab',
     'Bestellwert in Cent. Bestellungen ab dieser Summe werden ohne Lieferzuschlag berechnet. 0 = nie kostenlos.');
