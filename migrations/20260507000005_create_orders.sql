CREATE TABLE orders (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id),
    order_number TEXT UNIQUE NOT NULL,
    order_type TEXT NOT NULL,            -- 'delivery' | 'pickup'
    status TEXT NOT NULL,
    contact_name TEXT NOT NULL,
    contact_phone TEXT NOT NULL,
    contact_email TEXT NOT NULL,
    delivery_address_json TEXT,          -- snapshot; NULL for pickup
    scheduled_for TIMESTAMP,             -- NULL = ASAP
    subtotal_cents INTEGER NOT NULL,
    delivery_fee_cents INTEGER NOT NULL DEFAULT 0,
    tax_cents INTEGER NOT NULL DEFAULT 0,
    total_cents INTEGER NOT NULL,
    stripe_payment_intent_id TEXT,
    payment_status TEXT NOT NULL,        -- 'pending' | 'paid' | 'failed' | 'refunded'
    notes TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_orders_user ON orders(user_id);
CREATE INDEX idx_orders_status ON orders(status);
CREATE INDEX idx_orders_created_at ON orders(created_at);

CREATE TABLE order_items (
    id TEXT PRIMARY KEY,
    order_id TEXT NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
    menu_item_id TEXT NOT NULL REFERENCES menu_items(id),
    name_snapshot TEXT NOT NULL,
    quantity INTEGER NOT NULL,
    options_json TEXT NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    line_total_cents INTEGER NOT NULL
);

CREATE INDEX idx_order_items_order ON order_items(order_id);
