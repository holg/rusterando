CREATE TABLE opening_hours (
    id TEXT PRIMARY KEY,
    weekday INTEGER NOT NULL,            -- 0 = Sunday .. 6 = Saturday
    open_time TEXT NOT NULL,             -- 'HH:MM'
    close_time TEXT NOT NULL,
    is_closed INTEGER NOT NULL DEFAULT 0
);
