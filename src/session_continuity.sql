-- Keep only the current and preceding UTC rotation. Upgrade retains the
-- existing secret only until its original expiry, allowing an active visit to
-- continue on its next event. Stored traffic and summaries are unchanged.
ALTER TABLE site_salts ADD COLUMN previous_secret BLOB;
ALTER TABLE site_salts ADD COLUMN previous_expires_at INTEGER;
UPDATE site_salts SET day='';

-- Match a timeout measured in whole minutes: 30m59s is active, 31m is not.
DROP TRIGGER events_daily_reports;
CREATE TRIGGER events_daily_reports AFTER INSERT ON events BEGIN
 INSERT INTO sessions(id,site_id,visitor,day,started_at,last_at,active_at,entry_page,exit_page,source,referrer,country,region,city,browser,browser_version,os,os_version,device)
 SELECT NEW.id,NEW.site_id,NEW.visitor,NEW.day,NEW.received_at,NEW.received_at,NEW.received_at,NEW.path,NEW.path,NEW.source,NEW.referrer,NEW.country,NEW.region,NEW.city,NEW.browser,NEW.browser_version,NEW.os,NEW.os_version,NEW.device
 WHERE NOT EXISTS(SELECT 1 FROM sessions WHERE site_id=NEW.site_id AND visitor=NEW.visitor AND active_at>NEW.received_at-1860);
 UPDATE events SET session_id=(SELECT id FROM sessions WHERE site_id=NEW.site_id AND visitor=NEW.visitor ORDER BY active_at DESC,id DESC LIMIT 1) WHERE id=NEW.id;
 UPDATE sessions SET last_at=MAX(last_at,NEW.received_at),active_at=MAX(active_at,NEW.received_at),exit_page=NEW.path,pageviews=pageviews+1
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
PRAGMA user_version=16;
