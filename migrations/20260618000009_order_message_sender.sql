-- Two-way order chat: a message now records WHO sent it. Until now every
-- row was an admin→customer message, so existing rows default to 'admin'
-- (correct retroactively). Values: 'admin' | 'kitchen' | 'driver' (staff)
-- and 'customer' (VIP reply). The thread is shared per order; the sender
-- drives attribution + alignment in every view.
ALTER TABLE order_messages ADD COLUMN sender TEXT NOT NULL DEFAULT 'admin';
