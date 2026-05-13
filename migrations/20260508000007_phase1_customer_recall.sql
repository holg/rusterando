-- Phase 1: phone-based customer + address recall.
--
-- Design choice: introduce a `customers` table keyed by phone for guest
-- checkout. The pre-existing `users` table is kept untouched for the
-- future "logged-in customer" flow (it requires email + password_hash,
-- which doesn't fit anonymous checkout). Both can coexist; we'll bridge
-- them later if/when login lands.
--
-- The original `addresses` table is empty (never wired into code), so we
-- drop and rebuild it under a clearer name and with Phase-2 geo columns
-- already present (NULLABLE for now; populated once Nominatim lands).

PRAGMA defer_foreign_keys = ON;

CREATE TABLE customers (
    id             TEXT PRIMARY KEY,
    phone          TEXT NOT NULL UNIQUE,
    name           TEXT,
    email          TEXT,
    notes          TEXT,
    blacklisted_at TIMESTAMP,
    created_at     TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at   TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_customers_phone ON customers(phone);

DROP TABLE addresses;

CREATE TABLE customer_addresses (
    id                    TEXT PRIMARY KEY,
    customer_id           TEXT NOT NULL REFERENCES customers(id) ON DELETE CASCADE,

    street                TEXT NOT NULL,
    house_number          TEXT NOT NULL,
    postcode              TEXT NOT NULL,
    city                  TEXT NOT NULL,
    notes                 TEXT,

    -- Phase 2 geocoding output. NULL until Nominatim lands.
    latitude              REAL,
    longitude             REAL,
    geocoded_at           TIMESTAMP,
    geocode_provider      TEXT,
    suburb                TEXT,
    delivery_zone_id      TEXT REFERENCES delivery_zones(id),

    successful_deliveries INTEGER NOT NULL DEFAULT 0,
    failed_deliveries     INTEGER NOT NULL DEFAULT 0,

    last_used_at          TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    created_at            TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    UNIQUE (customer_id, street, house_number, postcode)
);

CREATE INDEX idx_customer_addresses_customer ON customer_addresses(customer_id);
CREATE INDEX idx_customer_addresses_zone     ON customer_addresses(delivery_zone_id);

-- Link orders to the recall tables. Both nullable: legacy/old orders won't
-- have them, and the JSON snapshot remains the source of truth for the
-- printed receipt + email body.
ALTER TABLE orders ADD COLUMN customer_id TEXT REFERENCES customers(id);
ALTER TABLE orders ADD COLUMN address_id  TEXT REFERENCES customer_addresses(id);

CREATE INDEX idx_orders_customer ON orders(customer_id);
CREATE INDEX idx_orders_address  ON orders(address_id);
