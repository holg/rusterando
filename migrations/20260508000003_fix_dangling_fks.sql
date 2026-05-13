-- Repair: when migration 20260508000001 ran via sqlx (which has
-- legacy_alter_table=ON behavior on some versions), the RENAME TO didn't
-- rewrite child FKs, leaving cart_items/order_items pointing at a now-dropped
-- "menu_items_old" table. This migration rebuilds those two tables with the
-- correct FK target.
--
-- For databases where the migration applied cleanly (FK already references
-- menu_items), the rebuild is still correct and harmless.

PRAGMA defer_foreign_keys = ON;

-- ---------- cart_items ----------------------------------------------------
CREATE TABLE cart_items_new (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL REFERENCES carts(id) ON DELETE CASCADE,
    menu_item_id TEXT NOT NULL REFERENCES menu_items(id),
    quantity INTEGER NOT NULL,
    options_json TEXT NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO cart_items_new SELECT * FROM cart_items;
DROP TABLE cart_items;
ALTER TABLE cart_items_new RENAME TO cart_items;
CREATE INDEX idx_cart_items_cart ON cart_items(cart_id);

-- ---------- order_items ---------------------------------------------------
CREATE TABLE order_items_new (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    menu_item_id TEXT NOT NULL REFERENCES menu_items(id),
    name_snapshot TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    options_json TEXT NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    line_total_cents INTEGER NOT NULL
);

INSERT INTO order_items_new SELECT * FROM order_items;
DROP TABLE order_items;
ALTER TABLE order_items_new RENAME TO order_items;
CREATE INDEX idx_order_items_order ON order_items(order_id);

-- ---------- menu_options --------------------------------------------------
-- Same risk; rebuild defensively.
CREATE TABLE menu_options_new (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES menu_items(id) ON DELETE CASCADE,
    group_name TEXT NOT NULL,
    label TEXT NOT NULL,
    price_delta_cents INTEGER NOT NULL DEFAULT 0,
    is_default INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0
);

INSERT INTO menu_options_new SELECT * FROM menu_options;
DROP TABLE menu_options;
ALTER TABLE menu_options_new RENAME TO menu_options;
CREATE INDEX idx_menu_options_item ON menu_options(item_id);
