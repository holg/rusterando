-- Bon-Editor: admin-overridable receipt badge/banner texts. Every value
-- starts EMPTY → the built-in German default prints (kitchen_protocol
-- ReceiptLabels::or). Non-empty → that text is used. Edited only in the
-- Bon-Editor (/admin/printer); hidden from the generic /admin/settings.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('receipt_label_channel_delivery', '', 'Bon-Badge: Lieferung',  'Standard: LIEFERUNG'),
  ('receipt_label_channel_pickup',   '', 'Bon-Badge: Abholung',   'Standard: ABHOLUNG'),
  ('receipt_label_channel_dinein',   '', 'Bon-Badge: Tisch',      'Standard: TISCH (Tischnr. wird angehängt)'),
  ('receipt_label_paid_badge',       '', 'Bon-Badge: Bezahlt',    'Standard: BEZAHLT'),
  ('receipt_label_unpaid_badge',     '', 'Bon-Badge: Nicht bezahlt', 'Standard: NICHT BEZAHLT'),
  ('receipt_label_paid_confirm',     '', 'Bon-Badge: Bezahlt-Bestätigung', 'Standard: BESTELLUNG IST / BEZAHLT (je Zeile)'),
  ('receipt_label_paid_by',          '', 'Bon: „Bezahlt von“',    'Standard: Bezahlt von:'),
  ('receipt_label_cash',             '', 'Bon: „Bar“',            'Standard: Bar'),
  ('receipt_label_collect_delivery', '', 'Bon: kassieren (Lieferung)', 'Standard: Bei Lieferung kassieren:'),
  ('receipt_label_collect_pickup',   '', 'Bon: kassieren (Abholung)',  'Standard: Bei Abholung kassieren:'),
  ('receipt_label_collect_dinein',   '', 'Bon: kassieren (Tisch)',     'Standard: Am Tisch kassieren:'),
  ('receipt_label_disclaimer',       '', 'Bon: Fuß-Hinweis',      'Standard: Das ist keine Rechnung'),
  ('receipt_label_qr_caption',       '', 'Bon: QR-Beschriftung',  'Standard: Zum Bestelldetail scannen'),
  ('receipt_label_allergen_notice',  '', 'Bon: Allergen-Hinweistext', 'Standard: WICHTIG-Block (eine Zeile je Umbruch).'),
  ('receipt_label_reprint_banner',   '', 'Bon-Badge: Nachdruck',  'Standard: *** REPRINT ***'),
  ('receipt_label_test_banner',      '', 'Bon-Badge: Testdruck',  'Standard: *** TEST ***');
