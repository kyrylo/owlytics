-- Clicks reference their originating pageview and its anonymous visitor. They
-- never create pageviews, visits, or another persistent visitor identifier.
CREATE TABLE outbound_clicks (
    id INTEGER PRIMARY KEY,
    site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    event_id INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    request_id BLOB NOT NULL CHECK(length(request_id)=16),
    received_at INTEGER NOT NULL,
    url TEXT NOT NULL,
    UNIQUE(site_id, request_id)
);
CREATE INDEX outbound_site_time ON outbound_clicks(site_id, received_at);
CREATE INDEX outbound_event ON outbound_clicks(event_id);
PRAGMA user_version = 18;
