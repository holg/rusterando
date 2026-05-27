-- Timed "snooze" for online orders.
--
-- Complements the indefinite `orders_paused` switch: when the kitchen is
-- overwhelmed the admin can pause incoming orders for a set duration
-- (e.g. 1 h) and have them resume automatically — no need to remember to
-- flip the switch back.
--
-- `orders_paused_until` holds a UTC timestamp ('YYYY-MM-DD HH:MM:SS').
-- Ordering is blocked while `now < orders_paused_until`; once the time
-- passes, the next order/page check reads the DB and ordering resumes on
-- its own (no background job). Empty string = no active snooze.
--
-- The effective "closed" decision is: orders_paused = '1' (indefinite)
-- OR now < orders_paused_until (timed). See `order_pause_state` in
-- pages/settings.rs.
INSERT INTO app_settings (key, value, label_de, hint_de) VALUES (
    'orders_paused_until',
    '',
    'Bestellungen pausiert bis (automatisch)',
    'Zeitpunkt (UTC), bis zu dem Online-Bestellungen pausiert sind. Wird über die Schnellpause-Knöpfe (30 Min / 1 Std / 2 Std) im Öffnungszeiten-Bereich gesetzt und läuft automatisch ab. Leer = keine zeitliche Pause.'
)
ON CONFLICT(key) DO NOTHING;
