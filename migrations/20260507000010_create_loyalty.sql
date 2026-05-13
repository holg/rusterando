-- Stempelkarte: 10 orders ≥ 10 € → next pizza or salad free.
CREATE TABLE loyalty_stamps (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    stamps INTEGER NOT NULL DEFAULT 0,
    redeemable_count INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
