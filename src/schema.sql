CREATE TABLE sites (
    id INTEGER PRIMARY KEY,
    domain TEXT NOT NULL UNIQUE,
    timezone TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE site_salts (
    site_id INTEGER PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    day TEXT NOT NULL,
    secret BLOB NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE events (
    id INTEGER PRIMARY KEY,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    received_at INTEGER NOT NULL,
    day TEXT NOT NULL,
    visitor BLOB NOT NULL CHECK(length(visitor)=16),
    request_id BLOB NOT NULL CHECK(length(request_id)=16),
    path TEXT NOT NULL,
    source TEXT NOT NULL,
    country TEXT NOT NULL,
    browser TEXT NOT NULL,
    device TEXT NOT NULL,
    UNIQUE(site_id, request_id)
);
CREATE INDEX events_site_day_visitor ON events(site_id, day, visitor);
CREATE INDEX events_site_time ON events(site_id, received_at);
CREATE TABLE geo_ranges (
    family INTEGER NOT NULL,
    start BLOB NOT NULL,
    end BLOB NOT NULL,
    country TEXT NOT NULL,
    PRIMARY KEY(family, start)
) WITHOUT ROWID;
CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;
PRAGMA user_version = 2;
