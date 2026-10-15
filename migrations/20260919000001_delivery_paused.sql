-- "Open, but no delivery" — pickup stays available, delivery is suspended.
--
-- The real case (Davids, 2026-09-19): the drivers were sent home, so the shop
-- was still cooking and serving walk-ins, but could not deliver. The only
-- tools were `orders_paused` (kills ALL online ordering — wrong, pickup was
-- fine) and deactivating every delivery zone (destructive, and it loses the
-- zone config). Customers saw "closed" and stopped ordering pickup too.
--
--   delivery_paused          '0'/'1' — when '1', delivery is not offered:
--                            the checkout hides the option, `place_order`
--                            rejects a delivery order, and the customer sees
--                            a "nur Abholung" badge instead of the red
--                            closed banner.
--   delivery_paused_message  optional free text for the badge/banner.
--                            Empty = generic translated text.
--
-- Wording note: the customer-facing default says "aktuell"/"right now", never
-- "heute". Drivers can turn up within the hour, and telling customers the
-- whole day is lost sends them elsewhere for good.
--
-- Deliberately NOT modelled as a ShopLevel variant: the shop IS open, and
-- pickup ordering must keep working. Mirrors the orders_paused pattern so the
-- admin UI, the SSE channel and the settings validation all stay uniform.
--
-- Default '0' / '' keeps current behaviour. Insert-only + ON CONFLICT DO
-- NOTHING so re-applying is a no-op.
INSERT INTO app_settings (key, value, label_de, hint_de) VALUES (
    'delivery_paused',
    '0',
    'Lieferung pausieren (Abholung bleibt möglich)',
    'Wenn aktiv: keine Lieferbestellungen (z. B. kein Fahrer da). Die Abholung läuft normal weiter und der Shop bleibt geöffnet. Jederzeit mit einem Klick wieder aktivierbar — Kundinnen und Kunden sehen die Änderung sofort.'
) ON CONFLICT(key) DO NOTHING;

INSERT INTO app_settings (key, value, label_de, hint_de) VALUES (
    'delivery_paused_message',
    '',
    'Hinweis bei pausierter Lieferung',
    'Optionaler Text für Kundinnen und Kunden, z. B. „Aktuell leider keine Lieferung — Abholung ist möglich.“ Am besten ohne „heute“ formulieren: sobald wieder ein Fahrer da ist, genügt ein Klick. Leer = Standardtext.'
) ON CONFLICT(key) DO NOTHING;
