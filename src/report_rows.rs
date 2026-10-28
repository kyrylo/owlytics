//! Report rows remain in SQLite until pagination, including after aggregation.
//! These tables belong to one read snapshot and never touch the main database.
use crate::analytics::Row;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) struct Workspace<'a>(&'a Connection);
impl<'a> Workspace<'a> {
    pub fn new(c: &'a Connection) -> rusqlite::Result<Self> {
        c.execute_batch(
            "CREATE TEMP TABLE report_sets(id INTEGER PRIMARY KEY);
             CREATE TEMP TABLE report_rows(
               set_id INTEGER NOT NULL,name TEXT NOT NULL,
               visitors INTEGER NOT NULL DEFAULT 0,pageviews INTEGER NOT NULL DEFAULT 0,
               visits INTEGER NOT NULL DEFAULT 0,bounces INTEGER NOT NULL DEFAULT 0,
               duration INTEGER NOT NULL DEFAULT 0,exits INTEGER NOT NULL DEFAULT 0,
               time_on_page_ms INTEGER NOT NULL DEFAULT 0,time_on_page_visits INTEGER NOT NULL DEFAULT 0,
               total_scroll_depth INTEGER NOT NULL DEFAULT 0,scroll_depth_visits INTEGER NOT NULL DEFAULT 0,
               entrances INTEGER,unavailable INTEGER NOT NULL DEFAULT 0,
               session_visits INTEGER,session_pageviews INTEGER,
               PRIMARY KEY(set_id,name)
             ) WITHOUT ROWID;
             CREATE TEMP TABLE report_rank(name TEXT PRIMARY KEY,label TEXT,number REAL) WITHOUT ROWID;",
        )?;
        Ok(Self(c))
    }
}

impl Drop for Workspace<'_> {
    fn drop(&mut self) {
        let _ = self.0.execute_batch(
            "DROP TABLE temp.report_rank; DROP TABLE temp.report_rows; DROP TABLE temp.report_sets;",
        );
    }
}

// Zero denotes an empty set. IDs are scoped to Workspace, not persistent IDs.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Rows(i64);

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        name: row.get(1)?,
        visitors: row.get(2)?,
        pageviews: row.get(3)?,
        visits: row.get(4)?,
        bounces: row.get(5)?,
        duration: row.get(6)?,
        exits: row.get(7)?,
        time_on_page_ms: row.get(8)?,
        time_on_page_visits: row.get(9)?,
        total_scroll_depth: row.get(10)?,
        scroll_depth_visits: row.get(11)?,
        entrances: row.get(12)?,
        unavailable: row.get(13)?,
        session_visits: row.get(14)?,
        session_pageviews: row.get(15)?,
        ..Default::default()
    })
}

pub(crate) struct Selection<'a> {
    pub dimension: &'a str,
    pub sort: &'a str,
    pub ascending: bool,
    pub search: Option<&'a str>,
    pub offset: usize,
    pub limit: usize,
    pub omit_empty: bool,
    pub pageviews: bool,
    pub denominator: i64,
}

impl Rows {
    pub fn omit(self, c: &Connection, names: &[&str]) -> rusqlite::Result<()> {
        if self.0 != 0 {
            let mut statement =
                c.prepare_cached("DELETE FROM temp.report_rows WHERE set_id=?1 AND name=?2")?;
            for name in names {
                statement.execute(params![self.0, name])?;
            }
        }
        Ok(())
    }

    /// Point lookups keep comparison memory proportional to the primary page,
    /// even when the old period has a different ranking or many more values.
    pub fn named(
        self,
        c: &Connection,
        names: &[String],
        pageviews: bool,
        denominator: i64,
    ) -> rusqlite::Result<Vec<Row>> {
        let mut rows = Vec::new();
        if self.0 == 0 {
            return Ok(rows);
        }
        let mut statement =
            c.prepare_cached("SELECT * FROM temp.report_rows WHERE set_id=?1 AND name=?2")?;
        for name in names {
            if let Some(mut row) = statement
                .query_row(params![self.0, name], decode)
                .optional()?
            {
                row.percentage = (denominator > 0).then_some((
                    if pageviews {
                        row.pageviews
                    } else {
                        row.visitors
                    },
                    denominator,
                ));
                rows.push(row);
            }
        }
        Ok(rows)
    }

    pub fn new(c: &Connection) -> rusqlite::Result<Self> {
        c.execute("INSERT INTO temp.report_sets DEFAULT VALUES", [])?;
        Ok(Self(c.last_insert_rowid()))
    }

    /// Copy an aggregate query inside SQLite without decoding/rebinding rows.
    pub fn from_query(
        c: &Connection,
        columns: &str,
        query: &str,
        values: &[rusqlite::types::Value],
    ) -> rusqlite::Result<Self> {
        let result = Self::new(c)?;
        // The column list/query are internal SQL; the set ID is generated here.
        c.execute(
            &format!(
                "INSERT INTO temp.report_rows(set_id,{columns}) SELECT {},q.* FROM ({query}) q",
                result.0
            ),
            rusqlite::params_from_iter(values),
        )?;
        Ok(result)
    }

    pub fn id(self) -> i64 {
        self.0
    }

    pub fn collect(
        c: &Connection,
        input: impl IntoIterator<Item = rusqlite::Result<Row>>,
    ) -> rusqlite::Result<Self> {
        let mut result = Self::default();
        for row in input {
            result.merge(c, &row?)?;
        }
        Ok(result)
    }

    pub fn merge(&mut self, c: &Connection, row: &Row) -> rusqlite::Result<()> {
        if self.0 == 0 {
            *self = Self::new(c)?;
        }
        c.prepare_cached(
            "INSERT INTO temp.report_rows VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(set_id,name) DO UPDATE SET
              visitors=visitors+excluded.visitors,pageviews=pageviews+excluded.pageviews,
              visits=visits+excluded.visits,bounces=bounces+excluded.bounces,
              duration=duration+excluded.duration,exits=exits+excluded.exits,
              time_on_page_ms=time_on_page_ms+excluded.time_on_page_ms,
              time_on_page_visits=time_on_page_visits+excluded.time_on_page_visits,
              total_scroll_depth=total_scroll_depth+excluded.total_scroll_depth,
              scroll_depth_visits=scroll_depth_visits+excluded.scroll_depth_visits,
              entrances=CASE WHEN entrances IS NULL AND excluded.entrances IS NULL THEN NULL ELSE COALESCE(entrances,0)+COALESCE(excluded.entrances,0) END,
              session_visits=CASE WHEN session_visits IS NULL AND excluded.session_visits IS NULL THEN NULL ELSE COALESCE(session_visits,visits)+COALESCE(excluded.session_visits,excluded.visits) END,
              session_pageviews=CASE WHEN session_pageviews IS NULL AND excluded.session_pageviews IS NULL THEN NULL ELSE COALESCE(session_pageviews,pageviews)+COALESCE(excluded.session_pageviews,excluded.pageviews) END,
              unavailable=unavailable|excluded.unavailable",
        )?.execute(params![
            self.0,row.name,row.visitors,row.pageviews,row.visits,row.bounces,row.duration,row.exits,
            row.time_on_page_ms,row.time_on_page_visits,row.total_scroll_depth,row.scroll_depth_visits,
            row.entrances,row.unavailable,row.session_visits,row.session_pageviews,
        ])?;
        Ok(())
    }

    pub fn for_each(
        self,
        c: &Connection,
        mut visit: impl FnMut(Row) -> rusqlite::Result<()>,
    ) -> rusqlite::Result<()> {
        if self.0 == 0 {
            return Ok(());
        }
        let mut statement =
            c.prepare_cached("SELECT * FROM temp.report_rows WHERE set_id=?1 ORDER BY name")?;
        let mut rows = statement.query([self.0])?;
        while let Some(row) = rows.next()? {
            visit(decode(row)?)?;
        }
        Ok(())
    }

    /// Only use for date series (bounded by the date range) or test oracles.
    pub fn to_vec(self, c: &Connection) -> rusqlite::Result<Vec<Row>> {
        let mut result = Vec::new();
        self.for_each(c, |row| {
            result.push(row);
            Ok(())
        })?;
        Ok(result)
    }

    pub fn counts(self, c: &Connection) -> rusqlite::Result<(i64, i64, i64)> {
        c.query_row("SELECT COALESCE(SUM(visitors),0),COALESCE(SUM(pageviews),0),COALESCE(SUM(visits),0) FROM temp.report_rows WHERE set_id=?1", [self.0], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
    }

    pub fn from_accumulator(c: &Connection, dimension: &str) -> rusqlite::Result<Self> {
        let result = Self::new(c)?;
        c.execute("INSERT INTO temp.report_rows(set_id,name,visitors,pageviews,visits,bounces,duration,entrances,time_on_page_ms,time_on_page_visits,total_scroll_depth,scroll_depth_visits) SELECT ?1,value,visitors,pageviews,visits,bounces,duration,entrances,time_ms,measured_visits,scroll_depth,measured_visits FROM temp.report_accumulator WHERE dimension=?2", params![result.0,dimension])?;
        Ok(result)
    }

    pub fn page_visits(self, c: &Connection, source: Self) -> rusqlite::Result<()> {
        c.execute("UPDATE temp.report_rows AS r SET visits=COALESCE((SELECT s.visits FROM temp.report_rows s WHERE s.set_id=?2 AND s.name=r.name),0) WHERE r.set_id=?1", params![self.0,source.0])?;
        Ok(())
    }

    pub fn sessions(self, c: &Connection, source: Self, page: bool) -> rusqlite::Result<()> {
        // Page visits already include internal arrivals. Only their entrance
        // count and bounce/duration values come from the entry-page report.
        if page {
            c.execute(
                "UPDATE temp.report_rows SET entrances=0 WHERE set_id=?1",
                [self.0],
            )?;
        }
        c.execute(
            "UPDATE temp.report_rows AS r SET
             visits=CASE WHEN ?3 THEN r.visits ELSE s.visits END,
             bounces=s.bounces,duration=s.duration,
             entrances=CASE WHEN ?3 THEN s.visits ELSE s.entrances END
             FROM temp.report_rows s WHERE r.set_id=?1 AND s.set_id=?2 AND r.name=s.name",
            params![self.0, source.0, page],
        )?;
        Ok(())
    }

    pub fn engagement(self, c: &Connection, source: Self) -> rusqlite::Result<()> {
        c.execute("UPDATE temp.report_rows AS r SET time_on_page_ms=r.time_on_page_ms+s.time_on_page_ms,time_on_page_visits=r.time_on_page_visits+s.time_on_page_visits,total_scroll_depth=r.total_scroll_depth+s.total_scroll_depth,scroll_depth_visits=r.scroll_depth_visits+s.scroll_depth_visits,unavailable=r.unavailable|(s.unavailable&96) FROM temp.report_rows s WHERE r.set_id=?1 AND s.set_id=?2 AND r.name=s.name", params![self.0,source.0])?;
        Ok(())
    }

    pub fn exits(self, c: &Connection, pages: Self) -> rusqlite::Result<()> {
        c.execute("UPDATE temp.report_rows AS r SET exits=visits,pageviews=COALESCE((SELECT p.pageviews FROM temp.report_rows p WHERE p.set_id=?2 AND p.name=r.name),0) WHERE r.set_id=?1", params![self.0,pages.0])?;
        Ok(())
    }

    pub fn select(
        self,
        c: &Connection,
        selection: Selection<'_>,
    ) -> rusqlite::Result<(Vec<Row>, usize)> {
        if self.0 == 0 {
            return Ok((Vec::new(), 0));
        }
        let percent = |row: &mut Row| {
            row.percentage = (selection.denominator > 0).then_some((
                if selection.pageviews {
                    row.pageviews
                } else {
                    row.visitors
                },
                selection.denominator,
            ));
        };
        let where_clause = if selection.omit_empty {
            "r.set_id=?1 AND r.name!=''"
        } else {
            "r.set_id=?1"
        };
        let direction = if selection.ascending { "ASC" } else { "DESC" };
        let ranked = selection.search.is_some_and(|s| !s.is_empty())
            || !matches!(selection.sort, "visitors" | "pageviews");
        let (from, order) = if ranked {
            // Label search uses the same Unicode folding and geography/source
            // names as the UI. Disk-backed ranks also bound deep pagination.
            c.execute("DELETE FROM temp.report_rank", [])?;
            let search = selection
                .search
                .filter(|s| !s.is_empty())
                .map(str::to_lowercase);
            let mut insert = c.prepare_cached("INSERT INTO temp.report_rank VALUES(?1,?2,?3)")?;
            self.for_each(c, |mut row| {
                if selection.omit_empty && row.name.is_empty() {
                    return Ok(());
                }
                let label = if selection.sort == "name" || search.is_some() {
                    crate::report_table::label(selection.dimension, &row.name).to_lowercase()
                } else {
                    String::new()
                };
                if search.as_ref().is_some_and(|s| !label.contains(s)) {
                    return Ok(());
                }
                percent(&mut row);
                let number = crate::report_table::value(&row, selection.sort);
                insert.execute(params![row.name, label, number])?;
                Ok(())
            })?;
            (
                "temp.report_rows r JOIN temp.report_rank k USING(name)",
                if selection.sort == "name" {
                    format!("k.label {direction},r.pageviews DESC,r.name ASC")
                } else {
                    format!("k.number IS NULL,k.number {direction},r.pageviews DESC,r.name ASC")
                },
            )
        } else {
            // Match Rust's f64 metric ordering, including missing visitor data.
            let value = if selection.sort == "visitors" {
                "CASE WHEN (r.unavailable&1)=0 THEN CAST(r.visitors AS REAL) END"
            } else {
                "CAST(r.pageviews AS REAL)"
            };
            (
                "temp.report_rows r",
                format!("({value}) IS NULL,({value}) {direction},r.pageviews DESC,r.name ASC"),
            )
        };
        let count = c.query_row(
            &format!("SELECT COUNT(*) FROM {from} WHERE {where_clause}"),
            [self.0],
            |r| r.get::<_, i64>(0),
        )?;
        let mut statement = c.prepare(&format!(
            "SELECT r.* FROM {from} WHERE {where_clause} ORDER BY {order} LIMIT ?2 OFFSET ?3"
        ))?;
        let limit = if selection.limit == usize::MAX {
            -1
        } else {
            selection.limit as i64
        };
        let result = statement
            .query_map(params![self.0, limit, selection.offset as i64], |r| {
                let mut row = decode(r)?;
                percent(&mut row);
                Ok(row)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((result, count as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(sort: &str) -> Selection<'_> {
        Selection {
            dimension: "page",
            sort,
            ascending: false,
            search: None,
            offset: 0,
            limit: 50,
            omit_empty: false,
            pageviews: false,
            denominator: 100_000,
        }
    }

    #[test]
    fn disk_selection_matches_full_sort_with_missing_metrics_and_deep_offsets() {
        let c = Connection::open_in_memory().unwrap();
        let _workspace = Workspace::new(&c).unwrap();
        let mut expected: Vec<_> = (0..5120)
            .map(|i| Row {
                name: format!("/page/{i:05}"),
                visitors: i % 17,
                pageviews: i % 29,
                visits: i % 13,
                bounces: i % 7,
                duration: i % 47,
                exits: i % 11,
                entrances: (i % 2 == 0).then_some(i % 5),
                time_on_page_ms: i % 101 * 1000,
                time_on_page_visits: i % 19,
                total_scroll_depth: i % 71,
                scroll_depth_visits: i % 3,
                unavailable: (i % 128) as u8,
                ..Default::default()
            })
            .collect();
        let stored = Rows::collect(&c, expected.iter().cloned().map(Ok)).unwrap();
        for row in &mut expected {
            row.percentage = Some((row.visitors, 100_000));
        }
        for key in [
            "name",
            "visitors",
            "pageviews",
            "visits",
            "bounce_rate",
            "visit_duration",
            "time_on_page",
            "scroll_depth",
            "exit_rate",
            "percentage",
        ] {
            for ascending in [false, true] {
                let mut sorted = expected.clone();
                crate::reports::sort(&mut sorted, "page", key, ascending);
                for offset in [0, 5000, 1_000_000] {
                    let (actual, count) = stored
                        .select(
                            &c,
                            Selection {
                                ascending,
                                offset,
                                ..selection(key)
                            },
                        )
                        .unwrap();
                    assert_eq!(count, 5120);
                    assert_eq!(
                        actual,
                        sorted
                            .iter()
                            .skip(offset)
                            .take(50)
                            .cloned()
                            .collect::<Vec<_>>(),
                        "{key}/{ascending}/{offset}"
                    );
                }
            }
        }
    }

    #[test]
    fn label_search_uses_display_names_unicode_folding_and_stable_ties() {
        let c = Connection::open_in_memory().unwrap();
        let _workspace = Workspace::new(&c).unwrap();
        for (dimension, names, search) in [
            ("country", vec!["US", "GB", "PH", "DE", "", "ZZ"], "united"),
            (
                "source",
                vec![
                    "google.com",
                    "Google",
                    "news.ycombinator.com",
                    "",
                    "example.org",
                ],
                "google",
            ),
            (
                "page",
                vec!["/CAFÉ", "/café", "/İSTANBUL", "/other", ""],
                "café",
            ),
            (
                "city",
                vec!["US||5128581", "PH||1701668", "GB||2643743", ""],
                "",
            ),
        ] {
            let mut expected: Vec<_> = names
                .iter()
                .map(|name| Row {
                    name: (*name).into(),
                    visitors: 5,
                    pageviews: 7,
                    percentage: Some((5, 100_000)),
                    ..Default::default()
                })
                .collect();
            let stored = Rows::collect(&c, expected.iter().cloned().map(Ok)).unwrap();
            expected.retain(|r| {
                !r.name.is_empty()
                    && crate::report_table::label(dimension, &r.name)
                        .to_lowercase()
                        .contains(search)
            });
            for ascending in [false, true] {
                crate::reports::sort(&mut expected, dimension, "name", ascending);
                let (actual, count) = stored
                    .select(
                        &c,
                        Selection {
                            dimension,
                            ascending,
                            search: Some(search),
                            omit_empty: true,
                            ..selection("name")
                        },
                    )
                    .unwrap();
                assert_eq!(count, expected.len());
                assert_eq!(actual, expected, "{dimension}/{ascending}");
            }
        }
    }

    #[test]
    fn disk_merges_preserve_missing_flags_nullable_entrances_and_weighted_metrics() {
        let c = Connection::open_in_memory().unwrap();
        let _workspace = Workspace::new(&c).unwrap();
        let mut stored = Rows::default();
        let mut expected = Row {
            name: "/page".into(),
            ..Default::default()
        };
        for i in 0..130 {
            let row = Row {
                name: "/page".into(),
                visitors: i,
                pageviews: i + 1,
                visits: i % 5,
                bounces: i % 2,
                duration: i * 3,
                session_visits: (i % 2 == 0).then_some(i % 7),
                session_pageviews: (i % 2 == 0).then_some(i * 2),
                exits: i % 4,
                entrances: (i % 3 == 0).then_some(i),
                unavailable: (i % 128) as u8,
                time_on_page_ms: i * 1000,
                time_on_page_visits: i % 3,
                total_scroll_depth: i * 2,
                scroll_depth_visits: i % 5,
                ..Default::default()
            };
            stored.merge(&c, &row).unwrap();
            crate::import_reports::merge(&mut expected, &row);
        }
        assert_eq!(stored.to_vec(&c).unwrap(), vec![expected]);
        let null = Rows::collect(
            &c,
            [Ok(Row {
                name: "/unknown".into(),
                ..Default::default()
            })],
        )
        .unwrap();
        assert_eq!(null.to_vec(&c).unwrap()[0].entrances, None);
    }
}
