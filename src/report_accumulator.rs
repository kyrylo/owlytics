//! Exact streaming aggregates. Input is ordered by visitor, then session.
//! Common reports fit a small cache; high-cardinality reports spill to SQLite.
use crate::{analytics::Row, report_rows::Rows};
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Default)]
pub(crate) struct Sample {
    pub visitor: Option<[u8; 16]>,
    pub session: Option<i64>,
    pub pageviews: i64,
    pub visits: i64,
    pub bounces: i64,
    pub duration: i64,
    pub entrances: Option<i64>,
    /// One already-folded measured page visit: foreground time and max depth.
    pub engagement: Option<(i64, i64)>,
}

#[derive(Default)]
struct Partial {
    first_visitor: Option<[u8; 16]>,
    last_visitor: Option<[u8; 16]>,
    first_session: Option<i64>,
    last_session: Option<i64>,
    visitors: i64,
    pageviews: i64,
    visits: i64,
    bounces: i64,
    duration: i64,
    entrances: Option<i64>,
    time_ms: i64,
    measured_visits: i64,
    scroll_depth: i64,
}
impl Partial {
    fn add(&mut self, sample: Sample) {
        if let Some(visitor) = sample.visitor {
            self.first_visitor.get_or_insert(visitor);
            if self.last_visitor != Some(visitor) {
                self.visitors += 1;
                self.last_visitor = Some(visitor);
            }
        }
        if let Some(session) = sample.session {
            self.first_session.get_or_insert(session);
            if self.last_session != Some(session) {
                self.visits += 1;
                self.last_session = Some(session);
            }
        } else {
            self.visits += sample.visits;
        }
        self.pageviews += sample.pageviews;
        self.bounces += sample.bounces;
        self.duration += sample.duration;
        if let Some(count) = sample.entrances {
            *self.entrances.get_or_insert(0) += count;
        }
        if let Some((time, depth)) = sample.engagement {
            self.time_ms += time;
            self.measured_visits += 1;
            self.scroll_depth += depth;
        }
    }
    fn row(self, name: String) -> Row {
        Row {
            name,
            visitors: self.visitors,
            pageviews: self.pageviews,
            visits: self.visits,
            bounces: self.bounces,
            duration: self.duration,
            entrances: self.entrances,
            time_on_page_ms: self.time_ms,
            time_on_page_visits: self.measured_visits,
            total_scroll_depth: self.scroll_depth,
            scroll_depth_visits: self.measured_visits,
            ..Default::default()
        }
    }
}

pub(crate) struct Accumulator<'a> {
    connection: &'a Connection,
    dimensions: Vec<&'a str>,
    groups: Vec<HashMap<String, Partial>>,
    entries: usize,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    spilled: bool,
}
impl<'a> Accumulator<'a> {
    pub fn new(connection: &'a Connection, dimensions: Vec<&'a str>) -> Self {
        Self {
            connection,
            groups: dimensions.iter().map(|_| HashMap::new()).collect(),
            dimensions,
            entries: 0,
            bytes: 0,
            max_entries: 4096,
            max_bytes: 1024 * 1024,
            spilled: false,
        }
    }

    pub fn add(&mut self, dimension: usize, name: &str, sample: Sample) -> rusqlite::Result<()> {
        if !self.groups[dimension].contains_key(name) {
            // Account for the key, counters and hash-table overhead. Also cap
            // entries independently, so tiny keys cannot create an enormous map.
            let bytes = name.len() + std::mem::size_of::<(String, Partial)>() + 64;
            if self.entries >= self.max_entries || self.bytes + bytes > self.max_bytes {
                self.flush()?;
            }
            self.groups[dimension].insert(name.to_owned(), Partial::default());
            self.entries += 1;
            self.bytes += bytes;
        }
        self.groups[dimension].get_mut(name).unwrap().add(sample);
        Ok(())
    }

    fn flush(&mut self) -> rusqlite::Result<()> {
        if !self.spilled {
            self.connection.execute_batch(
                "CREATE TEMP TABLE report_accumulator (
                  dimension TEXT NOT NULL,value TEXT NOT NULL,
                  visitors INTEGER NOT NULL,pageviews INTEGER NOT NULL,
                  visits INTEGER NOT NULL,bounces INTEGER NOT NULL,
                  duration INTEGER NOT NULL,entrances INTEGER,
                  last_visitor BLOB,last_session INTEGER,
                  time_ms INTEGER NOT NULL,measured_visits INTEGER NOT NULL,scroll_depth INTEGER NOT NULL,
                  PRIMARY KEY(dimension,value)
                 ) WITHOUT ROWID;",
            )?;
            self.spilled = true;
        }
        // Ordered input makes only the boundary visitor/session overlap between
        // chunks. Subtract that overlap instead of retaining a set of all IDs.
        let mut insert = self.connection.prepare(
            "INSERT INTO temp.report_accumulator VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?13,?14,?15)
             ON CONFLICT(dimension,value) DO UPDATE SET
              visitors=visitors+excluded.visitors-COALESCE(last_visitor=?11,0),
              pageviews=pageviews+excluded.pageviews,
              visits=visits+excluded.visits-COALESCE(last_session=?12,0),
              bounces=bounces+excluded.bounces,duration=duration+excluded.duration,
              entrances=CASE WHEN entrances IS NULL AND excluded.entrances IS NULL THEN NULL ELSE COALESCE(entrances,0)+COALESCE(excluded.entrances,0) END,
              last_visitor=excluded.last_visitor,last_session=excluded.last_session,
              time_ms=time_ms+excluded.time_ms,measured_visits=measured_visits+excluded.measured_visits,
              scroll_depth=scroll_depth+excluded.scroll_depth",
        )?;
        for (index, group) in self.groups.iter_mut().enumerate() {
            // Release each allocation: retaining capacities across flushes would
            // let different dimensions each grow to the entire cache limit.
            for (name, value) in std::mem::take(group) {
                insert.execute(params![
                    self.dimensions[index],
                    name,
                    value.visitors,
                    value.pageviews,
                    value.visits,
                    value.bounces,
                    value.duration,
                    value.entrances,
                    value.last_visitor.as_ref().map(|v| v.as_slice()),
                    value.last_session,
                    value.first_visitor.as_ref().map(|v| v.as_slice()),
                    value.first_session,
                    value.time_ms,
                    value.measured_visits,
                    value.scroll_depth,
                ])?;
            }
        }
        self.entries = 0;
        self.bytes = 0;
        Ok(())
    }

    pub fn finish(mut self) -> rusqlite::Result<BTreeMap<String, Rows>> {
        let mut result = BTreeMap::new();
        if self.spilled {
            self.flush()?;
            for dimension in &self.dimensions {
                result.insert(
                    (*dimension).to_owned(),
                    Rows::from_accumulator(self.connection, dimension)?,
                );
            }
            self.connection
                .execute_batch("DROP TABLE temp.report_accumulator;")?;
        } else {
            for (dimension, group) in self.dimensions.into_iter().zip(self.groups) {
                result.insert(
                    dimension.to_owned(),
                    Rows::collect(
                        self.connection,
                        group.into_iter().map(|(name, value)| Ok(value.row(name))),
                    )?,
                );
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_event_counts_match_sql_across_spill_boundaries() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE input(visitor BLOB,session INTEGER,path TEXT,country TEXT);")
            .unwrap();
        for visitor in 0u128..12 {
            for session in 1..=3 {
                for _ in 0..3 {
                    for page in 0..7 {
                        c.execute(
                            "INSERT INTO input VALUES(?1,?2,?3,?4)",
                            params![
                                visitor.to_be_bytes().as_slice(),
                                visitor as i64 * 3 + session,
                                format!("/日本/{page}'"),
                                if visitor % 2 == 0 { "US" } else { "" }
                            ],
                        )
                        .unwrap();
                    }
                }
            }
        }
        for limit in [2, 4096] {
            let _workspace = crate::report_rows::Workspace::new(&c).unwrap();
            let mut accumulator = Accumulator::new(&c, vec!["page", "country"]);
            accumulator.max_entries = limit;
            let mut statement = c
                .prepare("SELECT visitor,session,path,country FROM input ORDER BY visitor,session")
                .unwrap();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for index in 0..2 {
                    accumulator
                        .add(
                            index,
                            row.get_ref(index + 2).unwrap().as_str().unwrap(),
                            Sample {
                                visitor: Some(row.get(0).unwrap()),
                                session: Some(row.get(1).unwrap()),
                                pageviews: 1,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                }
                assert!(accumulator.entries <= limit);
            }
            let result: BTreeMap<_, _> = accumulator
                .finish()
                .unwrap()
                .into_iter()
                .map(|(key, rows)| (key, rows.to_vec(&c).unwrap()))
                .collect();
            for (dimension, column) in [("page", "path"), ("country", "country")] {
                let expected=c.prepare(&format!("SELECT {column},COUNT(DISTINCT visitor),COUNT(*),COUNT(DISTINCT session) FROM input GROUP BY {column}"))
                    .unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,(r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))))
                    .unwrap().collect::<rusqlite::Result<BTreeMap<_,_>>>().unwrap();
                let actual = result[dimension]
                    .iter()
                    .map(|r| (r.name.clone(), (r.visitors, r.pageviews, r.visits)))
                    .collect::<BTreeMap<_, _>>();
                assert_eq!(actual, expected, "limit={limit}, dimension={dimension}");
            }
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM sqlite_temp_master WHERE name='report_accumulator'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
    }

    #[test]
    fn session_totals_and_optional_entrances_survive_byte_limited_spills() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE input(visitor BLOB,source TEXT,views INTEGER,duration INTEGER,entrances INTEGER);").unwrap();
        for visitor in 0u128..8 {
            for session in 0..4 {
                c.execute(
                    "INSERT INTO input VALUES(?1,?2,?3,?4,?5)",
                    params![
                        visitor.to_be_bytes().as_slice(),
                        format!("source-{session}"),
                        1 + session,
                        session * 20,
                        session % 2
                    ],
                )
                .unwrap();
            }
        }
        for entrances in [false, true] {
            let _workspace = crate::report_rows::Workspace::new(&c).unwrap();
            let mut accumulator = Accumulator::new(&c, vec!["entry"]);
            accumulator.max_bytes = 250;
            let mut statement = c
                .prepare(
                    "SELECT visitor,source,views,duration,entrances FROM input ORDER BY visitor",
                )
                .unwrap();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let views = row.get(2).unwrap();
                accumulator
                    .add(
                        0,
                        row.get_ref(1).unwrap().as_str().unwrap(),
                        Sample {
                            visitor: Some(row.get(0).unwrap()),
                            pageviews: views,
                            visits: 1,
                            bounces: i64::from(views == 1),
                            duration: row.get(3).unwrap(),
                            entrances: entrances.then(|| row.get(4).unwrap()),
                            ..Default::default()
                        },
                    )
                    .unwrap();
            }
            for row in &accumulator.finish().unwrap()["entry"].to_vec(&c).unwrap() {
                let expected=c.query_row("SELECT COUNT(DISTINCT visitor),SUM(views),COUNT(*),SUM(views=1),SUM(duration),SUM(entrances) FROM input WHERE source=?1",[&row.name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get::<_,i64>(5)?))).unwrap();
                assert_eq!(
                    (
                        row.visitors,
                        row.pageviews,
                        row.visits,
                        row.bounces,
                        row.duration
                    ),
                    (expected.0, expected.1, expected.2, expected.3, expected.4)
                );
                assert_eq!(row.entrances, entrances.then_some(expected.5));
            }
        }
    }
}
