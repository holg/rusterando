-- The colour e-ink frame (Tagesangebot): which kitchen device drives it,
-- switched per device on /admin/drucker — no flashing, survives restarts
-- and power-off. Off for every device until switched on: the server sends
-- the picture's tag only to these devices' board feed (kitchen_board.rs).
ALTER TABLE kitchen_clients ADD COLUMN eink INTEGER NOT NULL DEFAULT 0;
