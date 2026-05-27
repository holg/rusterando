-- Date-specific opening-hours overrides ("holiday" exceptions).
--
-- The weekday-keyed `opening_hours` table can't express a single
-- calendar date: e.g. open on a normally-closed Wednesday because it's
-- a busy holiday, or close on a specific date for a private event.
--
-- One row per calendar date wins over the weekday schedule for that
-- date (see `today_slots` in pages/order.rs):
--   * is_closed = 1            -> shop forced closed that date, no slots.
--   * is_closed = 0 + times    -> shop forced open with that window,
--                                 even if the weekday would be closed.
--
-- `note` is an optional admin label (e.g. "Ostermontag") shown in the
-- /admin/hours editor. Creating the table only — rows are added by the
-- admin UI, so this is safe to (re-)apply to the live DB.
CREATE TABLE IF NOT EXISTS special_hours (
    date       TEXT PRIMARY KEY,            -- 'YYYY-MM-DD' (local date)
    is_closed  INTEGER NOT NULL DEFAULT 0,  -- 1 = force closed this date
    open_time  TEXT,                        -- 'HH:MM' when is_closed = 0
    close_time TEXT,
    note       TEXT
);
