-- Optional Google Search Console verification token. Seeds an EMPTY,
-- admin-editable `shop_*` row (label + hint), so it appears in /admin/settings
-- where the shop pastes its own token. Empty value = no meta tag rendered;
-- when set, the shell emits <meta name="google-site-verification" content=…>
-- in the homepage <head>. The TOKEN itself is confidential and lives only in
-- each shop's DB — it is NOT seeded here. INSERT OR IGNORE preserves any value
-- a shop has already set.
INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('shop_google_site_verification', '',
   'Google Search Console Token',
   'Optionaler Verifizierungs-Token (google-site-verification). Leer = kein Meta-Tag. Wird im <head> der Startseite gerendert.');
