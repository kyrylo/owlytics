//! Filtered traffic and overlapping visits have independent metric populations.
use crate::{
    analytics::{ReportQuery, Row},
    report_accumulator::{Accumulator, Sample},
    report_rows::Rows,
    reports::filters,
};
use rusqlite::{Connection, types::Value};
use std::collections::{BTreeMap, BTreeSet};

type Groups = BTreeMap<String, Rows>;

#[derive(Default)]
pub(crate) struct NativeReport {
    pub event_totals: Row,
    pub events: Groups,
    pub sessions: Groups,
    pub page_engagement: Rows,
    pub daily_engagement: Rows,
    pub calendar_events: Rows,
    pub calendar_sessions: Rows,
    pub calendar_engagement: Rows,
    pub acquisition_cohorts: Groups,
    pub location_cohorts: Groups,
    pub acquisition_incomplete: BTreeSet<String>,
}

// Ordered input keeps each session/page together. A whole measured page visit
// is folded before entering the bounded accumulator, even if its cache spills.
#[derive(Default)]
struct Measurement {
    key: Option<(Option<i64>, String, String)>,
    time: i64,
    depth: i64,
}
impl Measurement {
    fn add(
        &mut self,
        session: Option<i64>,
        name: &str,
        group: &str,
        time: i64,
        depth: i64,
        accumulator: &mut Accumulator<'_>,
        dimension: usize,
    ) -> rusqlite::Result<()> {
        if self
            .key
            .as_ref()
            .is_some_and(|(id, key, tag)| *id != session || key != name || tag != group)
        {
            self.finish(accumulator, dimension)?;
        }
        if self.key.is_none() {
            self.key = Some((session, name.to_owned(), group.to_owned()));
        }
        self.time += time;
        self.depth = self.depth.max(depth);
        Ok(())
    }
    fn finish(
        &mut self,
        accumulator: &mut Accumulator<'_>,
        dimension: usize,
    ) -> rusqlite::Result<()> {
        if let Some((_, name, _)) = self.key.take() {
            accumulator.add(
                dimension,
                &name,
                Sample {
                    engagement: Some((self.time, self.depth)),
                    ..Default::default()
                },
            )?;
            self.time = 0;
            self.depth = 0;
        }
        Ok(())
    }
}

pub(crate) fn build(
    c: &Connection,
    from: &str,
    values: &[Value],
    sessions: crate::timeline::Input<'_>,
    zone: chrono_tz::Tz,
    window: (i64, i64),
    query: &ReportQuery,
    selected: &BTreeSet<&str>,
) -> rusqlite::Result<NativeReport> {
    let fixed: BTreeMap<_, _> = filters(query)
        .into_iter()
        .map(|(key, _, value)| (key, value))
        .collect();
    let event_columns: Vec<_> = [("total", "day"), ("page", "path")]
        .into_iter()
        .filter(|(dimension, _)| selected.contains(dimension) && !fixed.contains_key(dimension))
        .collect();
    let session_columns: Vec<_> = selected
        .iter()
        .copied()
        .filter(|key| *key != "page")
        .map(|key| {
            (
                key,
                match key {
                    "total" => "last_at",
                    "entry" => "entry_page",
                    "exit" => "exit_page",
                    other => other,
                },
            )
        })
        .collect();
    let mut names = event_columns
        .iter()
        .map(|(key, _)| format!("event:{key}"))
        .chain(
            session_columns
                .iter()
                .map(|(key, _)| format!("session:{key}")),
        )
        .collect::<Vec<_>>();
    let page_index = names.len();
    names.push("engagement:page".into());
    let day_index = names.len();
    names.push("engagement:day".into());
    let cohort_keys: Vec<_> = selected
        .iter()
        .copied()
        .filter(|key| {
            query.scope != crate::analytics::ReportScope::Suggestions
                && (crate::acquisition::is_dimension(key)
                    || crate::location_reports::is_dimension(key))
        })
        .collect();
    let cohort_index = names.len();
    names.extend(cohort_keys.iter().map(|key| format!("cohort:{key}")));
    let total_index = names.len();
    names.push("event:range".into());
    let calendar = query.scope != crate::analytics::ReportScope::Suggestions;
    let calendar_last_day = chrono::DateTime::from_timestamp(window.1 - 1, 0)
        .expect("calendar window")
        .with_timezone(&zone)
        .date_naive()
        .to_string();
    let calendar_index = names.len();
    if calendar {
        names.extend(
            [
                "calendar:events",
                "calendar:sessions",
                "calendar:engagement",
            ]
            .map(str::to_owned),
        );
    }
    let mut calendar_keys = BTreeMap::<String, String>::new();
    let engagement = selected.contains("page") || query.has_filter("page");
    let mut accumulator = Accumulator::new(c, names.iter().map(String::as_str).collect());
    let fields = event_columns
        .iter()
        .map(|(key, column)| {
            format!(
                ",{}.{column}",
                if crate::acquisition::is_dimension(key) {
                    "a"
                } else {
                    "e"
                }
            )
        })
        .chain(cohort_keys.iter().map(|key| {
            format!(
                ",{}.{key}",
                if crate::acquisition::is_dimension(key) {
                    "a"
                } else {
                    "e"
                }
            )
        }))
        .collect::<String>();
    let (from, measured_fields, order) = if engagement {
        (
            from.replacen(
                " WHERE ",
                " LEFT JOIN page_engagement g ON g.event_id=e.id WHERE ",
                1,
            ),
            ",e.path,e.day,g.time_ms,g.scroll_depth",
            ",e.path,e.day",
        )
    } else {
        (from.to_owned(), "", "")
    };
    let sql = format!(
        "SELECT e.visitor,e.session_id{fields}{measured_fields} {from} ORDER BY e.visitor,e.session_id{order}"
    );
    let mut statement = c.prepare(&sql)?;
    let mut input = statement.query(rusqlite::params_from_iter(values))?;
    let mut page_measurement = Measurement::default();
    let mut daily_measurement = Measurement::default();
    let mut calendar_measurement = Measurement::default();
    let mut incomplete = BTreeSet::new();
    while let Some(row) = input.next()? {
        let visitor: [u8; 16] = row.get(0)?;
        let session: Option<i64> = row.get(1)?;
        accumulator.add(
            total_index,
            "",
            Sample {
                visitor: Some(visitor),
                session,
                pageviews: 1,
                ..Default::default()
            },
        )?;
        for (index, (dimension, _)) in event_columns.iter().enumerate() {
            let value = row.get_ref(2 + index)?;
            if value == rusqlite::types::ValueRef::Null {
                continue;
            }
            if calendar && *dimension == "total" && value.as_str()? <= calendar_last_day.as_str() {
                let day = value.as_str()?;
                let name = calendar_keys
                    .entry(day.into())
                    .or_insert_with(|| crate::calendar::key(query, day));
                accumulator.add(
                    calendar_index,
                    name,
                    Sample {
                        visitor: Some(visitor),
                        session,
                        pageviews: 1,
                        ..Default::default()
                    },
                )?;
            }
            accumulator.add(
                index,
                value.as_str()?,
                Sample {
                    visitor: Some(visitor),
                    session: if matches!(*dimension, "page" | "total") {
                        session
                    } else {
                        None
                    },
                    pageviews: 1,
                    ..Default::default()
                },
            )?;
        }
        // Percentages use the in-window event audience even when their row
        // counts include visits spanning the window without a pageview.
        for (index, key) in cohort_keys.iter().enumerate() {
            let value = row.get_ref(2 + event_columns.len() + index)?;
            if value != rusqlite::types::ValueRef::Null
                && if crate::acquisition::is_dimension(key) {
                    !value.as_str()?.is_empty()
                } else {
                    !crate::location_reports::excluded(key).contains(&value.as_str()?)
                }
            {
                accumulator.add(
                    cohort_index + index,
                    "",
                    Sample {
                        visitor: Some(visitor),
                        pageviews: 1,
                        ..Default::default()
                    },
                )?;
            }
        }
        if engagement {
            let index = 2 + event_columns.len() + cohort_keys.len();
            if let Some(time) = row.get::<_, Option<i64>>(index + 2)? {
                let depth = row.get(index + 3)?;
                if selected.contains("page") {
                    page_measurement.add(
                        session,
                        row.get_ref(index)?.as_str()?,
                        "",
                        time,
                        depth,
                        &mut accumulator,
                        page_index,
                    )?;
                }
                // Order by page, then day: each measured page visit has its own denominator.
                if query.has_filter("page") {
                    daily_measurement.add(
                        session,
                        row.get_ref(index + 1)?.as_str()?,
                        row.get_ref(index)?.as_str()?,
                        time,
                        depth,
                        &mut accumulator,
                        day_index,
                    )?;
                    if calendar && row.get_ref(index + 1)?.as_str()? <= calendar_last_day.as_str() {
                        let day = row.get_ref(index + 1)?.as_str()?;
                        let name = calendar_keys
                            .entry(day.into())
                            .or_insert_with(|| crate::calendar::key(query, day));
                        calendar_measurement.add(
                            session,
                            name,
                            row.get_ref(index)?.as_str()?,
                            time,
                            depth,
                            &mut accumulator,
                            calendar_index + 2,
                        )?;
                    }
                }
            }
        }
    }
    drop(input);
    drop(statement);
    page_measurement.finish(&mut accumulator, page_index)?;
    daily_measurement.finish(&mut accumulator, day_index)?;
    if calendar {
        calendar_measurement.finish(&mut accumulator, calendar_index + 2)?;
    }
    // All visit breakdowns include overlapping visits, including intervals
    // without a pageview. Page filters still require an in-window event via
    // the caller's membership predicate. Headlines and top pages stay event-based.
    let fields = session_columns
        .iter()
        .map(|(key, column)| {
            format!(
                ",{}.{column}",
                if crate::acquisition::is_dimension(key) {
                    "a"
                } else {
                    "s"
                }
            )
        })
        .collect::<String>();
    let sql = format!(
        "SELECT s.visitor,s.id,s.pageviews,s.last_at-s.started_at,s.entry_page{fields} {} ORDER BY s.visitor,s.id",
        sessions.from
    );
    let mut statement = c.prepare(&sql)?;
    let mut input = statement.query(rusqlite::params_from_iter(sessions.values))?;
    while let Some(row) = input.next()? {
        let visitor: [u8; 16] = row.get(0)?;
        let pageviews: i64 = row.get(2)?;
        let entry = row.get_ref(4)?.as_str()?;
        let entrance = query
            .has_filter("page")
            .then(|| i64::from(query.matches_filter("page", entry)));
        for (index, (dimension, _)) in session_columns.iter().enumerate() {
            let value = row.get_ref(5 + index)?;
            if cohort_keys.contains(dimension) && value == rusqlite::types::ValueRef::Null {
                incomplete.insert((*dimension).to_owned());
            }
            if value == rusqlite::types::ValueRef::Null {
                continue;
            }
            let date;
            let name = if *dimension == "total" {
                let last = value.as_i64()?;
                date = chrono::DateTime::from_timestamp(last, 0)
                    .expect("stored event timestamp")
                    .with_timezone(&zone)
                    .date_naive()
                    .to_string();
                if calendar && last >= window.0 && last < window.1 {
                    let name = calendar_keys
                        .entry(date.clone())
                        .or_insert_with(|| crate::calendar::key(query, &date));
                    accumulator.add(
                        calendar_index + 1,
                        name,
                        Sample {
                            visitor: Some(visitor),
                            visits: 1,
                            pageviews,
                            bounces: i64::from(pageviews == 1 && entrance != Some(0)),
                            duration: row.get(3)?,
                            entrances: entrance,
                            ..Default::default()
                        },
                    )?;
                }
                &date
            } else {
                value.as_str()?
            };
            accumulator.add(
                event_columns.len() + index,
                name,
                Sample {
                    visitor: (*dimension != "total").then_some(visitor),
                    pageviews,
                    visits: 1,
                    bounces: i64::from(pageviews == 1 && entrance != Some(0)),
                    duration: row.get(3)?,
                    entrances: entrance,
                    ..Default::default()
                },
            )?;
        }
    }
    drop(input);
    drop(statement);
    let mut report = NativeReport::default();
    report.acquisition_incomplete = incomplete;
    for (key, rows) in accumulator.finish()? {
        if key == "event:range" {
            report.event_totals = rows.to_vec(c)?.into_iter().next().unwrap_or_default();
        } else if let Some(key) = key.strip_prefix("event:") {
            report.events.insert(key.into(), rows);
        } else if let Some(key) = key.strip_prefix("session:") {
            report.sessions.insert(key.into(), rows);
        } else if key == "engagement:page" {
            report.page_engagement = rows;
        } else if key == "engagement:day" {
            report.daily_engagement = rows;
        } else if key == "calendar:events" {
            report.calendar_events = rows;
        } else if key == "calendar:sessions" {
            report.calendar_sessions = rows;
        } else if key == "calendar:engagement" {
            report.calendar_engagement = rows;
        } else if let Some(key) = key.strip_prefix("cohort:") {
            if crate::location_reports::is_dimension(key) {
                report.location_cohorts.insert(key.into(), rows);
            } else {
                report.acquisition_cohorts.insert(key.into(), rows);
            }
        }
    }
    let Row {
        visitors,
        pageviews,
        visits,
        ..
    } = report.event_totals;
    if pageviews > 0
        && selected.contains("page")
        && let Some(value) = fixed.get("page")
    {
        report.events.insert(
            "page".into(),
            Rows::collect(
                c,
                [Ok(Row {
                    name: (**value).into(),
                    visitors,
                    pageviews,
                    visits,
                    ..Default::default()
                })],
            )?,
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::DIMENSIONS;

    #[tokio::test]
    async fn independent_populations_match_sql_groups() {
        let db = crate::db::Database::open(":memory:").unwrap();
        db.call(|c| {
            c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0);")?;
            for site in 1..=2 {
                for visitor in 0_u128..6 {
                    for visit in 0..2 {
                        for (step,path) in ["/a","/b","/a","/c","/a"].into_iter().enumerate() {
                            let stamp=1793059200+visit*3600+visitor as i64*3+step as i64*30;
                            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?1,?2,'2026-10-27',?3,randomblob(16),?4,'Google',?5,'Chrome','Desktop')",
                                rusqlite::params![site,stamp,visitor.to_be_bytes().as_slice(),path,if (visitor+step as u128)%2==0 {"US"} else {"PH"}])?;
                            if step!=3 {
                                c.execute("INSERT INTO page_engagement VALUES(last_insert_rowid(),?1,?2)",rusqlite::params![step as i64*1000,[30,80,10,0,90][step]])?;
                            }
                        }
                    }
                }
            }
            c.execute("INSERT INTO session_acquisition SELECT id,'Organic Search','Google','organic','A','','',NULL FROM sessions WHERE id%2=0",[])?;
            let selected = DIMENSIONS.iter().map(|(key,_)|*key).chain(["total","entry","exit"]).collect();
            let base = ReportQuery {browser:Some("Chrome".into()),..Default::default()};
            for (query, event_filter, session_filter) in [
                (base.clone(), "1", "1"),
                (ReportQuery {country:Some("PH".into()),..base.clone()}, "e.country='PH'", "s.country='PH'"),
                (ReportQuery {page:Some("/a".into()),..base.clone()}, "e.path='/a'", "1"),
                (ReportQuery {page:Some("/a".into()),country:Some("PH".into()),..base.clone()}, "e.path='/a' AND e.country='PH'", "s.country='PH'"),
                (ReportQuery {entry:Some("/a".into()),exit:Some("/a".into()),..base.clone()}, "s.entry_page='/a' AND s.exit_page='/a'", "s.entry_page='/a' AND s.exit_page='/a'"),
                (ReportQuery {window:Some((1793059235,1793062885)),..base.clone()}, "e.received_at>=1793059235 AND e.received_at<1793062885", "s.last_at>=1793059235 AND s.started_at<1793062885"),
            ] {
                let from=format!("FROM events e LEFT JOIN sessions s ON s.id=e.session_id LEFT JOIN session_acquisition a ON a.session_id=e.session_id WHERE e.site_id=1 AND e.browser='Chrome' AND {event_filter}");
                let predicate=format!("s.site_id=1 AND s.browser='Chrome' AND {session_filter}");
                let _workspace=crate::report_rows::Workspace::new(c)?;
                let membership = if query.has_filter("page") { format!(" AND s.id IN(SELECT e.session_id {from})") } else { String::new() };
                let session_from = format!("FROM sessions s LEFT JOIN session_acquisition a ON a.session_id=s.id WHERE {predicate}{membership}");
                let actual=build(c,&from,&[],crate::timeline::Input {from:&session_from, values:&[]},chrono_tz::UTC,query.window.unwrap_or((1793059200,1793145600)),&query,&selected)?;
                let expected_total = c.query_row(&format!("SELECT COUNT(DISTINCT e.visitor),COUNT(*),COUNT(DISTINCT e.session_id) {from}"),[],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?;
                assert_eq!((actual.event_totals.visitors,actual.event_totals.pageviews,actual.event_totals.visits),expected_total);
                let sorted=|mut rows:Vec<Row>| {rows.sort_by(|a,b|a.name.cmp(&b.name));rows};
                for (key,column) in [("total","day"),("page","path")] {
                    let visits=if matches!(key,"page"|"total") {"COUNT(DISTINCT e.session_id)"} else {"0"};
                    let table=if crate::acquisition::is_dimension(key) {"a"} else {"e"};
                    let sql=format!("SELECT {table}.{column},COUNT(DISTINCT e.visitor),COUNT(*),{visits} {from} AND {table}.{column} IS NOT NULL GROUP BY {table}.{column}");
                    let expected=c.prepare(&sql)?.query_map([],|r|Ok(Row{name:r.get(0)?,visitors:r.get(1)?,pageviews:r.get(2)?,visits:r.get(3)?,..Default::default()}))?.collect::<Result<Vec<_>,_>>()?;
                    assert_eq!(sorted(actual.events.get(key).copied().unwrap_or_default().to_vec(c)?),sorted(expected),"events/{key}/{event_filter}");
                }
                for key in &selected {
                    if *key=="page" {continue;}
                    let column=match *key {"total"=>"day","entry"=>"entry_page","exit"=>"exit_page",other=>other};
                    let counts=*key!="total";
                    let visitors=if counts {"COUNT(DISTINCT s.visitor)"} else {"0"};
                    let pageviews="SUM(s.pageviews)";
                    let entrance=if query.has_filter("page") {"s.entry_page='/a'"} else {"1"};
                    let entrances=if query.has_filter("page") {"SUM(s.entry_page='/a')"} else {"NULL"};
                    let table=if crate::acquisition::is_dimension(key) {"a"} else {"s"};
                    let sql=format!("SELECT {table}.{column},{visitors},{pageviews},COUNT(*),SUM(s.pageviews=1 AND {entrance}),SUM(s.last_at-s.started_at),{entrances} {session_from} AND {table}.{column} IS NOT NULL GROUP BY {table}.{column}");
                    let expected=c.prepare(&sql)?.query_map([],|r|Ok(Row{name:r.get(0)?,visitors:r.get(1)?,pageviews:r.get(2)?,visits:r.get(3)?,bounces:r.get(4)?,duration:r.get(5)?,entrances:r.get(6)?,..Default::default()}))?.collect::<Result<Vec<_>,_>>()?;
                    assert_eq!(sorted(actual.sessions.get(*key).copied().unwrap_or_default().to_vec(c)?),sorted(expected),"sessions/{key}/{event_filter}");
                }
                for (column,actual) in [("path",&actual.page_engagement),("day",&actual.daily_engagement)] {
                    if column=="day" && !query.has_filter("page") {assert!(actual.to_vec(c)?.is_empty());continue;}
                    let measured_from=from.replacen(" WHERE "," JOIN page_engagement g ON g.event_id=e.id WHERE ",1);
                    let sql=format!("SELECT name,SUM(time),COUNT(*),SUM(depth) FROM (SELECT e.{column} name,e.session_id,SUM(g.time_ms) time,MAX(g.scroll_depth) depth {measured_from} GROUP BY e.{column},e.session_id) GROUP BY name");
                    let expected=c.prepare(&sql)?.query_map([],|r|Ok(Row{name:r.get(0)?,time_on_page_ms:r.get(1)?,time_on_page_visits:r.get(2)?,total_scroll_depth:r.get(3)?,scroll_depth_visits:r.get(2)?,..Default::default()}))?.collect::<Result<Vec<_>,_>>()?;
                    assert_eq!(sorted(actual.to_vec(c)?),sorted(expected),"engagement/{column}/{event_filter}");
                }
            }
            Ok(())
        }).await.unwrap();
    }
}
