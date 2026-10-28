//! One-time conversion of known location names to the imported identity format.
//! The caller owns the schema transaction; any failure rolls the whole upgrade back.
use rusqlite::{Connection, params};

pub(crate) fn migrate(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch("CREATE TEMP TABLE location_keys(dimension TEXT,old TEXT,new TEXT,PRIMARY KEY(dimension,old)) WITHOUT ROWID;")?;
    for dimension in ["region", "city"] {
        let mut rows = c.prepare(&format!(
            "SELECT {dimension} FROM events UNION SELECT {dimension} FROM sessions
             UNION SELECT {dimension} FROM geo_locations
             UNION SELECT value FROM import_breakdowns WHERE dimension=?1
             UNION SELECT value FROM import_metrics WHERE dimension=?1
             UNION SELECT json_extract(dimensions,'$.{dimension}') FROM import_groups
             WHERE kind='location' AND json_type(dimensions,'$.{dimension}')='text'"
        ))?;
        let mut insert = c.prepare("INSERT INTO temp.location_keys VALUES(?1,?2,?3)")?;
        for row in rows.query_map([dimension], |r| r.get::<_, String>(0))? {
            let old = row?;
            let new = crate::geography::location_key(dimension, &old);
            if old != new {
                insert.execute(params![dimension, old, new])?;
            }
        }
        let changed: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM temp.location_keys WHERE dimension=?1)",
            [dimension],
            |r| r.get(0),
        )?;
        if !changed {
            continue;
        }
        let mut native_changed = false;
        for table in ["events", "sessions", "geo_locations"] {
            let changed = c.execute(&format!(
                "UPDATE {table} SET {dimension}=(SELECT new FROM temp.location_keys WHERE dimension=?1 AND old={table}.{dimension})
                 WHERE {dimension} IN (SELECT old FROM temp.location_keys WHERE dimension=?1)"
            ), [dimension])?;
            native_changed |= changed > 0 && table != "geo_locations";
        }
        if native_changed {
            // Merged aliases may share a daily visitor: summing the old cache
            // would double count. Recompute uniqueness from the preserved facts.
            c.execute("DELETE FROM daily_reports WHERE dimension=?1", [dimension])?;
            c.execute(&format!("INSERT INTO daily_reports SELECT site_id,day,?1,{dimension},COUNT(DISTINCT visitor),COUNT(*) FROM events NOT INDEXED GROUP BY site_id,day,{dimension}"), [dimension])?;
            c.execute(
                "DELETE FROM daily_session_reports WHERE dimension=?1",
                [dimension],
            )?;
            c.execute(&format!("INSERT INTO daily_session_reports SELECT site_id,day,?1,{dimension},0,COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions NOT INDEXED WHERE pageviews>0 GROUP BY site_id,day,{dimension}"), [dimension])?;
        }
        c.execute(&format!(
            "UPDATE import_groups SET dimensions=json_set(dimensions,'$.{dimension}',
             (SELECT new FROM temp.location_keys WHERE dimension=?1 AND old=json_extract(import_groups.dimensions,'$.{dimension}')))
             WHERE kind='location' AND json_extract(dimensions,'$.{dimension}') IN (SELECT old FROM temp.location_keys WHERE dimension=?1)"
        ), [dimension])?;
        c.execute("CREATE TEMP TABLE normalized_breakdowns AS SELECT site_id,day,dimension,COALESCE((SELECT new FROM temp.location_keys WHERE dimension=b.dimension AND old=b.value),value) AS value,batch_id,SUM(pageviews) AS pageviews FROM import_breakdowns b WHERE dimension=?1 GROUP BY site_id,day,dimension,4,batch_id", [dimension])?;
        c.execute(
            "DELETE FROM import_breakdowns WHERE dimension=?1",
            [dimension],
        )?;
        c.execute_batch("INSERT INTO import_breakdowns SELECT * FROM temp.normalized_breakdowns; DROP TABLE temp.normalized_breakdowns;")?;
        let sums = ["visitors", "visits", "bounces", "duration", "exits"]
            .map(|metric| {
                format!("CASE WHEN COUNT({metric})=COUNT(*) THEN SUM({metric}) END AS {metric}")
            })
            .join(",");
        c.execute(&format!("CREATE TEMP TABLE normalized_metrics AS SELECT site_id,day,dimension,COALESCE((SELECT new FROM temp.location_keys WHERE dimension=m.dimension AND old=m.value),value) AS value,batch_id,{sums} FROM import_metrics m WHERE dimension=?1 GROUP BY site_id,day,dimension,4,batch_id"), [dimension])?;
        c.execute("DELETE FROM import_metrics WHERE dimension=?1", [dimension])?;
        c.execute_batch("INSERT INTO import_metrics SELECT * FROM temp.normalized_metrics; DROP TABLE temp.normalized_metrics;")?;
    }
    c.execute_batch("DROP TABLE temp.location_keys; PRAGMA user_version=12;")
}
