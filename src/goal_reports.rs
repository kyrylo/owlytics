//! Goal reports use click timestamps and distinct anonymous visitors. In these
//! report rows visitors/pageviews/visits hold uniques/total/eligible visitors;
//! only conversion metrics and columns are exposed by the goal UI and export.
use crate::{
    analytics::{Range, Report, ReportQuery, ReportScope, Row},
    report_rows::{Rows, Selection, Workspace},
};
use rusqlite::{Connection, params};
use std::collections::BTreeMap;

struct Buckets<'a>(&'a Connection);
impl Drop for Buckets<'_> {
    fn drop(&mut self) {
        let _ = self.0.execute_batch("DROP TABLE temp.goal_buckets");
    }
}

fn series(
    c: &Connection,
    site: i64,
    window: (i64, i64),
    query: &ReportQuery,
    bins: &[(String, i64, i64)],
) -> rusqlite::Result<Vec<Row>> {
    c.execute("DELETE FROM temp.goal_buckets", [])?;
    let mut insert = c.prepare("INSERT INTO temp.goal_buckets VALUES(?1,?2,?3)")?;
    for (name, start, end) in bins {
        insert.execute(params![name, start, end])?;
    }
    let join = |from: String, alias: &str| {
        from.replacen(" WHERE ",&format!(" JOIN temp.goal_buckets b ON {alias}.received_at>=b.start AND {alias}.received_at<b.end WHERE "),1)
    };
    let (clicks, values) = crate::outbound::source(site, window, query, true, true);
    let (all_clicks, _) = crate::outbound::source(site, window, query, true, false);
    let (pages, _) = crate::outbound::source(site, window, query, false, false);
    let numerator = format!(
        "SELECT b.name,COUNT(DISTINCT e.visitor) AS uniques,COUNT(*) AS total {} GROUP BY b.name",
        join(clicks, "o")
    );
    let denominator = format!(
        "SELECT name,COUNT(DISTINCT visitor) AS eligible FROM (SELECT b.name,e.visitor {} UNION SELECT b.name,e.visitor {}) GROUP BY name",
        join(pages, "e"),
        join(all_clicks, "o")
    );
    c.prepare(&format!("SELECT b.name,COALESCE(n.uniques,0),COALESCE(n.total,0),COALESCE(d.eligible,0) FROM temp.goal_buckets b LEFT JOIN ({numerator}) n USING(name) LEFT JOIN ({denominator}) d USING(name) ORDER BY b.start"))?
        .query_map(rusqlite::params_from_iter(&values),|r|Ok(Row{name:r.get(0)?,visitors:r.get(1)?,pageviews:r.get(2)?,visits:r.get(3)?,..Default::default()}))?.collect()
}

pub(crate) fn build(
    c: &Connection,
    site: i64,
    range: &Range,
    query: &ReportQuery,
) -> rusqlite::Result<Report> {
    let _workspace = Workspace::new(c)?;
    c.execute_batch("CREATE TEMP TABLE goal_buckets(name TEXT PRIMARY KEY,start INTEGER,end INTEGER) WITHOUT ROWID")?;
    let _buckets = Buckets(c);
    let zone: String = c.query_row("SELECT timezone FROM sites WHERE id=?1", [site], |r| {
        r.get(0)
    })?;
    let zone = zone.parse::<chrono_tz::Tz>().expect("validated timezone");
    let calendar = (
        crate::timeline::day_start(zone, range.from),
        crate::timeline::day_start(zone, range.to.succ_opt().unwrap()),
    );
    let window = query
        .window
        .map_or(calendar, |(a, b)| (a.max(calendar.0), b.min(calendar.1)));
    let now = query
        .window
        .map_or_else(crate::auth::now, |(_, end)| end - 1);
    let outbound = crate::outbound::report(c, site, window, query)?;
    let totals = Row {
        visitors: outbound.visitors,
        pageviews: outbound.clicks,
        visits: outbound.eligible,
        ..Default::default()
    };
    let mut result=Report{goal:true,generated_at:now,realtime:query.period.as_deref()==Some("realtime"),visitors:totals.visitors,pageviews:totals.pageviews,totals,outbound,
        current_visitors:c.query_row("SELECT COUNT(DISTINCT visitor) FROM events WHERE site_id=?1 AND received_at>=?2 AND received_at<=?3",params![site,now-300,now],|r|r.get(0))?,
        geo_enabled:c.query_row("SELECT EXISTS(SELECT 1 FROM metadata WHERE key='geo_source')",[],|r|r.get(0))?,
        has_data:c.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE site_id=?1)",[site],|r|r.get(0))?,
        ..Default::default()};
    let (clicks, args) = crate::outbound::source(site, window, query, true, true);
    let mut dimensions = BTreeMap::new();
    for key in crate::reports::dimensions(query)
        .into_iter()
        .filter(|key| *key != "total")
    {
        let (_, column, _) = crate::filters::DIMENSIONS
            .iter()
            .find(|(name, _, _)| *name == key)
            .expect("dimension");
        let table = if crate::acquisition::is_dimension(key) {
            "a"
        } else if matches!(key, "entry" | "exit") {
            "s"
        } else {
            "e"
        };
        let column = format!("COALESCE({table}.{column},'')");
        let (audience, _) = crate::outbound::audience(site, window, query, &column);
        let numerator = format!(
            "SELECT {column} AS name,COUNT(DISTINCT e.visitor) AS uniques,COUNT(*) AS total {clicks} GROUP BY {column}"
        );
        let denominator = format!(
            "SELECT name,COUNT(DISTINCT visitor) AS eligible FROM ({audience}) GROUP BY name"
        );
        let rows = Rows::from_query(
            c,
            "name,visitors,pageviews,visits",
            &format!(
                "SELECT n.name,n.uniques,n.total,COALESCE(d.eligible,0) FROM ({numerator}) n LEFT JOIN ({denominator}) d USING(name)"
            ),
            &args,
        )?;
        if query.scope != ReportScope::Suggestions {
            rows.omit(c, crate::location_reports::excluded(key))?;
        }
        let detail = !query.export && query.detail.as_deref() == Some(key);
        let selected = if let Some(names) = &query.comparison_names {
            rows.named(
                c,
                names.get(key).map(Vec::as_slice).unwrap_or_default(),
                false,
                result.visitors,
            )?
        } else {
            let (selected, count) = rows.select(
                c,
                Selection {
                    dimension: key,
                    sort: if detail {
                        query.sort.as_deref().unwrap_or("visitors")
                    } else {
                        "visitors"
                    },
                    ascending: detail && query.order.as_deref() == Some("asc"),
                    search: query.search.as_deref().filter(|_| detail),
                    offset: if detail { query.offset.unwrap_or(0) } else { 0 },
                    limit: if detail {
                        crate::reports::PAGE_SIZE
                    } else if key == "country" && !query.export {
                        usize::MAX
                    } else if query.export && !matches!(key, "page" | "entry" | "exit") {
                        300
                    } else {
                        100
                    },
                    omit_empty: query.scope != ReportScope::Suggestions
                        && crate::acquisition::is_campaign(key),
                    pageviews: false,
                    denominator: result.visitors,
                },
            )?;
            if detail {
                result.detail_total = count;
            }
            selected
        };
        dimensions.insert(key.to_owned(), selected);
    }
    result.pages = dimensions.remove("page").unwrap_or_default();
    result.entry_pages = dimensions.remove("entry").unwrap_or_default();
    result.exit_pages = dimensions.remove("exit").unwrap_or_default();
    result.sources = dimensions.remove("source").unwrap_or_default();
    result.referrers = dimensions.remove("referrer").unwrap_or_default();
    result.countries = dimensions.remove("country").unwrap_or_default();
    result.regions = dimensions.remove("region").unwrap_or_default();
    result.cities = dimensions.remove("city").unwrap_or_default();
    result.browsers = dimensions.remove("browser").unwrap_or_default();
    result.browser_versions = dimensions.remove("browser_version").unwrap_or_default();
    result.operating_systems = dimensions.remove("os").unwrap_or_default();
    result.os_versions = dimensions.remove("os_version").unwrap_or_default();
    result.devices = dimensions.remove("device").unwrap_or_default();
    result.acquisition = dimensions;
    if query.scope == ReportScope::Suggestions {
        return Ok(result);
    }
    let mut day = range.from;
    let mut days = Vec::new();
    let mut calendar_bins: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    while day <= range.to {
        let start = crate::timeline::day_start(zone, day);
        let end = crate::timeline::day_start(zone, day.succ_opt().unwrap());
        days.push((day.to_string(), start, end));
        calendar_bins
            .entry(crate::calendar::key(query, &day.to_string()))
            .and_modify(|v| v.1 = end)
            .or_insert((start, end));
        day = day.succ_opt().unwrap();
    }
    result.daily = series(c, site, window, query, &days)?;
    result.calendar = Some(
        if matches!(query.interval.as_deref(), Some("week" | "month")) {
            series(
                c,
                site,
                window,
                query,
                &calendar_bins
                    .into_iter()
                    .map(|(key, (start, end))| (key, start, end))
                    .collect::<Vec<_>>(),
            )?
        } else {
            result.daily.clone()
        },
    );
    let step = if query.interval.as_deref() == Some("minute") || result.realtime {
        60
    } else {
        3600
    };
    result.timeline_start = window.0;
    result.timeline_end = if result.realtime {
        (window.1 - 1).div_euclid(60) * 60
    } else if matches!(query.period.as_deref(), Some("day" | "month" | "year")) {
        window.1.min(now + 1)
    } else {
        window.1
    };
    result.timeline_step = step;
    let fine = matches!(query.interval.as_deref(), Some("hour" | "minute"))
        || (query.interval.is_none() && (query.window.is_some() || range.from == range.to));
    if fine && result.timeline_end > window.0 && result.timeline_end - window.0 <= 8 * 86400 {
        let mut buckets = crate::timeline::boundaries(&crate::timeline::Window {
            start: window.0,
            end: result.timeline_end,
            step,
            zone,
        });
        let rows = series(
            c,
            site,
            window,
            query,
            &buckets
                .iter()
                .map(|b| (b.start.to_string(), b.start, b.end))
                .collect::<Vec<_>>(),
        )?;
        for (bucket, row) in buckets.iter_mut().zip(rows) {
            bucket.visitors = row.visitors;
            bucket.pageviews = row.pageviews;
            bucket.completed = row;
        }
        result.timeline = buckets;
    }
    Ok(result)
}
