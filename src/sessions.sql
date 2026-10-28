-- Session identities use only the existing daily, site-scoped visitor hash.
-- No additional browser storage or long-lived identifier is introduced.
DROP TRIGGER events_daily_reports;
ALTER TABLE events ADD COLUMN session_id INTEGER;
ALTER TABLE events ADD COLUMN referrer TEXT NOT NULL DEFAULT '';
UPDATE events SET referrer=source WHERE instr(source,'.')>0;
ALTER TABLE events ADD COLUMN region TEXT NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN city TEXT NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN browser_version TEXT NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN os TEXT NOT NULL DEFAULT '';
ALTER TABLE events ADD COLUMN os_version TEXT NOT NULL DEFAULT '';
CREATE TABLE sessions (
 id INTEGER PRIMARY KEY,
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 visitor BLOB NOT NULL,
 day TEXT NOT NULL,
 started_at INTEGER NOT NULL,
 last_at INTEGER NOT NULL,
 entry_page TEXT NOT NULL,
 exit_page TEXT NOT NULL,
 source TEXT NOT NULL,
 referrer TEXT NOT NULL,
 country TEXT NOT NULL,
 region TEXT NOT NULL,
 city TEXT NOT NULL,
 browser TEXT NOT NULL,
 browser_version TEXT NOT NULL,
 os TEXT NOT NULL,
 os_version TEXT NOT NULL,
 device TEXT NOT NULL,
 pageviews INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX sessions_visitor_time ON sessions(site_id,visitor,last_at DESC);
CREATE INDEX sessions_site_day ON sessions(site_id,day);
CREATE INDEX events_session ON events(session_id);
CREATE TABLE geo_locations (
 family INTEGER NOT NULL,
 start BLOB NOT NULL,
 region TEXT NOT NULL,
 city TEXT NOT NULL,
 PRIMARY KEY(family,start),
 FOREIGN KEY(family,start) REFERENCES geo_ranges(family,start) ON DELETE CASCADE
) WITHOUT ROWID;
-- Recover visits from the stored event sequence, including existing installs.
CREATE TEMP TABLE session_backfill AS
WITH gaps AS (
 SELECT id,site_id,visitor,received_at,
 CASE WHEN received_at-LAG(received_at) OVER(PARTITION BY site_id,visitor ORDER BY received_at,id)<=1800 THEN 0 ELSE 1 END AS starts
 FROM events
), groups AS (
 SELECT *,SUM(starts) OVER(PARTITION BY site_id,visitor ORDER BY received_at,id) AS visit FROM gaps
)
SELECT id,FIRST_VALUE(id) OVER(PARTITION BY site_id,visitor,visit ORDER BY received_at,id) AS session_id FROM groups;
CREATE UNIQUE INDEX session_backfill_id ON session_backfill(id);
UPDATE events SET session_id=(SELECT session_id FROM session_backfill WHERE id=events.id);
DROP TABLE session_backfill;
INSERT INTO sessions
 SELECT e.id,e.site_id,e.visitor,e.day,e.received_at,MAX(v.received_at),e.path,
 (SELECT path FROM events WHERE session_id=e.id ORDER BY received_at DESC,id DESC LIMIT 1),
 e.source,e.referrer,e.country,e.region,e.city,e.browser,e.browser_version,e.os,e.os_version,e.device,COUNT(*)
 FROM events e JOIN events v ON v.session_id=e.id WHERE e.id=e.session_id GROUP BY e.id;
UPDATE events SET source=(SELECT source FROM sessions WHERE id=events.session_id),referrer=(SELECT referrer FROM sessions WHERE id=events.session_id);
DELETE FROM daily_reports;
INSERT INTO daily_reports SELECT site_id,day,'total','',COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day;
INSERT INTO daily_reports SELECT site_id,day,'page',path,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,path;
INSERT INTO daily_reports SELECT site_id,day,'source',source,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,source;
INSERT INTO daily_reports SELECT site_id,day,'referrer',referrer,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,referrer;
INSERT INTO daily_reports SELECT site_id,day,'country',country,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,country;
INSERT INTO daily_reports SELECT site_id,day,'region',region,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,region;
INSERT INTO daily_reports SELECT site_id,day,'city',city,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,city;
INSERT INTO daily_reports SELECT site_id,day,'browser',browser,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,browser;
INSERT INTO daily_reports SELECT site_id,day,'browser_version',browser_version,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,browser_version;
INSERT INTO daily_reports SELECT site_id,day,'os',os,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,os;
INSERT INTO daily_reports SELECT site_id,day,'os_version',os_version,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,os_version;
INSERT INTO daily_reports SELECT site_id,day,'device',device,COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,device;
CREATE TRIGGER events_daily_reports AFTER INSERT ON events BEGIN
 INSERT INTO sessions(id,site_id,visitor,day,started_at,last_at,entry_page,exit_page,source,referrer,country,region,city,browser,browser_version,os,os_version,device)
 SELECT NEW.id,NEW.site_id,NEW.visitor,NEW.day,NEW.received_at,NEW.received_at,NEW.path,NEW.path,NEW.source,NEW.referrer,NEW.country,NEW.region,NEW.city,NEW.browser,NEW.browser_version,NEW.os,NEW.os_version,NEW.device
 WHERE NOT EXISTS(SELECT 1 FROM sessions WHERE site_id=NEW.site_id AND visitor=NEW.visitor AND last_at>=NEW.received_at-1800);
 UPDATE events SET session_id=(SELECT id FROM sessions WHERE site_id=NEW.site_id AND visitor=NEW.visitor ORDER BY last_at DESC,id DESC LIMIT 1) WHERE id=NEW.id;
 UPDATE sessions SET last_at=MAX(last_at,NEW.received_at),exit_page=NEW.path,pageviews=pageviews+1
 WHERE id=(SELECT session_id FROM events WHERE id=NEW.id);
 UPDATE events SET source=(SELECT source FROM sessions WHERE id=events.session_id),referrer=(SELECT referrer FROM sessions WHERE id=events.session_id) WHERE id=NEW.id;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'total','',NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'page',e.path,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.path=e.path),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'source',e.source,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.source=e.source),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'referrer',e.referrer,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.referrer=e.referrer),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'country',e.country,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.country=e.country),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'region',e.region,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.region=e.region),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'city',e.city,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.city=e.city),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'browser',e.browser,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.browser=e.browser),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'browser_version',e.browser_version,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.browser_version=e.browser_version),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'os',e.os,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.os=e.os),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'os_version',e.os_version,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.os_version=e.os_version),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_reports SELECT e.site_id,e.day,'device',e.device,NOT EXISTS(SELECT 1 FROM events p WHERE p.site_id=e.site_id AND p.day=e.day AND p.visitor=e.visitor AND p.id!=e.id AND p.device=e.device),1 FROM events e WHERE e.id=NEW.id ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
END;
CREATE TABLE import_metrics (
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 day TEXT NOT NULL,
 dimension TEXT NOT NULL,
 value TEXT NOT NULL,
 batch_id INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
 visitors INTEGER,
 visits INTEGER,
 bounces INTEGER,
 duration INTEGER,
 exits INTEGER,
 PRIMARY KEY(site_id,day,dimension,value)
) WITHOUT ROWID;
PRAGMA user_version=6;
