-- Timestamped log of restaurant→customer messages for an order.
--
-- Replaces the single `orders.admin_message` column (which overwrote on
-- each send) with an append-only log, so the customer sees a running list
-- of updates with timestamps ("14:32 — braucht 10 Min länger"), shown
-- chat-style on /orders/{id} and pushed live via SSE.
--
-- `orders.admin_message` is left in place (harmless, unused going forward).
CREATE TABLE IF NOT EXISTS order_messages (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id   TEXT NOT NULL,
    body       TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_order_messages_order
    ON order_messages(order_id, created_at);
