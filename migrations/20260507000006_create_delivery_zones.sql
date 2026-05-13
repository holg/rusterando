CREATE TABLE delivery_zones (
    id TEXT PRIMARY KEY,
    postcode TEXT NOT NULL UNIQUE,
    fee_cents INTEGER NOT NULL,
    min_order_cents INTEGER NOT NULL DEFAULT 0,
    eta_minutes INTEGER NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1
);
