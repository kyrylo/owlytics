-- Cumulative measurements belong to an existing random pageview request ID.
-- They never create visitors, visits or pageviews. Older traffic stays unknown.
CREATE TABLE page_engagement (
 event_id INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
 time_ms INTEGER NOT NULL CHECK(time_ms BETWEEN 0 AND 86400000),
 scroll_depth INTEGER NOT NULL CHECK(scroll_depth BETWEEN 0 AND 100)
);
CREATE TABLE daily_engagement (
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 day TEXT NOT NULL,
 path TEXT NOT NULL,
 time_ms INTEGER NOT NULL,
 visits INTEGER NOT NULL,
 scroll_depth INTEGER NOT NULL,
 PRIMARY KEY(site_id,day,path)
) WITHOUT ROWID;
CREATE TABLE import_engagement (
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 day TEXT NOT NULL,
 path TEXT NOT NULL,
 batch_id INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
 time_ms INTEGER,
 time_visits INTEGER,
 scroll_depth INTEGER,
 scroll_visits INTEGER,
 PRIMARY KEY(site_id,day,path)
) WITHOUT ROWID;
PRAGMA user_version=8;
