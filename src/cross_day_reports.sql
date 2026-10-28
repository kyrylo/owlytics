-- Daily counters are additive only when identities and visits stay in one
-- local date. Remember affected dates, not another copy of visitor identities.
CREATE TABLE report_cross_day_dates (
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 day TEXT NOT NULL,
 PRIMARY KEY(site_id,day)
) WITHOUT ROWID;

-- Historical facts remain unchanged. SQLite sorts this migration on disk;
-- intervals also cover event-free dates inside a visit spanning midnight.
INSERT OR IGNORE INTO report_cross_day_dates
WITH RECURSIVE visitors AS (
 SELECT site_id,MIN(day) AS first,MAX(day) AS last
 FROM events GROUP BY site_id,visitor HAVING MIN(day)<>MAX(day)
), dates(site_id,day,last) AS (
 SELECT site_id,first,last FROM visitors
 UNION ALL SELECT site_id,date(day,'+1 day'),last FROM dates WHERE day<last
)
SELECT site_id,day FROM dates;

-- Session assignment happens inside the event transaction. Its visitor index
-- finds other visits by this identity without scanning unrelated traffic.
CREATE TRIGGER events_cross_day_reports AFTER UPDATE OF session_id ON events
WHEN OLD.session_id IS NULL AND NEW.session_id IS NOT NULL
 AND EXISTS(SELECT 1 FROM sessions INDEXED BY sessions_visitor_time WHERE site_id=NEW.site_id
  AND visitor=NEW.visitor AND day<>NEW.day)
BEGIN
 INSERT OR IGNORE INTO report_cross_day_dates
 SELECT site_id,day FROM sessions INDEXED BY sessions_visitor_time WHERE site_id=NEW.site_id AND visitor=NEW.visitor
 UNION SELECT NEW.site_id,NEW.day;

 INSERT OR IGNORE INTO report_cross_day_dates
 SELECT NEW.site_id,day FROM (
  WITH RECURSIVE dates(day) AS (
   SELECT day FROM sessions WHERE id=NEW.session_id AND day<NEW.day
   UNION ALL SELECT date(day,'+1 day') FROM dates WHERE day<NEW.day
  ) SELECT day FROM dates
 );
END;
PRAGMA user_version=15;
