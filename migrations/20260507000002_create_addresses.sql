CREATE TABLE addresses (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label TEXT,
    street TEXT NOT NULL,
    house_number TEXT NOT NULL,
    postcode TEXT NOT NULL,
    city TEXT NOT NULL,
    notes TEXT,
    is_default INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_addresses_user_id ON addresses(user_id);
