-- Two-track Stripe configuration: sandbox vs live.
--
-- The shop picks one mode at a time. The `S_…` / `L_…` env vars
-- on the server hold both key sets; an app_settings row decides
-- which is active at request time. Each new order snapshots the
-- mode it was minted in so a mid-flight sandbox→live flip can't
-- strand pending Payment Intents — the webhook routes by row
-- mode, not by the live "active" setting.

INSERT OR IGNORE INTO app_settings (key, value, label_de, hint_de) VALUES
  ('stripe_mode',
   'sandbox',
   'Stripe Modus',
   'Sandbox (Testkarten, kein Geldfluss) oder Live (echte Zahlungen). Wechsel wirkt sofort für NEUE Bestellungen. Laufende Bestellungen behalten ihren ursprünglichen Modus.');

ALTER TABLE orders ADD COLUMN stripe_mode TEXT NOT NULL DEFAULT 'sandbox';

-- Backfill historical rows: we don't know what mode they ran in
-- (older code only had one S_/L_ pair), so leave them as the
-- default 'sandbox'. Real value only kicks in for NEW orders.
