CREATE TABLE daily_reports (
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    day TEXT NOT NULL,
    dimension TEXT NOT NULL,
    value TEXT NOT NULL,
    visitors INTEGER NOT NULL,
    pageviews INTEGER NOT NULL,
    PRIMARY KEY(site_id, day, dimension, value)
) WITHOUT ROWID;
INSERT INTO daily_reports SELECT site_id,day,'total','',COUNT(DISTINCT visitor),COUNT(*) FROM events INDEXED BY events_site_time GROUP BY site_id,day;
INSERT INTO daily_reports SELECT site_id,day,'page',path,COUNT(DISTINCT visitor),COUNT(*) FROM events INDEXED BY events_site_time GROUP BY site_id,day,path;
INSERT INTO daily_reports SELECT site_id,day,'source',source,COUNT(DISTINCT visitor),COUNT(*) FROM events INDEXED BY events_site_time GROUP BY site_id,day,source;
INSERT INTO daily_reports SELECT site_id,day,'country',country,COUNT(DISTINCT visitor),COUNT(*) FROM events INDEXED BY events_site_time GROUP BY site_id,day,country;
INSERT INTO daily_reports SELECT site_id,day,'browser',browser,COUNT(DISTINCT visitor),COUNT(*) FROM events INDEXED BY events_site_time GROUP BY site_id,day,browser;
INSERT INTO daily_reports SELECT site_id,day,'device',device,COUNT(DISTINCT visitor),COUNT(*) FROM events INDEXED BY events_site_time GROUP BY site_id,day,device;
CREATE TRIGGER events_daily_reports AFTER INSERT ON events BEGIN
    INSERT INTO daily_reports VALUES
    (NEW.site_id,NEW.day,'total','',NOT EXISTS(SELECT 1 FROM events WHERE site_id=NEW.site_id AND day=NEW.day AND visitor=NEW.visitor AND id!=NEW.id),1),
    (NEW.site_id,NEW.day,'page',NEW.path,NOT EXISTS(SELECT 1 FROM events WHERE site_id=NEW.site_id AND day=NEW.day AND visitor=NEW.visitor AND path=NEW.path AND id!=NEW.id),1),
    (NEW.site_id,NEW.day,'source',NEW.source,NOT EXISTS(SELECT 1 FROM events WHERE site_id=NEW.site_id AND day=NEW.day AND visitor=NEW.visitor AND source=NEW.source AND id!=NEW.id),1),
    (NEW.site_id,NEW.day,'country',NEW.country,NOT EXISTS(SELECT 1 FROM events WHERE site_id=NEW.site_id AND day=NEW.day AND visitor=NEW.visitor AND country=NEW.country AND id!=NEW.id),1),
    (NEW.site_id,NEW.day,'browser',NEW.browser,NOT EXISTS(SELECT 1 FROM events WHERE site_id=NEW.site_id AND day=NEW.day AND visitor=NEW.visitor AND browser=NEW.browser AND id!=NEW.id),1),
    (NEW.site_id,NEW.day,'device',NEW.device,NOT EXISTS(SELECT 1 FROM events WHERE site_id=NEW.site_id AND day=NEW.day AND visitor=NEW.visitor AND device=NEW.device AND id!=NEW.id),1)
    ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews;
END;
PRAGMA user_version = 4;
