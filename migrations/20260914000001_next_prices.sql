-- Staged price lists ("Neue Preise").
--
-- A shop raises prices in one go, but the new printed menu has to be at the
-- print shop BEFORE the online prices change. So the admin prepares the new
-- prices side-by-side with the live ones (next_* columns), renders
-- /menu.pdf?prices=next for the printer, and only later activates the
-- staged prices atomically. Every activation is journaled so it can be
-- reverted (and so the old prices are never lost).
--
-- The live columns (price_small_cents / price_large_cents / price_cents) are
-- untouched by staging; storefront, cart, checkout and SEO keep reading them.

ALTER TABLE menu_items ADD COLUMN next_price_small_cents INTEGER;
ALTER TABLE menu_items ADD COLUMN next_price_large_cents INTEGER;
ALTER TABLE pizza_extras ADD COLUMN next_price_cents INTEGER;

CREATE TABLE price_activations (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    activated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    reverted_at  TIMESTAMP,
    row_count    INTEGER NOT NULL DEFAULT 0
);

-- One row per price that changed in an activation: the old (pre-activation)
-- and new values. target_kind = 'item' (menu_items: small/large) or 'extra'
-- (pizza_extras: small only, large NULL).
CREATE TABLE price_activation_rows (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    activation_id INTEGER NOT NULL REFERENCES price_activations(id) ON DELETE CASCADE,
    target_kind   TEXT NOT NULL,
    target_id     TEXT NOT NULL,
    old_small     INTEGER,
    old_large     INTEGER,
    new_small     INTEGER,
    new_large     INTEGER
);
CREATE INDEX idx_price_activation_rows_act ON price_activation_rows(activation_id);
