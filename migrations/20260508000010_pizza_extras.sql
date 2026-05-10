-- Pizza extras (Tabasco, Extra Käse, Sucuk…). One shared catalog applied to
-- every pizza/calzone in the cart picker. Each cart_items / order_items row
-- snapshots the picked extras as JSON so price changes later don't rewrite
-- the customer's order history.
--
-- Tabasco starts at 0 € (free for now per David); the admin can edit any
-- extra's price + availability without a code change.

CREATE TABLE pizza_extras (
    id              TEXT PRIMARY KEY,
    label           TEXT NOT NULL,
    price_cents     INTEGER NOT NULL DEFAULT 100,
    is_available    INTEGER NOT NULL DEFAULT 1,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at      TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_pizza_extras_active ON pizza_extras(is_available, sort_order);

-- Snapshot of picked extras for each cart/order line. Stored as JSON so the
-- shape can grow (e.g. portion size) without another migration. Schema:
--   [{ "id": "ex-tabasco", "label": "Tabasco", "price_cents": 0 }, ...]
ALTER TABLE cart_items  ADD COLUMN extras_json TEXT;
ALTER TABLE order_items ADD COLUMN extras_json TEXT;

-- Seed the standard pizza-topping catalog. Sort order chosen so the most
-- common extras (Tabasco, Extra Käse) sit at the top of the picker.
INSERT INTO pizza_extras (id, label, price_cents, is_available, sort_order) VALUES
    ('ex-tabasco',      'Tabasco',         0,  1, 10),
    ('ex-extra-kaese',  'Extra Käse',     100, 1, 20),
    ('ex-extra-salami', 'Extra Salami',   100, 1, 30),
    ('ex-schinken',     'Schinken',       100, 1, 40),
    ('ex-pilze',        'Pilze',          100, 1, 50),
    ('ex-sucuk',        'Sucuk',          100, 1, 60),
    ('ex-peperoni',     'Peperoni',       100, 1, 70),
    ('ex-knoblauch',    'Knoblauch',      100, 1, 80),
    ('ex-zwiebeln',     'Zwiebeln',       100, 1, 90),
    ('ex-tomaten',      'Tomaten',        100, 1, 100),
    ('ex-artischocken', 'Artischocken',   100, 1, 110),
    ('ex-mais',         'Mais',           100, 1, 120),
    ('ex-oliven',       'Oliven',         100, 1, 130),
    ('ex-ananas',       'Ananas',         100, 1, 140),
    ('ex-thunfisch',    'Thunfisch',      100, 1, 150),
    ('ex-krabben',      'Krabben',        100, 1, 160),
    ('ex-lachs',        'Lachs',          200, 1, 170);
