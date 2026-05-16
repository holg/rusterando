-- Runtime toggle for the multilingual UI.
--
-- Before this commit, /en/, /fr/, etc. were gated by the cargo
-- feature `i18n`. That meant two binaries: Davids' German-only one
-- and rusterando's multilingual one. The runtime toggle makes that
-- a single binary — the locale machinery is always compiled in, but
-- the admin chooses per-shop whether to surface it.
--
-- Default '0' (disabled) matches Davids' current behaviour: the
-- /admin/settings page picks up the toggle, and admins of new
-- deployments must explicitly turn it on.
INSERT INTO app_settings (key, value) VALUES ('i18n_enabled', '0')
ON CONFLICT(key) DO NOTHING;
