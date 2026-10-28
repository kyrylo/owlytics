//! Calendar chart buckets retain distinct identities before merging history.
use crate::{
    analytics::{ReportQuery, Row},
    native_report::NativeReport,
};
use chrono::{Datelike, NaiveDate};
use std::collections::BTreeMap;

pub(crate) fn key(query: &ReportQuery, day: &str) -> String {
    interval_key(query.interval.as_deref(), day)
}

pub(crate) fn interval_key(interval: Option<&str>, day: &str) -> String {
    let date: NaiveDate = day.parse().expect("stored calendar date");
    match interval {
        Some("week") => date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64),
        Some("month") => date.with_day(1).unwrap(),
        _ => date,
    }
    .to_string()
}

// A graph requests one metric; CSV requests event and session metrics together.
// Minute counts always use active visits. At coarser intervals a page filter
// chooses events unless an entry/exit filter requires the session population.
pub(crate) fn event_counts(query: &ReportQuery, step: i64) -> bool {
    step != 60
        && (query.export
            || (query.has_filter("page")
                && !query.has_filter("entry")
                && !query.has_filter("exit")))
}

pub(crate) type Days = BTreeMap<String, Row>;

pub(crate) fn native(
    c: &rusqlite::Connection,
    report: &NativeReport,
    query: &ReportQuery,
) -> rusqlite::Result<Days> {
    let mut result = Days::new();
    report.calendar_events.for_each(c, |row| {
        let name = row.name.clone();
        result.insert(
            name,
            Row {
                session_visits: Some(0),
                session_pageviews: Some(0),
                ..row
            },
        );
        Ok(())
    })?;
    let events = event_counts(query, 86400);
    if !events {
        for row in result.values_mut() {
            row.visitors = 0;
            row.visits = 0;
        }
    }
    report.calendar_sessions.for_each(c, |row| {
        let day = result.entry(row.name.clone()).or_insert_with(|| Row {
            name: row.name.clone(),
            ..Default::default()
        });
        if !events {
            day.visitors = row.visitors;
            day.visits = row.visits;
        }
        day.session_visits = Some(row.visits);
        day.session_pageviews = Some(row.pageviews);
        day.bounces = row.bounces;
        day.duration = row.duration;
        day.entrances = row.entrances;
        Ok(())
    })?;
    report.calendar_engagement.for_each(c, |row| {
        result
            .entry(row.name.clone())
            .or_insert_with(|| Row {
                name: row.name.clone(),
                session_visits: Some(0),
                session_pageviews: Some(0),
                ..Default::default()
            })
            .add_engagement(&row);
        Ok(())
    })?;
    Ok(result)
}

pub(crate) fn merge(days: &mut Days, query: &ReportQuery, row: &Row) {
    let name = key(query, &row.name);
    days.entry(name.clone())
        .or_insert_with(|| Row {
            name,
            ..Default::default()
        })
        .add(row);
}

pub(crate) fn from_daily(query: &ReportQuery, daily: &[Row], through: &str) -> Vec<Row> {
    let mut days = Days::new();
    for row in daily.iter().filter(|row| row.name.as_str() <= through) {
        merge(&mut days, query, row);
    }
    days.into_values().collect()
}
