-- Saved price templates ("Vorlagen") for /admin/next-prices.
--
-- Working out a new price list takes a while (print shop, back and forth),
-- and there is only one set of staged prices (the next_* columns), which
-- "Verwerfen" wipes. A template is a named copy of the staged prices at the
-- moment it was saved; loading one puts its prices back as the staged ones.
-- Activation and its journal (price_activations) are unchanged.

CREATE TABLE price_templates (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    row_count  INTEGER NOT NULL DEFAULT 0
);

-- One staged price per row, as in price_activation_rows: target_kind =
-- 'item' (menu_items: small/large) or 'extra' (pizza_extras: small only).
CREATE TABLE price_template_rows (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id INTEGER NOT NULL REFERENCES price_templates(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL,
    target_id   TEXT NOT NULL,
    small_cents INTEGER,
    large_cents INTEGER
);
CREATE INDEX idx_price_template_rows_tpl ON price_template_rows(template_id);
