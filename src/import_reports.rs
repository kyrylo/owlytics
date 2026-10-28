//! Filter aggregate history only where the export retained the relationship.
//! A separate plan per panel avoids joining independent marginal counts.
use crate::{
    analytics::{ReportQuery, Row},
    filters::Filter,
    report_rows::Rows,
};
use rusqlite::{Connection, types::Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct Imported {
    pub rows: BTreeMap<String, Rows>,
    pub cohorts: BTreeMap<String, Row>,
    pub included: BTreeSet<String>,
    pub excluded: BTreeSet<String>,
}

fn family(key: &str) -> &str {
    match key {
        "source" | "referrer" | "channel" | "utm_source" | "utm_medium" | "utm_campaign"
        | "utm_content" | "utm_term" => "acquisition",
        "country" | "region" | "city" => "location",
        "browser" | "browser_version" => "browser",
        "os" | "os_version" => "os",
        other => other,
    }
}

pub(crate) fn file_coverage(required: &BTreeSet<&str>, batch: &str) -> String {
    let first = *required.first().expect("a history query has a dimension");
    let report = match family(first) {
        "acquisition" => "sources",
        "location" => "locations",
        "browser" => "browsers",
        "os" => "operating_systems",
        "page" => "pages",
        "entry" => "entry_pages",
        "exit" => "exit_pages",
        "device" => "devices",
        _ => unreachable!("validated report dimension"),
    };
    let mut columns = BTreeSet::new();
    for key in required {
        match *key {
            "channel" => columns.extend(["source", "utm_source", "utm_medium", "utm_campaign"]),
            "entry" => {
                columns.insert("entry_page");
            }
            "exit" => {
                columns.insert("exit_page");
            }
            "os" => {
                columns.insert("operating_system");
            }
            "os_version" => {
                columns.insert("operating_system_version");
            }
            other => {
                columns.insert(other);
            }
        }
    }
    let checks = columns
        .iter()
        .map(|name| format!("EXISTS(SELECT 1 FROM json_each(f.columns_json) WHERE value='{name}')"))
        .collect::<Vec<_>>()
        .join(" AND ");
    format!(
        "EXISTS(SELECT 1 FROM import_files f WHERE f.batch_id={batch} AND f.report='{report}' AND {checks})"
    )
}

enum Storage<'a> {
    Marginal(&'a str),
    Acquisition,
    Group(&'a str),
}
impl Storage<'_> {
    fn alias(&self) -> &'static str {
        match self {
            Self::Marginal(_) => "b",
            _ => "a",
        }
    }
    fn column(&self, key: &str) -> String {
        match self {
            Self::Marginal(_) => "b.value".into(),
            Self::Acquisition => format!("a.{key}"),
            Self::Group(_) => format!("json_extract(a.dimensions,'$.{key}')"),
        }
    }
    fn from(&self) -> &'static str {
        match self {
            Self::Marginal(_) => {
                "import_breakdowns b LEFT JOIN import_metrics m USING(site_id,day,dimension,value) LEFT JOIN import_engagement g ON b.dimension='page' AND g.site_id=b.site_id AND g.day=b.day AND g.path=b.value LEFT JOIN import_metrics entry ON b.dimension='page' AND entry.site_id=b.site_id AND entry.day=b.day AND entry.dimension='entry' AND entry.value=b.value"
            }
            Self::Acquisition => "import_acquisition a",
            Self::Group(_) => "import_groups a",
        }
    }
    fn metrics(&self) -> String {
        let (m, views, extras) = match self {
            Self::Marginal("page") => (
                "m",
                "b.pageviews",
                "32*MAX(g.time_ms IS NULL OR g.time_visits IS NULL)+64*MAX(g.scroll_depth IS NULL OR g.scroll_visits IS NULL)",
            ),
            Self::Marginal("exit") => ("m", "b.pageviews", "16"),
            Self::Marginal(_) => ("m", "b.pageviews", "0"),
            _ => ("a", "a.pageviews", "0"),
        };
        let entry_count = |column| {
            format!(
                "CASE WHEN entry.value IS NULL AND EXISTS(SELECT 1 FROM import_files f WHERE f.batch_id=b.batch_id AND f.report='entry_pages' AND EXISTS(SELECT 1 FROM json_each(f.columns_json) WHERE value='entrances') AND EXISTS(SELECT 1 FROM json_each(f.columns_json) WHERE value='bounces')) THEN 0 ELSE entry.{column} END"
            )
        };
        let bounce = if matches!(self, Self::Marginal("page")) {
            entry_count("bounces")
        } else {
            format!("{m}.bounces")
        };
        // Missing bounce counts and missing entrance counts share the same bit.
        let bounce_missing = if matches!(self, Self::Marginal("page")) {
            format!(
                "MAX(({bounce}) IS NULL OR ({}) IS NULL)",
                entry_count("visits")
            )
        } else {
            format!("MAX({bounce} IS NULL)")
        };
        let page_metrics = if matches!(self, Self::Marginal("page")) {
            format!(
                "SUM(COALESCE(g.time_ms,0)),SUM(COALESCE(g.time_visits,0)),SUM(COALESCE(g.scroll_depth,0)),SUM(COALESCE(g.scroll_visits,0)),SUM(COALESCE(({}),0))",
                entry_count("visits")
            )
        } else {
            "0,0,0,0,NULL".into()
        };
        format!(
            "SUM(COALESCE({m}.visitors,0)),SUM({views}),SUM(COALESCE({m}.visits,0)),SUM(COALESCE({bounce},0)),SUM(COALESCE({m}.duration,0)),MAX({m}.visitors IS NULL)+2*MAX({m}.visits IS NULL)+4*{bounce_missing}+8*MAX({m}.duration IS NULL)+({extras}),{page_metrics}"
        )
    }
}

fn plan<'a>(filters: &'a [Filter], dimension: &'a str) -> Option<(Storage<'a>, BTreeSet<&'a str>)> {
    let mut required: BTreeSet<_> = filters.iter().map(|f| f.key.as_str()).collect();
    if dimension != "total" {
        required.insert(dimension);
    }
    let first = *required.first()?;
    if required.len() == 1 {
        return Some((Storage::Marginal(first), required));
    }
    if !required.iter().all(|key| family(key) == family(first)) {
        return None;
    }
    let storage = match family(first) {
        "acquisition" => Storage::Acquisition,
        kind @ ("location" | "browser" | "os") => Storage::Group(kind),
        _ => return None,
    };
    Some((storage, required))
}

pub(crate) fn build(
    c: &Connection,
    base: &[Value],
    query: &ReportQuery,
    selected: &BTreeSet<&str>,
    collect_rows: bool,
) -> rusqlite::Result<Imported> {
    let filters = query.active_filters();
    let mut result = Imported::default();
    for dimension in selected {
        let Some((storage, required)) = plan(&filters, dimension) else {
            result.excluded.insert((*dimension).into());
            continue;
        };
        let alias = storage.alias();
        let mut values = base.to_vec();
        let fixed = match &storage {
            Storage::Marginal(key) => {
                values.push((*key).to_owned().into());
                format!("b.dimension=?{}", values.len())
            }
            Storage::Group(kind) => {
                values.push((*kind).to_owned().into());
                format!("a.kind=?{}", values.len())
            }
            Storage::Acquisition => "1".into(),
        };
        let known = required
            .iter()
            .map(|key| format!("{} IS NOT NULL", storage.column(key)))
            .collect::<Vec<_>>()
            .join(" AND ");
        // Check before applying the audience predicate. A known export with no
        // matching traffic is zero, not an unsupported or missing dataset.
        let coverage = format!(
            "({} OR EXISTS(SELECT 1 FROM {} WHERE {alias}.site_id=d.site_id AND {alias}.batch_id=d.batch_id AND {fixed} AND {known}))",
            file_coverage(&required, "d.batch_id"),
            storage.from()
        );
        let (included, excluded): (bool,bool) = c.query_row(&format!("SELECT COALESCE(MAX({coverage}),0),COALESCE(MAX(NOT {coverage}),0) FROM import_daily d WHERE d.site_id=?1 AND d.day BETWEEN ?2 AND ?3 AND (d.pageviews>0 OR d.visitors>0)"),rusqlite::params_from_iter(&values),|r|Ok((r.get(0)?,r.get(1)?)))?;
        if included {
            result.included.insert((*dimension).into());
        }
        if excluded {
            result.excluded.insert((*dimension).into());
        }
        if !collect_rows {
            continue;
        }
        let mut predicate =
            format!("{alias}.site_id=?1 AND {alias}.day BETWEEN ?2 AND ?3 AND {fixed} AND {known}");
        for filter in &filters {
            predicate.push_str(&format!(
                " AND {}",
                filter.sql(&storage.column(&filter.key), &mut values)
            ));
        }
        let name = if *dimension == "total" {
            format!("{alias}.day")
        } else {
            storage.column(dimension)
        };
        let mut statement = c.prepare(&format!(
            "SELECT {name},{} FROM {} WHERE {predicate} GROUP BY {name}",
            storage.metrics(),
            storage.from()
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(&values), |r| {
            Ok(Row {
                name: r.get(0)?,
                visitors: r.get(1)?,
                pageviews: r.get(2)?,
                visits: r.get(3)?,
                bounces: r.get(4)?,
                duration: r.get(5)?,
                unavailable: r.get(6)?,
                time_on_page_ms: r.get(7)?,
                time_on_page_visits: r.get(8)?,
                total_scroll_depth: r.get(9)?,
                scroll_depth_visits: r.get(10)?,
                entrances: r.get(11)?,
                ..Default::default()
            })
        })?;
        let mut cohort = Row::default();
        let mut stored = Rows::default();
        for row in rows {
            let row = row?;
            if (!crate::acquisition::is_campaign(dimension) || !row.name.is_empty())
                && !crate::location_reports::excluded(dimension).contains(&row.name.as_str())
            {
                cohort.visitors += row.visitors;
                cohort.pageviews += row.pageviews;
            }
            stored.merge(c, &row)?;
        }
        result.cohorts.insert((*dimension).into(), cohort);
        result.rows.insert((*dimension).into(), stored);
    }
    Ok(result)
}

pub(crate) fn merge(target: &mut Row, row: &Row) {
    target.add(row);
}
