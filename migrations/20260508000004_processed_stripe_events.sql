-- Idempotency ledger for Stripe webhooks. Stripe retries failed deliveries
-- with backoff, so the same event id may arrive multiple times. Inserting
-- the id with ON CONFLICT DO NOTHING is our gate: if the row already
-- existed, we've handled this event before — return 200 OK without acting.

CREATE TABLE processed_stripe_events (
    id TEXT PRIMARY KEY,
    received_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
