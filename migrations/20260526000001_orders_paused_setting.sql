-- Manual "pause online orders" switch + optional customer-facing message.
--
-- The shop already has opening_hours, but `place_order` never enforced
-- them and there was no way to stop taking orders on the fly (sudden
-- closure, illness, kitchen overwhelmed). This adds a manual override
-- on top of the opening-hours check.
--
--   orders_paused          '0'/'1' — when '1', no online orders are
--                          accepted regardless of opening hours.
--   orders_paused_message  optional free text shown to customers in the
--                          "closed" banner when set (e.g. "Heute wegen
--                          Krankheit geschlossen"). Empty = generic text.
--
-- Default '0' / '' keeps current behaviour: ordering follows opening
-- hours only. Insert-only + ON CONFLICT DO NOTHING so re-applying is a
-- no-op and never touches existing rows.
INSERT INTO app_settings (key, value, label_de, hint_de) VALUES (
    'orders_paused',
    '0',
    'Online-Bestellungen pausieren',
    'An = es werden vorübergehend KEINE Online-Bestellungen angenommen (unabhängig von den Öffnungszeiten). Kunden sehen einen Hinweis und können nicht bestellen. Aus = normaler Betrieb nach Öffnungszeiten.'
)
ON CONFLICT(key) DO NOTHING;

INSERT INTO app_settings (key, value, label_de, hint_de) VALUES (
    'orders_paused_message',
    '',
    'Hinweistext bei pausierten Bestellungen',
    'Optionaler Text, der Kunden angezeigt wird, wenn Online-Bestellungen pausiert sind (z. B. "Heute wegen Krankheit geschlossen"). Leer = Standardtext.'
)
ON CONFLICT(key) DO NOTHING;
