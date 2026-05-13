CREATE TABLE carts (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id),
    session_id TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_carts_session ON carts(session_id);
CREATE INDEX idx_carts_user ON carts(user_id);

CREATE TABLE cart_items (
    id TEXT PRIMARY KEY,
    cart_id TEXT NOT NULL REFERENCES carts(id) ON DELETE CASCADE,
    menu_item_id TEXT NOT NULL REFERENCES menu_items(id),
    quantity INTEGER NOT NULL,
    options_json TEXT NOT NULL,
    unit_price_cents INTEGER NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_cart_items_cart ON cart_items(cart_id);
