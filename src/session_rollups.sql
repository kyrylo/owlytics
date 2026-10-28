-- Session summaries use daily, site-scoped identities already held in events.
-- They contain counts and durations, never additional visitor identifiers.
CREATE TABLE daily_session_reports (
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 day TEXT NOT NULL,
 dimension TEXT NOT NULL,
 value TEXT NOT NULL,
 visitors INTEGER NOT NULL,
 visits INTEGER NOT NULL,
 pageviews INTEGER NOT NULL,
 bounces INTEGER NOT NULL,
 duration INTEGER NOT NULL,
 PRIMARY KEY(site_id,day,dimension,value)
) WITHOUT ROWID;

-- Full rebuild for existing schema-6 installations. Sequential reads avoid the
-- random I/O of walking a date index over an interleaved historical import.
INSERT INTO daily_session_reports SELECT site_id,day,'total','',0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,site_id;
INSERT INTO daily_session_reports SELECT site_id,day,'source',source,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,source;
INSERT INTO daily_session_reports SELECT site_id,day,'referrer',referrer,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,referrer;
INSERT INTO daily_session_reports SELECT site_id,day,'country',country,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,country;
INSERT INTO daily_session_reports SELECT site_id,day,'region',region,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,region;
INSERT INTO daily_session_reports SELECT site_id,day,'city',city,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,city;
INSERT INTO daily_session_reports SELECT site_id,day,'browser',browser,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,browser;
INSERT INTO daily_session_reports SELECT site_id,day,'browser_version',browser_version,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,browser_version;
INSERT INTO daily_session_reports SELECT site_id,day,'os',os,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,os;
INSERT INTO daily_session_reports SELECT site_id,day,'os_version',os_version,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,os_version;
INSERT INTO daily_session_reports SELECT site_id,day,'device',device,0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,device;
INSERT INTO daily_session_reports SELECT site_id,day,'entry',entry_page,COUNT(DISTINCT visitor),COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,entry_page;
INSERT INTO daily_session_reports SELECT site_id,day,'exit',exit_page,COUNT(DISTINCT visitor),COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,exit_page;
INSERT INTO daily_session_reports
 SELECT site_id,day,'page',path,0,COUNT(DISTINCT session_id),0,0,0
 FROM events NOT INDEXED WHERE session_id IS NOT NULL GROUP BY site_id,day,path;

-- The collector creates a zero-view session, then updates it after assigning
-- the pageview. One update maintains visit, bounce and duration deltas for all
-- first-touch dimensions. Replayed event IDs never reach these triggers.
CREATE TRIGGER sessions_daily_reports AFTER UPDATE OF pageviews,last_at,exit_page ON sessions
WHEN NEW.pageviews>0 BEGIN
 INSERT INTO daily_session_reports
 SELECT NEW.site_id,NEW.day,d.dimension,d.value,
 CASE WHEN d.dimension='entry' AND OLD.pageviews=0 THEN
  NOT EXISTS(SELECT 1 FROM sessions s WHERE s.site_id=NEW.site_id AND s.visitor=NEW.visitor
   AND s.day=NEW.day AND s.entry_page=NEW.entry_page AND s.id!=NEW.id AND s.pageviews>0)
 ELSE 0 END,
 (OLD.pageviews=0),NEW.pageviews-OLD.pageviews,
 (NEW.pageviews=1)-(OLD.pageviews=1),
 (NEW.last_at-NEW.started_at)-(OLD.last_at-OLD.started_at)
 FROM (
SELECT 'total' AS dimension,'' AS value
 UNION ALL SELECT 'source' AS dimension,NEW.source AS value
 UNION ALL SELECT 'referrer' AS dimension,NEW.referrer AS value
 UNION ALL SELECT 'country' AS dimension,NEW.country AS value
 UNION ALL SELECT 'region' AS dimension,NEW.region AS value
 UNION ALL SELECT 'city' AS dimension,NEW.city AS value
 UNION ALL SELECT 'browser' AS dimension,NEW.browser AS value
 UNION ALL SELECT 'browser_version' AS dimension,NEW.browser_version AS value
 UNION ALL SELECT 'os' AS dimension,NEW.os AS value
 UNION ALL SELECT 'os_version' AS dimension,NEW.os_version AS value
 UNION ALL SELECT 'device' AS dimension,NEW.device AS value
 UNION ALL SELECT 'entry' AS dimension,NEW.entry_page AS value
 ) d WHERE 1
 ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET
 visitors=visitors+excluded.visitors,visits=visits+excluded.visits,
 pageviews=pageviews+excluded.pageviews,bounces=bounces+excluded.bounces,
 duration=duration+excluded.duration;

 -- A new pageview can move a visit's exit. Remove the old contribution and add
 -- the new one; uniqueness changes only when no other visit by that daily
 -- visitor still ends on the respective page.
 UPDATE daily_session_reports SET
 visitors=visitors-NOT EXISTS(SELECT 1 FROM sessions s WHERE s.site_id=OLD.site_id
  AND s.day=OLD.day AND s.visitor=OLD.visitor AND s.exit_page=OLD.exit_page
  AND s.id!=OLD.id AND s.pageviews>0),
 visits=visits-1,pageviews=pageviews-OLD.pageviews,
 bounces=bounces-(OLD.pageviews=1),duration=duration-(OLD.last_at-OLD.started_at)
 WHERE OLD.pageviews>0 AND site_id=OLD.site_id AND day=OLD.day
  AND dimension='exit' AND value=OLD.exit_page;
 INSERT INTO daily_session_reports
 SELECT NEW.site_id,NEW.day,'exit',NEW.exit_page,
 NOT EXISTS(SELECT 1 FROM sessions s WHERE s.site_id=NEW.site_id AND s.day=NEW.day
  AND s.visitor=NEW.visitor AND s.exit_page=NEW.exit_page AND s.id!=NEW.id AND s.pageviews>0),
 1,NEW.pageviews,(NEW.pageviews=1),NEW.last_at-NEW.started_at
 ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET
 visitors=visitors+excluded.visitors,visits=visits+excluded.visits,
 pageviews=pageviews+excluded.pageviews,bounces=bounces+excluded.bounces,
 duration=duration+excluded.duration;
 DELETE FROM daily_session_reports WHERE site_id=OLD.site_id AND day=OLD.day
  AND dimension='exit' AND value=OLD.exit_page AND visits=0;
END;

-- This trigger runs when the existing event trigger assigns a session, rather
-- than relying on the unspecified execution order of AFTER INSERT triggers.
CREATE TRIGGER events_daily_session_pages AFTER UPDATE OF session_id ON events
WHEN OLD.session_id IS NULL AND NEW.session_id IS NOT NULL BEGIN
 INSERT INTO daily_session_reports
 SELECT NEW.site_id,NEW.day,'page',NEW.path,0,
 NOT EXISTS(SELECT 1 FROM events e WHERE e.session_id=NEW.session_id AND e.path=NEW.path AND e.id!=NEW.id),
 0,0,0
 ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visits=visits+excluded.visits;
END;
PRAGMA user_version=7;
