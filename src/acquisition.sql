-- Acquisition is stored once per visit. Missing rows mean the old tracker did
-- not measure campaign tags; they must never be treated as known direct traffic.
CREATE TABLE session_acquisition (
 session_id INTEGER PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
 channel TEXT NOT NULL,
 utm_source TEXT NOT NULL,
 utm_medium TEXT NOT NULL,
 utm_campaign TEXT NOT NULL,
 utm_content TEXT NOT NULL,
 utm_term TEXT NOT NULL,
 click_id TEXT CHECK(click_id IN ('gclid','msclkid'))
);
-- Keep the source/referrer/UTM intersection from each exported CE aggregate.
-- Nullable tags mean a column was absent, not that the campaign was untagged.
CREATE TABLE import_acquisition (
 id INTEGER PRIMARY KEY,
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 day TEXT NOT NULL,
 batch_id INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
 source TEXT NOT NULL,
 referrer TEXT NOT NULL,
 channel TEXT,
 utm_source TEXT,
 utm_medium TEXT,
 utm_campaign TEXT,
 utm_content TEXT,
 utm_term TEXT,
 pageviews INTEGER NOT NULL CHECK(pageviews>=0),
 visitors INTEGER CHECK(visitors>=0),
 visits INTEGER CHECK(visits>=0),
 bounces INTEGER CHECK(bounces>=0),
 duration INTEGER CHECK(duration>=0)
);
CREATE INDEX import_acquisition_site_day ON import_acquisition(site_id,day);
-- Batch the six dimensions in each statement: one acquisition lookup per
-- update, while retaining separate daily visitor and session counts.
CREATE TRIGGER acquisition_initial_reports AFTER INSERT ON session_acquisition BEGIN
 INSERT INTO daily_reports
 SELECT s.site_id,s.day,k.dimension,CASE k.dimension WHEN 'channel' THEN a.channel WHEN 'utm_source' THEN a.utm_source WHEN 'utm_medium' THEN a.utm_medium WHEN 'utm_campaign' THEN a.utm_campaign WHEN 'utm_content' THEN a.utm_content WHEN 'utm_term' THEN a.utm_term END,NOT EXISTS(SELECT 1 FROM sessions p JOIN session_acquisition previous ON previous.session_id=p.id WHERE p.site_id=s.site_id AND p.day=s.day AND p.visitor=s.visitor AND p.id!=s.id AND p.pageviews>0 AND CASE k.dimension WHEN 'channel' THEN previous.channel WHEN 'utm_source' THEN previous.utm_source WHEN 'utm_medium' THEN previous.utm_medium WHEN 'utm_campaign' THEN previous.utm_campaign WHEN 'utm_content' THEN previous.utm_content WHEN 'utm_term' THEN previous.utm_term END=CASE k.dimension WHEN 'channel' THEN a.channel WHEN 'utm_source' THEN a.utm_source WHEN 'utm_medium' THEN a.utm_medium WHEN 'utm_campaign' THEN a.utm_campaign WHEN 'utm_content' THEN a.utm_content WHEN 'utm_term' THEN a.utm_term END),s.pageviews
 FROM sessions s JOIN session_acquisition a ON a.session_id=s.id CROSS JOIN (SELECT 'channel' AS dimension UNION ALL SELECT 'utm_source' UNION ALL SELECT 'utm_medium' UNION ALL SELECT 'utm_campaign' UNION ALL SELECT 'utm_content' UNION ALL SELECT 'utm_term') k
 WHERE s.id=NEW.session_id AND s.pageviews>0
 ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_session_reports
 SELECT s.site_id,s.day,k.dimension,CASE k.dimension WHEN 'channel' THEN a.channel WHEN 'utm_source' THEN a.utm_source WHEN 'utm_medium' THEN a.utm_medium WHEN 'utm_campaign' THEN a.utm_campaign WHEN 'utm_content' THEN a.utm_content WHEN 'utm_term' THEN a.utm_term END,NOT EXISTS(SELECT 1 FROM sessions p JOIN session_acquisition previous ON previous.session_id=p.id WHERE p.site_id=s.site_id AND p.day=s.day AND p.visitor=s.visitor AND p.id!=s.id AND p.pageviews>0 AND CASE k.dimension WHEN 'channel' THEN previous.channel WHEN 'utm_source' THEN previous.utm_source WHEN 'utm_medium' THEN previous.utm_medium WHEN 'utm_campaign' THEN previous.utm_campaign WHEN 'utm_content' THEN previous.utm_content WHEN 'utm_term' THEN previous.utm_term END=CASE k.dimension WHEN 'channel' THEN a.channel WHEN 'utm_source' THEN a.utm_source WHEN 'utm_medium' THEN a.utm_medium WHEN 'utm_campaign' THEN a.utm_campaign WHEN 'utm_content' THEN a.utm_content WHEN 'utm_term' THEN a.utm_term END),1,s.pageviews,s.pageviews=1,s.last_at-s.started_at
 FROM sessions s JOIN session_acquisition a ON a.session_id=s.id CROSS JOIN (SELECT 'channel' AS dimension UNION ALL SELECT 'utm_source' UNION ALL SELECT 'utm_medium' UNION ALL SELECT 'utm_campaign' UNION ALL SELECT 'utm_content' UNION ALL SELECT 'utm_term') k
 WHERE s.id=NEW.session_id AND s.pageviews>0
 ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,visits=visits+excluded.visits,pageviews=pageviews+excluded.pageviews,bounces=bounces+excluded.bounces,duration=duration+excluded.duration;
END;
CREATE TRIGGER acquisition_updated_reports AFTER UPDATE OF pageviews,last_at ON sessions
WHEN OLD.pageviews>0 AND EXISTS(SELECT 1 FROM session_acquisition WHERE session_id=NEW.id) BEGIN
 INSERT INTO daily_reports
 SELECT NEW.site_id,NEW.day,k.dimension,CASE k.dimension WHEN 'channel' THEN a.channel WHEN 'utm_source' THEN a.utm_source WHEN 'utm_medium' THEN a.utm_medium WHEN 'utm_campaign' THEN a.utm_campaign WHEN 'utm_content' THEN a.utm_content WHEN 'utm_term' THEN a.utm_term END,0,NEW.pageviews-OLD.pageviews
 FROM session_acquisition a CROSS JOIN (SELECT 'channel' AS dimension UNION ALL SELECT 'utm_source' UNION ALL SELECT 'utm_medium' UNION ALL SELECT 'utm_campaign' UNION ALL SELECT 'utm_content' UNION ALL SELECT 'utm_term') k WHERE a.session_id=NEW.id
 ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET pageviews=pageviews+excluded.pageviews;
 INSERT INTO daily_session_reports
 SELECT NEW.site_id,NEW.day,k.dimension,CASE k.dimension WHEN 'channel' THEN a.channel WHEN 'utm_source' THEN a.utm_source WHEN 'utm_medium' THEN a.utm_medium WHEN 'utm_campaign' THEN a.utm_campaign WHEN 'utm_content' THEN a.utm_content WHEN 'utm_term' THEN a.utm_term END,0,0,NEW.pageviews-OLD.pageviews,(NEW.pageviews=1)-(OLD.pageviews=1),NEW.last_at-OLD.last_at
 FROM session_acquisition a CROSS JOIN (SELECT 'channel' AS dimension UNION ALL SELECT 'utm_source' UNION ALL SELECT 'utm_medium' UNION ALL SELECT 'utm_campaign' UNION ALL SELECT 'utm_content' UNION ALL SELECT 'utm_term') k WHERE a.session_id=NEW.id
 ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET pageviews=pageviews+excluded.pageviews,bounces=bounces+excluded.bounces,duration=duration+excluded.duration;
END;
PRAGMA user_version=9;
