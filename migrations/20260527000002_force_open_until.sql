-- Ad-hoc "force open" override for the quick switch.
--
-- The manual pause switch could only ADD a closure (orders_paused); on a
-- day that's closed by the weekly schedule (e.g. Wednesday Ruhetag) or
-- outside opening hours, hitting "Jetzt öffnen" did nothing because the
-- closed state came from the schedule, not the pause flag.
--
-- `force_open_until` holds a UTC timestamp ('YYYY-MM-DD HH:MM:SS', set to
-- end of the local day). While `now < force_open_until`, ordering is OPEN
-- regardless of the weekly schedule — the quick "we're open today even
-- though it's normally closed" switch. It auto-expires at end of day so a
-- future Ruhetag isn't accidentally left open. Empty = no force-open.
--
-- Precedence (see `order_pause_state` / `today_slots`):
--   orders_paused=1  >  orders_paused_until(snooze)  >  force_open_until  >  weekly schedule
-- i.e. an explicit pause/snooze still beats force-open (closing always wins
-- over a stale force-open), but force-open beats a closed schedule.
--
-- Distinct from `special_hours`: that table is for PLANNED, listed date
-- overrides in /admin/hours; this is an unlisted, same-day quick toggle.
INSERT INTO app_settings (key, value, label_de, hint_de) VALUES (
    'force_open_until',
    '',
    'Manuell geöffnet bis (automatisch)',
    'Zeitpunkt (UTC), bis zu dem Online-Bestellungen ZWANGSWEISE offen sind — auch an Ruhetagen / außerhalb der Öffnungszeiten. Wird über "Jetzt öffnen" gesetzt (bis Tagesende) und läuft automatisch ab. Leer = keine Zwangsöffnung.'
)
ON CONFLICT(key) DO NOTHING;
