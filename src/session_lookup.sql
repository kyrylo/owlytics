-- Entry/exit and acquisition summaries check other visits by the same daily
-- visitor. Include the visitor in the existing day index so a fresh database
-- does not scan every visit that day before planner statistics are available.
-- The site/day prefix still serves date-range reports. No counts are rebuilt.
DROP INDEX sessions_site_day;
CREATE INDEX sessions_site_day ON sessions(site_id,day,visitor);
-- The page-visitor check has the same ambiguity with a popular page's index.
-- Retain its site/path/day prefix for reports, and narrow collection to the
-- visitor instead of walking everyone who reached that page today.
DROP INDEX events_site_path_day;
CREATE INDEX events_site_path_day ON events(site_id,path,day,visitor);
PRAGMA user_version=11;
