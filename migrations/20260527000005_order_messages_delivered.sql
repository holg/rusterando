-- Delivery acknowledgement for restaurant→customer messages.
--
-- When a customer's /orders/{id} page renders a message, it calls
-- `ack_order_message`, which stamps `delivered_at`. The admin's order
-- detail then shows "✓ Zugestellt" (delivered) vs "gesendet" (saved but
-- not yet shown in any open browser). NULL = not yet delivered.
--
-- Additive column (default NULL) — safe to apply to the live DB.
ALTER TABLE order_messages ADD COLUMN delivered_at TIMESTAMP;
