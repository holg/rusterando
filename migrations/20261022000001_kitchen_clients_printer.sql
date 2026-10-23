-- Kitchen devices that print only while a USB printer is attached (the
-- ESP32-P4 displays, /api/kitchen/ws) report it: 1 = printer attached,
-- 0 = display only. NULL = a classic printer client (the Pi / STM32 board
-- over the SSH tunnel), which counts as printing whenever it is connected.
-- The printer-offline alarm watches only printing clients.
ALTER TABLE kitchen_clients ADD COLUMN has_printer INTEGER;
