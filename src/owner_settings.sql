CREATE TABLE owner_credentials (
    id INTEGER PRIMARY KEY CHECK(id=1),
    password_hash TEXT NOT NULL
);
CREATE TABLE site_favicons (
    site_id INTEGER PRIMARY KEY REFERENCES sites(id) ON DELETE CASCADE,
    content_type TEXT NOT NULL,
    data BLOB NOT NULL,
    fetched_at INTEGER NOT NULL
);
PRAGMA user_version=17;
