-- Per-shop toggle: when on, the customer-facing menu page renders an
-- always-visible vertical list of category links pinned to the left
-- edge of the viewport (in addition to the horizontal tab strip at
-- the top). Hidden below 60rem so phones / tablets only see the
-- tabs. Default '0' so existing deployments are unchanged.
INSERT INTO app_settings (key, value, label_de, hint_de) VALUES (
    'menu_category_overlay',
    '0',
    'Kategorie-Übersicht im Menü',
    'Wenn aktiviert, zeigt die Speisekarte zusätzlich zur horizontalen Tab-Leiste eine dauerhaft sichtbare Kategorie-Liste am linken Bildschirmrand (nur auf größeren Bildschirmen, unter 60 rem versteckt). Aus = nur die Tabs oben.'
)
ON CONFLICT(key) DO NOTHING;
