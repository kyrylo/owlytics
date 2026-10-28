-- Location and software belong to the visit's first pageview. Only native
-- event labels and their affected daily aggregates need normalization.
CREATE TEMP TABLE visit_attribute_days (
 site_id INTEGER NOT NULL,
 day TEXT NOT NULL,
 PRIMARY KEY(site_id,day)
) WITHOUT ROWID;
INSERT INTO visit_attribute_days
 SELECT DISTINCT e.site_id,e.day FROM events e JOIN sessions s ON s.id=e.session_id
 WHERE (e.country,e.region,e.city,e.browser,e.browser_version,e.os,e.os_version,e.device)
 IS NOT (s.country,s.region,s.city,s.browser,s.browser_version,s.os,s.os_version,s.device);

UPDATE events AS e SET (country,region,city,browser,browser_version,os,os_version,device)
 = (s.country,s.region,s.city,s.browser,s.browser_version,s.os,s.os_version,s.device)
 FROM sessions s WHERE s.id=e.session_id
 AND (e.country,e.region,e.city,e.browser,e.browser_version,e.os,e.os_version,e.device)
 IS NOT (s.country,s.region,s.city,s.browser,s.browser_version,s.os,s.os_version,s.device);

-- Recount distinct visitors from normalized facts; summing old label rows
-- would double-count visitors present under both the old and new labels.
DELETE FROM daily_reports
 WHERE dimension IN('country','region','city','browser','browser_version','os','os_version','device')
 AND (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days);
INSERT INTO daily_reports SELECT site_id,day,'country',country,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,country;
INSERT INTO daily_reports SELECT site_id,day,'region',region,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,region;
INSERT INTO daily_reports SELECT site_id,day,'city',city,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,city;
INSERT INTO daily_reports SELECT site_id,day,'browser',browser,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,browser;
INSERT INTO daily_reports SELECT site_id,day,'browser_version',browser_version,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,browser_version;
INSERT INTO daily_reports SELECT site_id,day,'os',os,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,os;
INSERT INTO daily_reports SELECT site_id,day,'os_version',os_version,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,os_version;
INSERT INTO daily_reports SELECT site_id,day,'device',device,COUNT(DISTINCT visitor),COUNT(*) FROM events
 WHERE (site_id,day) IN(SELECT site_id,day FROM visit_attribute_days) GROUP BY site_id,day,device;
DROP TABLE visit_attribute_days;

-- Assignment happens inside events_daily_reports, before its aggregate inserts.
-- This does not depend on the order of sibling AFTER INSERT triggers. The page
-- visit trigger also observes this assignment but only reads path/session IDs.
CREATE TRIGGER events_visit_attributes AFTER UPDATE OF session_id ON events
WHEN OLD.session_id IS NULL AND NEW.session_id IS NOT NULL BEGIN
 UPDATE events SET (country,region,city,browser,browser_version,os,os_version,device)
 = (SELECT country,region,city,browser,browser_version,os,os_version,device
    FROM sessions WHERE id=NEW.session_id)
 WHERE id=NEW.id;
END;
PRAGMA user_version=14;
