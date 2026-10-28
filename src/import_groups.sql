-- Preserve the relationships present in each exported row. Existing marginal
-- aggregates cannot reconstruct these intersections and are not backfilled.
CREATE TABLE import_groups (
 id INTEGER PRIMARY KEY,
 site_id INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
 day TEXT NOT NULL,
 batch_id INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('location','browser','os')),
 dimensions TEXT NOT NULL CHECK(json_valid(dimensions) AND json_type(dimensions)='object'),
 pageviews INTEGER NOT NULL CHECK(pageviews>=0),
 visitors INTEGER CHECK(visitors>=0),
 visits INTEGER CHECK(visits>=0),
 bounces INTEGER CHECK(bounces>=0),
 duration INTEGER CHECK(duration>=0)
);
CREATE INDEX import_groups_site_kind_day ON import_groups(site_id,kind,day);
CREATE INDEX import_groups_batch_kind ON import_groups(batch_id,kind);
-- Header coverage distinguishes a complete empty export from a missing file or
-- absent column. Existing imports have no invented header metadata.
CREATE TABLE import_files (
 batch_id INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
 report TEXT NOT NULL,
 columns_json TEXT NOT NULL CHECK(json_valid(columns_json) AND json_type(columns_json)='array'),
 PRIMARY KEY(batch_id,report)
) WITHOUT ROWID;
PRAGMA user_version=10;
