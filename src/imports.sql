CREATE TABLE import_batches (
    id INTEGER PRIMARY KEY,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    from_day TEXT NOT NULL,
    to_day TEXT NOT NULL,
    imported_at INTEGER NOT NULL,
    UNIQUE(site_id, from_day, to_day)
);
CREATE TABLE import_daily (
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    day TEXT NOT NULL,
    batch_id INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
    visitors INTEGER NOT NULL CHECK(visitors>=0),
    pageviews INTEGER NOT NULL CHECK(pageviews>=0),
    PRIMARY KEY(site_id, day)
) WITHOUT ROWID;
CREATE TABLE import_breakdowns (
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    day TEXT NOT NULL,
    dimension TEXT NOT NULL,
    value TEXT NOT NULL,
    batch_id INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
    pageviews INTEGER NOT NULL CHECK(pageviews>=0),
    PRIMARY KEY(site_id, day, dimension, value)
) WITHOUT ROWID;
PRAGMA user_version = 3;
