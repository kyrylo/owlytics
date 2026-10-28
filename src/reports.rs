use crate::{
    analytics::{Range, Report, ReportQuery, ReportScope, Row},
    db::{Database, Error},
    report_rows::{Rows, Selection, Workspace},
};
use rusqlite::{Connection, params, types::Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const DIMENSIONS: &[(&str, &str)] = &[
    ("page", "path"),
    ("source", "source"),
    ("referrer", "referrer"),
    ("channel", "channel"),
    ("utm_medium", "utm_medium"),
    ("utm_source", "utm_source"),
    ("utm_campaign", "utm_campaign"),
    ("utm_content", "utm_content"),
    ("utm_term", "utm_term"),
    ("country", "country"),
    ("region", "region"),
    ("city", "city"),
    ("browser", "browser"),
    ("browser_version", "browser_version"),
    ("os", "os"),
    ("os_version", "os_version"),
    ("device", "device"),
];

pub(crate) fn filters(query: &ReportQuery) -> Vec<(&'static str, &'static str, &str)> {
    crate::filters::DIMENSIONS
        .iter()
        .filter_map(|(key, column, _)| query.exact_filter(key).map(|value| (*key, *column, value)))
        .collect()
}

fn rows(c: &Connection, sql: &str, values: &[Value]) -> rusqlite::Result<Vec<Row>> {
    c.prepare(sql)?
        .query_map(rusqlite::params_from_iter(values), |r| {
            Ok(Row {
                name: r.get(0)?,
                visitors: r.get(1)?,
                pageviews: r.get(2)?,
                ..Default::default()
            })
        })?
        .collect()
}

fn engagement_rows(c: &Connection, base: &[Value]) -> rusqlite::Result<Rows> {
    Rows::from_query(
        c,
        "name,time_on_page_ms,time_on_page_visits,total_scroll_depth,scroll_depth_visits",
        "SELECT path,SUM(time_ms),SUM(visits),SUM(scroll_depth),SUM(visits) FROM daily_engagement WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 GROUP BY path",
        base,
    )
}
pub(crate) const PAGE_SIZE: usize = 50;

#[cfg(test)]
pub(crate) fn sort(rows: &mut [Row], dimension: &str, key: &str, ascending: bool) {
    if key == "name" {
        // Resolve labels once. Sorting country codes or GeoNames IDs would
        // disagree with the names the owner sees and searches in the table.
        use std::cmp::Reverse;
        if ascending {
            rows.sort_by_cached_key(|row| {
                (
                    crate::report_table::label(dimension, &row.name).to_lowercase(),
                    Reverse(row.pageviews),
                    row.name.clone(),
                )
            });
        } else {
            rows.sort_by_cached_key(|row| {
                (
                    Reverse(crate::report_table::label(dimension, &row.name).to_lowercase()),
                    Reverse(row.pageviews),
                    row.name.clone(),
                )
            });
        }
        return;
    }
    rows.sort_by(|a, b| {
        let order = match (
            crate::report_table::value(a, key),
            crate::report_table::value(b, key),
        ) {
            (Some(a), Some(b)) => {
                if ascending {
                    a.total_cmp(&b)
                } else {
                    b.total_cmp(&a)
                }
            }
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        };
        order
            .then_with(|| b.pageviews.cmp(&a.pageviews))
            .then_with(|| a.name.cmp(&b.name))
    });
}

type Groups = BTreeMap<String, Rows>;

// A navigation request only needs its visible tabs. The comparison needs the
// headline metrics/chart plus exact names requested by the primary report;
// downloads need every dimension. Keep the
// page dependencies so entrance bounce rates and exit denominators are exact.
fn dimensions(query: &ReportQuery) -> BTreeSet<&'static str> {
    let choose = |value: Option<&str>, allowed: &[&'static str], default: &'static str| {
        allowed
            .iter()
            .copied()
            .find(|key| Some(*key) == value)
            .unwrap_or(default)
    };
    let mut selected = match query.scope {
        ReportScope::All => DIMENSIONS
            .iter()
            .map(|(key, _)| *key)
            .chain(["entry", "exit"])
            .collect(),
        ReportScope::Timeline => BTreeSet::new(),
        ReportScope::Suggestions => crate::filters::DIMENSIONS
            .iter()
            .filter_map(|(key, _, _)| (query.filter.as_deref() == Some(*key)).then_some(*key))
            .collect(),
        ReportScope::Dashboard => {
            let device_default = if query.exact_filter("browser").is_some() {
                "browser_version"
            } else {
                "browser"
            };
            let device = query.devices_tab.as_deref().unwrap_or(device_default);
            let device = match device {
                "browser" if query.exact_filter("browser").is_some() => "browser_version",
                "os" if query.exact_filter("os").is_some() => "os_version",
                other => other,
            };
            let device = choose(
                Some(device),
                &[
                    device_default,
                    if query.exact_filter("os").is_some() {
                        "os_version"
                    } else {
                        "os"
                    },
                    "device",
                ],
                device_default,
            );
            let detail = DIMENSIONS
                .iter()
                .map(|(key, _)| *key)
                .chain(["entry", "exit"])
                .find(|key| query.detail.as_deref() == Some(*key));
            [
                query.source_tab(),
                choose(
                    query.pages_tab.as_deref(),
                    &["page", "entry", "exit"],
                    "page",
                ),
                match query.location_tab() {
                    "region" => "region",
                    "city" => "city",
                    _ => "country",
                },
                device,
            ]
            .into_iter()
            .chain(detail)
            .collect()
        }
    };
    if let Some(names) = &query.comparison_names {
        selected = DIMENSIONS
            .iter()
            .map(|(key, _)| *key)
            .chain(["entry", "exit"])
            .filter(|key| names.contains_key(*key))
            .collect();
    }
    if selected.contains("exit") {
        selected.insert("page");
    }
    if selected.contains("page") {
        selected.insert("entry");
    }
    selected.insert("total");
    selected
}

// Main files stay read-only on pooled reader connections. query_only is relaxed
// only for bounded aggregate spills, then restored even on an error or panic.
// Spill-table DDL belongs to the report transaction and rolls back with it.
struct TempWrites<'a> {
    connection: &'a Connection,
    query_only: bool,
}
impl<'a> TempWrites<'a> {
    fn new(connection: &'a Connection) -> rusqlite::Result<Self> {
        let query_only = connection.pragma_query_value(None, "query_only", |r| r.get(0))?;
        connection.pragma_update(None, "query_only", false)?;
        Ok(Self {
            connection,
            query_only,
        })
    }
}
impl Drop for TempWrites<'_> {
    fn drop(&mut self) {
        let _ = self
            .connection
            .pragma_update(None, "query_only", self.query_only);
    }
}

fn session_groups(
    c: &Connection,
    values: &[Value],
    selected: &BTreeSet<&str>,
) -> rusqlite::Result<Groups> {
    let mut result = Groups::new();
    let mut arguments = values.to_vec();
    let mut choices = Vec::new();
    let mut keys = Vec::new();
    for dimension in selected {
        let rows = Rows::new(c)?;
        arguments.extend([Value::Text((*dimension).into()), Value::Integer(rows.id())]);
        keys.push(format!("?{}", arguments.len() - 1));
        choices.push(format!(
            "WHEN ?{} THEN ?{}",
            arguments.len() - 1,
            arguments.len()
        ));
        result.insert((*dimension).into(), rows);
    }
    // Map each grouped dimension to its result set after the scan. Joining a
    // VALUES table before grouping makes SQLite rescan the date range per tab.
    c.execute(&format!("INSERT INTO temp.report_rows(set_id,name,visitors,pageviews,visits,bounces,duration) SELECT CASE dimension {} END,name,visitors,pageviews,visits,bounces,duration FROM (SELECT dimension,CASE WHEN dimension='total' THEN day ELSE value END AS name,SUM(visitors) AS visitors,SUM(pageviews) AS pageviews,SUM(visits) AS visits,SUM(bounces) AS bounces,SUM(duration) AS duration FROM daily_session_reports WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 AND dimension IN({}) GROUP BY dimension,name)",choices.join(" "),keys.join(",")),rusqlite::params_from_iter(&arguments))?;
    Ok(result)
}

// Visit attributes are inherited by every pageview. Their daily summaries can
// supply headline-only calendar requests; page/entry/exit filters still need
// their different event and session predicates.
fn summary_filter<'a>(range: &Range, query: &'a ReportQuery) -> Option<(&'static str, &'a str)> {
    // Single-dimension summaries contain headline totals, not the intersections
    // needed to compare rows from other dimensions within this audience.
    if query
        .comparison_names
        .as_ref()
        .is_some_and(|names| !names.is_empty())
    {
        return None;
    }
    if query.scope != ReportScope::Timeline
        || query.window.is_some()
        || matches!(query.interval.as_deref(), Some("hour" | "minute"))
        || (query.interval.is_none() && range.from == range.to)
    {
        return None;
    }
    if query.active_filters().len() != 1 {
        return None;
    }
    let filters = filters(query);
    match filters.as_slice() {
        [(key, _, value)] if !matches!(*key, "page" | "entry" | "exit") => Some((key, value)),
        _ => None,
    }
}

fn summary_days(
    c: &Connection,
    base: &[Value],
    dimension: &str,
    value: &str,
) -> rusqlite::Result<Vec<Row>> {
    let mut values = base.to_vec();
    values.extend([dimension.to_owned().into(), value.to_owned().into()]);
    c.prepare("SELECT d.day,d.visitors,d.pageviews,COALESCE(s.visits,0),COALESCE(s.bounces,0),COALESCE(s.duration,0),COALESCE(s.pageviews,0) FROM daily_reports d LEFT JOIN daily_session_reports s USING(site_id,day,dimension,value) WHERE d.site_id=?1 AND d.day BETWEEN ?2 AND ?3 AND d.dimension=?4 AND d.value=?5 ORDER BY d.day")?
        .query_map(rusqlite::params_from_iter(&values), |r| Ok(Row {
            name:r.get(0)?, visitors:r.get(1)?, pageviews:r.get(2)?, visits:r.get(3)?, bounces:r.get(4)?, duration:r.get(5)?, session_visits:Some(r.get(3)?), session_pageviews:Some(r.get(6)?), ..Default::default()
        }))?.collect()
}

// Missing first-touch data matters only for visits in the requested audience
// and time window. Remove campaign predicates: unknown tags cannot match them.
fn missing_acquisition(
    c: &Connection,
    base: &[Value],
    query: &ReportQuery,
    window: (i64, i64),
) -> rusqlite::Result<bool> {
    let mut values = base.to_vec();
    let events = query.has_filter("page");
    let table = if events { "e" } else { "s" };
    let mut predicate = format!(
        "{table}.site_id=?1 AND {table}.day BETWEEN {} AND ?3 AND a.session_id IS NULL",
        if events { "?2" } else { "date(?2,'-8 days')" }
    );
    for filter in query
        .active_filters()
        .into_iter()
        .filter(|f| !crate::acquisition::is_dimension(&f.key))
    {
        let (_, column, _) = crate::filters::DIMENSIONS
            .iter()
            .find(|(key, _, _)| *key == filter.key)
            .expect("validated filter");
        let table = if filter.key == "page" { "e" } else { "s" };
        predicate.push_str(&format!(
            " AND {}",
            filter.sql(&format!("{table}.{column}"), &mut values)
        ));
    }
    values.extend([window.0.into(), window.1.into()]);
    let (start, end) = (values.len() - 1, values.len());
    predicate.push_str(&format!(
        " AND ?{start}<?{end} AND s.started_at>=?{start}-604800 AND s.last_at>=?{start} AND s.started_at<?{end}"
    ));
    if events {
        predicate.push_str(&format!(
            " AND e.received_at>=?{start} AND e.received_at<?{end}"
        ));
    }
    let from = if events {
        "events e JOIN sessions s ON s.id=e.session_id"
    } else {
        "sessions s"
    };
    c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {from} LEFT JOIN session_acquisition a ON a.session_id=s.id WHERE {predicate})"), rusqlite::params_from_iter(values), |row| row.get(0))
}

pub async fn report(
    db: &Database,
    site_id: i64,
    range: Range,
    query: ReportQuery,
) -> Result<Report, Error> {
    query
        .validate_filters()
        .map_err(|message| rusqlite::Error::InvalidParameterName(message.into()))?;
    db.read(move |c| {
        let mut query = query;
        let realtime = query.period.as_deref() == Some("realtime");
        if realtime {
            let now = crate::auth::now();
            query.window.get_or_insert((now - 1800, now + 1));
            query.interval = Some("minute".into());
        }
        let _temporary_writes = TempWrites::new(c)?;
        let tx = c.unchecked_transaction()?;
        let result = if realtime && query.scope != ReportScope::Timeline {
            let (_, end) = query.window.expect("realtime window");
            let mut recent = query.clone();
            recent.window = Some((end - 301, end));
            // Breakdowns and suggestions need five minutes, without a second chart.
            recent.interval = Some("day".into());
            let mut report = build(&tx, site_id, &range, &recent)?;
            if query.scope != ReportScope::Suggestions {
                let mut overview = query.clone();
                overview.scope = ReportScope::Timeline;
                overview.export = false;
                let headline = build(&tx, site_id, &range, &overview)?;
                // Row percentages were already calculated from the five-minute
                // audience. Only the headline and chart use the thirty-minute view.
                report.visitors = headline.visitors;
                report.pageviews = headline.pageviews;
                report.totals = headline.totals;
                report.daily = headline.daily;
                report.calendar = headline.calendar;
                report.timeline = headline.timeline;
                report.timeline_start = headline.timeline_start;
                report.timeline_end = headline.timeline_end;
                report.timeline_step = headline.timeline_step;
                report.current_visitors = headline.current_visitors;
                report
                    .acquisition_incomplete
                    .extend(headline.acquisition_incomplete);
            }
            report
        } else {
            build(&tx, site_id, &range, &query)?
        };
        tx.commit()?;
        Ok(result)
    })
    .await
}

// All components of a report share one read snapshot, including realtime's
// different headline/chart and breakdown windows.
fn build(
    c: &Connection,
    site_id: i64,
    range: &Range,
    query: &ReportQuery,
) -> rusqlite::Result<Report> {
    let _workspace = Workspace::new(c)?;
    let zone: String = c.query_row("SELECT timezone FROM sites WHERE id=?1", [site_id], |r| {
        r.get(0)
    })?;
    let zone = zone.parse::<chrono_tz::Tz>().expect("validated timezone");
    let now = if query.period.as_deref() == Some("realtime") {
        query
            .window
            .map_or_else(crate::auth::now, |(_, end)| end - 1)
    } else {
        crate::auth::now()
    };
    let selected = dimensions(query);
    let from = range.from.to_string();
    let to = range.to.to_string();
    let observed = if query.period.as_deref() == Some("24h") {
        query.window.map_or(now, |(_, end)| end - 1)
    } else {
        now
    };
    let today = chrono::DateTime::from_timestamp(observed, 0)
        .expect("observation time")
        .with_timezone(&zone)
        .date_naive();
    let calendar_through = if query.export {
        range.to
    } else {
        range.to.min(today)
    };
    let base: Vec<Value> = vec![site_id.into(), from.clone().into(), to.clone().into()];
    // Summing daily counters loses identities repeated across dates and visits
    // that overlap midnight. The date-only marker makes the safe cached path
    // cheap without storing more visitor identifiers or scanning all traffic.
    let cross_day: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM report_cross_day_dates WHERE site_id=?1 AND day BETWEEN ?2 AND ?3)",
        rusqlite::params_from_iter(&base), |r| r.get(0),
    )?;
    let summarized = (!cross_day).then(|| summary_filter(range, query)).flatten();
    let raw = (cross_day || query.filtered() || query.window.is_some()) && summarized.is_none();
    let summarized_days = summarized
        .map(|(dimension, value)| summary_days(c, &base, dimension, value))
        .transpose()?;
    let has_imports: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM import_daily WHERE site_id=?1 AND day BETWEEN ?2 AND ?3)",
        params![site_id, from, to],
        |r| r.get(0),
    )?;
    let imports_available: bool = has_imports
        || c.query_row(
            "SELECT EXISTS(SELECT 1 FROM import_daily WHERE site_id=?1)",
            [site_id],
            |r| r.get(0),
        )?;
    let imports_enabled = query.imports_enabled();
    let mut include_imports =
        imports_enabled && has_imports && !query.filtered() && query.window.is_none();
    let filtered_imports =
        if imports_enabled && has_imports && query.filtered() && query.window.is_none() {
            Some(crate::import_reports::build(
                c, &base, &query, &selected, true,
            )?)
        } else {
            None
        };
    let mut event_values = base.clone();
    let mut session_values = base.clone();
    let calendar = (
        crate::timeline::day_start(zone, range.from),
        crate::timeline::day_start(zone, range.to.succ_opt().unwrap()),
    );
    let window = query.window.map_or(calendar, |(start, end)| {
        (start.max(calendar.0), end.min(calendar.1))
    });
    let mut ep = String::from("e.site_id=?1 AND e.day BETWEEN ?2 AND ?3");
    // The date index is only a coarse guard for visits. Exact overlap and the
    // reference's seven-day start bound use UTC instants below. Eight local
    // dates also cover an offset change inside that seven-day lookback.
    let mut sp = String::from("s.site_id=?1 AND s.day BETWEEN date(?2,'-8 days') AND ?3");
    for filter in query.active_filters() {
        let (_, col, _) = crate::filters::DIMENSIONS
            .iter()
            .find(|(key, _, _)| *key == filter.key)
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName("Invalid filter dimension".into())
            })?;
        let acquisition = crate::acquisition::is_dimension(&filter.key);
        let table = if acquisition {
            "a"
        } else if matches!(filter.key.as_str(), "entry" | "exit") {
            "s"
        } else {
            "e"
        };
        ep.push_str(&format!(
            " AND {}",
            filter.sql(&format!("{table}.{col}"), &mut event_values)
        ));
        if filter.key == "page" {
            session_values.extend(filter.values.iter().cloned().map(Value::Text));
        } else {
            sp.push_str(&format!(
                " AND {}",
                filter.sql(
                    &format!("{}.{col}", if acquisition { "a" } else { "s" }),
                    &mut session_values
                )
            ));
        }
    }
    event_values.extend([window.0.into(), window.1.into()]);
    session_values.extend([window.0.into(), window.1.into()]);
    let (start, end) = (event_values.len() - 1, event_values.len());
    ep.push_str(&format!(
        " AND e.received_at>=?{start} AND e.received_at<?{end}"
    ));
    sp.push_str(&format!(
        " AND ?{start}<?{end} AND s.started_at>=?{start}-604800 AND s.last_at>=?{start} AND s.started_at<?{end}"
    ));
    let join = if query.has_filter("entry") || query.has_filter("exit") {
        " JOIN sessions s ON s.id=e.session_id"
    } else {
        ""
    };
    let index = if query.exact_filter("page").is_some() {
        "events_site_path_day"
    } else {
        "events_site_time"
    };
    let acquisition_needed = query
        .active_filters()
        .iter()
        .any(|f| crate::acquisition::is_dimension(&f.key))
        || (raw
            && selected
                .iter()
                .any(|key| crate::acquisition::is_dimension(key)));
    let acquisition_join = if acquisition_needed {
        " LEFT JOIN session_acquisition a ON a.session_id=e.session_id"
    } else {
        ""
    };
    let event_from = format!("FROM events e INDEXED BY {index}{join}{acquisition_join} WHERE {ep}");
    let mut session_from = format!(
        "FROM sessions s{}{} WHERE {sp}",
        if raw && query.has_filter("page") {
            " NOT INDEXED"
        } else {
            ""
        },
        if acquisition_needed {
            " LEFT JOIN session_acquisition a ON a.session_id=s.id"
        } else {
            ""
        }
    );
    let active_session_from = session_from.clone();
    let active_session_values = session_values.clone();
    if raw && query.has_filter("page") {
        // Keep only matching session IDs in SQLite's membership operation;
        // avoid copying complete events and sessions into temporary tables.
        // NOT INDEXED still permits primary-key lookups for selective pages.
        session_from.push_str(&format!(" AND s.id IN(SELECT e.session_id {event_from})"));
        session_values = event_values.clone();
    }
    let mut native = if raw {
        crate::native_report::build(
            c,
            &event_from,
            &event_values,
            crate::timeline::Input {
                from: &session_from,
                values: &session_values,
            },
            zone,
            (
                window.0,
                window.1.min(crate::timeline::day_start(
                    zone,
                    calendar_through.succ_opt().unwrap(),
                )),
            ),
            &query,
            &selected,
        )?
    } else {
        crate::native_report::NativeReport::default()
    };
    let mut calendar = if raw && query.scope != ReportScope::Suggestions {
        Some(crate::calendar::native(c, &native, query)?)
    } else {
        None
    };
    let mut acquisition = if query.scope == ReportScope::Suggestions {
        crate::acquisition_reports::Coverage::default()
    } else if raw {
        let mut coverage = crate::acquisition_reports::Coverage {
            incomplete: std::mem::take(&mut native.acquisition_incomplete),
            ..Default::default()
        };
        for (key, days) in std::mem::take(&mut native.acquisition_cohorts) {
            let (visitors, pageviews, _) = days.counts(c)?;
            coverage.cohorts.insert(
                key,
                Row {
                    visitors,
                    pageviews,
                    ..Default::default()
                },
            );
        }
        coverage
    } else {
        crate::acquisition_reports::native(c, &base, &selected)?
    };
    if query.scope != ReportScope::Suggestions {
        let filtered_keys: Vec<_> = query
            .active_filters()
            .into_iter()
            .filter(|f| crate::acquisition::is_dimension(&f.key))
            .map(|f| f.key)
            .collect();
        if !filtered_keys.is_empty() && missing_acquisition(c, &base, query, window)? {
            acquisition.incomplete.extend(filtered_keys);
        }
    }
    if include_imports && query.scope != ReportScope::Suggestions {
        crate::acquisition_reports::imports(c, &base, &selected, &mut acquisition)?;
    }
    let mut raw_events = std::mem::take(&mut native.events);
    let session_data = if let Some(days) = &summarized_days {
        Groups::from([(
            "total".into(),
            Rows::collect(c, days.iter().cloned().map(Ok))?,
        )])
    } else if raw {
        std::mem::take(&mut native.sessions)
    } else {
        session_groups(c, &base, &selected)?
    };
    let mut daily = if let Some(days) = summarized_days {
        days
    } else if raw {
        raw_events.remove("total").unwrap_or_default().to_vec(c)?
    } else {
        rows(
            c,
            "SELECT day,visitors,pageviews FROM daily_reports WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 AND dimension='total' ORDER BY day",
            &base,
        )?
    };
    let native_daily_visitors: i64 = daily.iter().map(|row| row.visitors).sum();
    let native_denominator = if raw {
        (native.event_totals.visitors, native.event_totals.pageviews)
    } else {
        (
            native_daily_visitors,
            daily.iter().map(|row| row.pageviews).sum(),
        )
    };
    let mut location_cohorts = if query.scope == ReportScope::Suggestions {
        BTreeMap::new()
    } else if raw {
        let mut cohorts = BTreeMap::new();
        for (key, rows) in std::mem::take(&mut native.location_cohorts) {
            let (visitors, pageviews, _) = rows.counts(c)?;
            cohorts.insert(
                key,
                Row {
                    visitors,
                    pageviews,
                    ..Default::default()
                },
            );
        }
        cohorts
    } else {
        crate::location_reports::native(c, &base, &selected, native_denominator)?
    };
    for day in &mut daily {
        day.session_visits = Some(0);
        day.session_pageviews = Some(0);
    }
    let mut by_dimension = BTreeMap::new();
    for (dimension, _) in DIMENSIONS {
        if (raw && *dimension != "page") || !selected.contains(dimension) {
            continue;
        }
        let data = if raw {
            raw_events.remove(*dimension).unwrap_or_default()
        } else {
            let mut values = base.clone();
            values.push((*dimension).to_owned().into());
            Rows::from_query(
                c,
                "name,visitors,pageviews",
                "WITH RECURSIVE dates(day) AS (SELECT ?2 UNION ALL SELECT date(day,'+1 day') FROM dates WHERE day<?3) SELECT r.value,SUM(r.visitors),SUM(r.pageviews) FROM dates CROSS JOIN daily_reports r WHERE r.site_id=?1 AND r.day=dates.day AND r.dimension=?4 GROUP BY r.value",
                &values,
            )?
        };
        by_dimension.insert(*dimension, data);
    }
    if !raw && let Some(pages) = by_dimension.get("page") {
        pages.page_visits(c, session_data.get("page").copied().unwrap_or_default())?;
    }
    let mut totals = Row {
        entrances: query.has_filter("page").then_some(0),
        visits: if raw { native.event_totals.visits } else { 0 },
        session_visits: Some(0),
        session_pageviews: Some(0),
        ..Default::default()
    };
    for dimension in std::iter::once("total")
        .chain(DIMENSIONS.iter().map(|(key, _)| *key))
        .chain(["entry", "exit"])
    {
        if !selected.contains(dimension) {
            continue;
        }
        let key = if dimension == "page" {
            "entry"
        } else {
            dimension
        };
        let session_rows = session_data.get(key).copied().unwrap_or_default();
        if dimension == "total" {
            for row in session_rows.to_vec(c)? {
                if !raw {
                    totals.visits += row.visits;
                }
                *totals.session_visits.as_mut().unwrap() += row.mean_visits();
                *totals.session_pageviews.as_mut().unwrap() += row.mean_pageviews();
                totals.bounces += row.bounces;
                totals.duration += row.duration;
                if let Some(entrances) = row.entrances {
                    *totals.entrances.get_or_insert(0) += entrances;
                }
                if row.name >= from && row.name <= to {
                    if !daily.iter().any(|d| d.name == row.name) {
                        daily.push(Row {
                            name: row.name.clone(),
                            ..Default::default()
                        });
                    }
                    let day = daily.iter_mut().find(|d| d.name == row.name).unwrap();
                    if !raw {
                        day.visits = row.visits;
                    }
                    day.session_visits = Some(row.mean_visits());
                    day.session_pageviews = Some(row.mean_pageviews());
                    day.bounces = row.bounces;
                    day.duration = row.duration;
                    day.entrances = row.entrances;
                }
            }
        } else if matches!(dimension, "entry" | "exit") || (raw && dimension != "page") {
            by_dimension.insert(dimension, session_rows);
        } else if let Some(data) = by_dimension.get(dimension) {
            // Calendar event summaries have the inherited labels and visitor
            // counts; session summaries omit those counts except for entry/exit.
            data.sessions(c, session_rows, dimension == "page")?;
        }
    }
    if let Some(pages) = by_dimension.get("page") {
        let data = if raw {
            native.page_engagement
        } else {
            engagement_rows(c, &base)?
        };
        pages.engagement(c, data)?;
    }
    if query.has_filter("page") {
        for row in native.daily_engagement.to_vec(c)? {
            totals.add_engagement(&row);
            if let Some(day) = daily.iter_mut().find(|d| d.name == row.name) {
                day.add_engagement(&row);
            }
        }
    }
    let mut breakdown_pageviews = false;
    let mut session_metrics_available = true;
    if include_imports {
        breakdown_pageviews=c.query_row("SELECT EXISTS(SELECT 1 FROM import_breakdowns b LEFT JOIN import_metrics m USING(site_id,day,dimension,value) WHERE b.site_id=?1 AND b.day BETWEEN ?2 AND ?3 AND m.visitors IS NULL)",rusqlite::params_from_iter(&base),|r|r.get(0))?;
        session_metrics_available=!c.query_row("SELECT EXISTS(SELECT 1 FROM import_daily d LEFT JOIN import_metrics m ON m.site_id=d.site_id AND m.day=d.day AND m.dimension='total' AND m.value='' WHERE d.site_id=?1 AND d.day BETWEEN ?2 AND ?3 AND (m.visits IS NULL OR m.bounces IS NULL OR m.duration IS NULL))",rusqlite::params_from_iter(&base),|r|r.get::<_,bool>(0))?;
        let imported=c.prepare("SELECT d.day,d.visitors,d.pageviews,COALESCE(m.visits,0),COALESCE(m.bounces,0),COALESCE(m.duration,0),2*(m.visits IS NULL)+4*(m.bounces IS NULL)+8*(m.duration IS NULL) FROM import_daily d LEFT JOIN import_metrics m ON m.site_id=d.site_id AND m.day=d.day AND m.dimension='total' AND m.value='' WHERE d.site_id=?1 AND d.day BETWEEN ?2 AND ?3")?
                .query_map(rusqlite::params_from_iter(&base),|r|Ok(Row{name:r.get(0)?,visitors:r.get(1)?,pageviews:r.get(2)?,visits:r.get(3)?,bounces:r.get(4)?,duration:r.get(5)?,unavailable:r.get(6)?,..Default::default()}))?.collect::<Result<Vec<_>,_>>()?;
        for row in &imported {
            totals.add(row);
            if let Some(calendar) = &mut calendar
                && row.name <= calendar_through.to_string()
            {
                crate::calendar::merge(calendar, query, row);
            }
            if let Some(day) = daily.iter_mut().find(|day| day.name == row.name) {
                day.add(row);
            } else {
                daily.push(row.clone());
            }
        }
        daily.sort_by(|a, b| a.name.cmp(&b.name));
        let mut statement=c.prepare("SELECT b.dimension,b.value,SUM(COALESCE(m.visitors,0)),SUM(b.pageviews),SUM(COALESCE(m.visits,0)),SUM(COALESCE(m.bounces,0)),SUM(COALESCE(m.duration,0)),SUM(COALESCE(m.exits,0)),MAX(m.visitors IS NULL)+2*MAX(m.visits IS NULL)+4*MAX(m.bounces IS NULL)+8*MAX(m.duration IS NULL)+16*MAX(m.exits IS NULL)+32*MAX(b.dimension='page' AND (g.time_ms IS NULL OR g.time_visits IS NULL))+64*MAX(b.dimension='page' AND (g.scroll_depth IS NULL OR g.scroll_visits IS NULL)),SUM(COALESCE(g.time_ms,0)),SUM(COALESCE(g.time_visits,0)),SUM(COALESCE(g.scroll_depth,0)),SUM(COALESCE(g.scroll_visits,0)) FROM import_breakdowns b LEFT JOIN import_metrics m USING(site_id,day,dimension,value) LEFT JOIN import_engagement g ON b.dimension='page' AND g.site_id=b.site_id AND g.day=b.day AND g.path=b.value WHERE b.site_id=?1 AND b.day BETWEEN ?2 AND ?3 GROUP BY b.dimension,b.value")?;
        let mut results = statement.query(rusqlite::params_from_iter(&base))?;
        while let Some(r) = results.next()? {
            let dimension: String = r.get(0)?;
            if !selected.contains(dimension.as_str()) {
                continue;
            }
            let row = Row {
                name: r.get(1)?,
                visitors: r.get(2)?,
                pageviews: r.get(3)?,
                visits: r.get(4)?,
                bounces: r.get(5)?,
                duration: r.get(6)?,
                exits: r.get(7)?,
                unavailable: r.get(8)?,
                time_on_page_ms: r.get(9)?,
                time_on_page_visits: r.get(10)?,
                total_scroll_depth: r.get(11)?,
                scroll_depth_visits: r.get(12)?,
                ..Default::default()
            };
            if crate::location_reports::is_dimension(&dimension)
                && !crate::location_reports::excluded(&dimension).contains(&row.name.as_str())
            {
                let cohort = location_cohorts.entry(dimension.clone()).or_default();
                cohort.visitors += row.visitors;
                cohort.pageviews += row.pageviews;
            }
            if let Some(data) = by_dimension.get_mut(dimension.as_str()) {
                data.merge(c, &row)?;
            }
        }
    }
    if let Some(imported) = &filtered_imports {
        include_imports = imported.included.contains("total");
        for (dimension, rows) in &imported.rows {
            rows.for_each(c, |row| {
                if dimension == "total" {
                    crate::import_reports::merge(&mut totals, &row);
                    if let Some(calendar) = &mut calendar
                        && row.name <= calendar_through.to_string()
                    {
                        crate::calendar::merge(calendar, query, &row);
                    }
                    if let Some(day) = daily.iter_mut().find(|day| day.name == row.name) {
                        crate::import_reports::merge(day, &row);
                    } else {
                        daily.push(row);
                    }
                } else if let Some(data) = by_dimension.get_mut(dimension.as_str()) {
                    breakdown_pageviews |= row.unavailable & 1 != 0;
                    data.merge(c, &row)?;
                }
                Ok(())
            })?;
        }
        session_metrics_available = totals.unavailable & (2 | 4 | 8) == 0;
        for key in selected
            .iter()
            .filter(|key| crate::acquisition::is_dimension(key))
        {
            if let Some(cohort) = imported.cohorts.get(*key) {
                crate::import_reports::merge(
                    acquisition.cohorts.entry((*key).into()).or_default(),
                    cohort,
                );
            }
            if imported.excluded.contains(*key) {
                acquisition.incomplete.insert((*key).into());
            }
        }
        for (key, cohort) in &imported.cohorts {
            if crate::location_reports::is_dimension(key) {
                crate::import_reports::merge(
                    location_cohorts.entry(key.clone()).or_default(),
                    cohort,
                );
            }
        }
    }
    let imports_excluded_dimensions = if let Some(imported) = &filtered_imports {
        imported.excluded.clone()
    } else if imports_enabled && has_imports && !include_imports {
        selected.iter().map(|key| (*key).to_owned()).collect()
    } else {
        BTreeSet::new()
    };
    let imports_excluded = if filtered_imports.is_some() {
        imports_excluded_dimensions.contains("total")
    } else {
        imports_enabled && has_imports && !include_imports
    };
    let mut comparison_missing = BTreeSet::new();
    if query.comparison_names.is_some() {
        comparison_missing.extend(acquisition.incomplete.iter().cloned());
        comparison_missing.extend(imports_excluded_dimensions.iter().cloned());
        if include_imports && filtered_imports.is_none() {
            let breakdowns = selected
                .iter()
                .copied()
                .filter(|key| *key != "total")
                .collect();
            comparison_missing.extend(
                crate::import_reports::build(c, &base, query, &breakdowns, false)?.excluded,
            );
        }
    }
    daily.sort_by(|a, b| a.name.cmp(&b.name));
    totals.visitors = daily.iter().map(|r| r.visitors).sum::<i64>() - native_daily_visitors
        + native_denominator.0;
    totals.pageviews = daily.iter().map(|r| r.pageviews).sum();
    if let Some(exits) = by_dimension.get("exit") {
        // Indexed lookup keeps the denominator independent of the exit audience.
        exits.exits(c, by_dimension["page"])?;
    }
    let (timeline_start, mut timeline_end) = window;
    let timeline_step = if query.interval.as_deref() == Some("minute")
        || (query.period.as_deref() == Some("realtime")
            && query.interval.as_deref() != Some("hour"))
    {
        60
    } else {
        3600
    };
    if query.period.as_deref() == Some("realtime") && timeline_step == 60 {
        timeline_end = (timeline_end - 1).div_euclid(60) * 60;
    } else if matches!(query.period.as_deref(), Some("day" | "month" | "year")) {
        timeline_end = timeline_end.min(now + 1);
    }
    let fine = query.scope != ReportScope::Suggestions
        && !include_imports
        && (matches!(query.interval.as_deref(), Some("hour" | "minute"))
            || (query.interval.is_none() && (query.window.is_some() || range.from == range.to)));
    let mut timeline = Vec::new();
    if fine && timeline_end > timeline_start && timeline_end - timeline_start <= 8 * 86400 {
        // Without a page predicate, a visit can span a bucket without any
        // pageview in that bucket. Page filters still require a matching view.
        let (from, values) = if query.has_filter("page") {
            (&session_from, &session_values)
        } else {
            (&active_session_from, &active_session_values)
        };
        timeline = crate::timeline::build(
            c,
            crate::timeline::Window {
                start: timeline_start,
                end: timeline_end,
                step: timeline_step,
                zone,
            },
            &query,
            crate::timeline::Input {
                from: &event_from,
                values: &event_values,
            },
            crate::timeline::Input { from, values },
        )?;
    }
    let mut detail_total = 0;
    let mut take = |key| -> rusqlite::Result<Vec<Row>> {
        let values = by_dimension.remove(key).unwrap_or_default();
        if query.scope != ReportScope::Suggestions {
            values.omit(c, crate::location_reports::excluded(key))?;
        }
        let denominator = native_denominator;
        let total = if crate::acquisition::is_dimension(key)
            || crate::location_reports::is_dimension(key)
        {
            let cohorts = if crate::location_reports::is_dimension(key) {
                &location_cohorts
            } else {
                &acquisition.cohorts
            };
            cohorts.get(key).map_or(0, |cohort| {
                if breakdown_pageviews {
                    cohort.pageviews
                } else {
                    cohort.visitors
                }
            })
        } else if let Some(imported) = &filtered_imports {
            let cohort = imported.cohorts.get(key);
            if breakdown_pageviews {
                denominator.1 + cohort.map_or(0, |r| r.pageviews)
            } else {
                denominator.0 + cohort.map_or(0, |r| r.visitors)
            }
        } else if breakdown_pageviews {
            totals.pageviews - native_denominator.1 + denominator.1
        } else {
            totals.visitors - native_denominator.0 + denominator.0
        };
        if let Some(names) = &query.comparison_names {
            return values.named(
                c,
                names.get(key).map(Vec::as_slice).unwrap_or_default(),
                breakdown_pageviews,
                total,
            );
        }
        let detail = !query.export && query.detail.as_deref() == Some(key);
        let default_sort = if breakdown_pageviews {
            "pageviews"
        } else {
            "visitors"
        };
        let limit = if detail {
            PAGE_SIZE
        } else if key == "country" && !query.export {
            usize::MAX
        } else if query.export && !matches!(key, "page" | "entry" | "exit") {
            300
        } else {
            100
        };
        let (rows, count) = values.select(
            c,
            Selection {
                dimension: key,
                sort: if detail {
                    query.sort.as_deref().unwrap_or(default_sort)
                } else {
                    default_sort
                },
                ascending: detail && query.order.as_deref() == Some("asc"),
                search: if detail {
                    query.search.as_deref()
                } else {
                    None
                },
                offset: if detail { query.offset.unwrap_or(0) } else { 0 },
                limit,
                omit_empty: query.scope != ReportScope::Suggestions
                    && crate::acquisition::is_campaign(key),
                pageviews: breakdown_pageviews,
                denominator: total,
            },
        )?;
        if detail {
            detail_total = count;
        }
        Ok(rows)
    };
    let calendar = (query.scope != ReportScope::Suggestions).then(|| {
        calendar
            .map(|days| days.into_values().collect())
            .unwrap_or_else(|| {
                crate::calendar::from_daily(query, &daily, &calendar_through.to_string())
            })
    });
    let mut result=Report {
            generated_at: if query.period.as_deref()==Some("24h") {query.window.map_or(now, |(_,end)|end-1)} else {now},
            realtime:query.period.as_deref()==Some("realtime"),detail_total:0,visitors:totals.visitors,pageviews:totals.pageviews,daily,calendar,
            pages:take("page")?,sources:take("source")?,referrers:take("referrer")?,countries:take("country")?,regions:take("region")?,cities:take("city")?,
            browsers:take("browser")?,browser_versions:take("browser_version")?,operating_systems:take("os")?,os_versions:take("os_version")?,devices:take("device")?,
            entry_pages:take("entry")?,exit_pages:take("exit")?,
            acquisition:crate::acquisition::DIMENSIONS.iter().map(|(key,_)|Ok(((*key).to_owned(),take(key)?))).collect::<rusqlite::Result<_>>()?,
            acquisition_incomplete:acquisition.incomplete,totals,
            timeline,timeline_start,timeline_end,timeline_step,
            current_visitors:c.query_row("SELECT COUNT(DISTINCT visitor) FROM events WHERE site_id=?1 AND received_at>=?2 AND received_at<=?3",params![site_id,now-300,now],|r|r.get(0))?,
            geo_enabled:c.query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key='geo_source')",[],|r|r.get(0))?,
            has_imports:include_imports,imports_available,imports_disabled:!imports_enabled,breakdown_pageviews,imports_excluded,imports_excluded_dimensions,
            session_metrics_available,comparison_missing,
            has_data:c.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE site_id=?1) OR EXISTS(SELECT 1 FROM import_daily WHERE site_id=?1)",[site_id],|r|r.get(0))?,
        };
    result.detail_total = detail_total;
    Ok(result)
}
