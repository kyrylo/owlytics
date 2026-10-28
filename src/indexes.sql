-- Cover recent visitor counts without reading the event table.
DROP INDEX IF EXISTS events_site_time;
CREATE INDEX events_site_time ON events(site_id, received_at, visitor);
-- Page filters are usually selective, even on large sites.
CREATE INDEX IF NOT EXISTS events_site_path_day ON events(site_id, path, day);
PRAGMA user_version = 5;
