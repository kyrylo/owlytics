//! Location panels exclude unknown places from their own percentage audience.
use crate::analytics::Row;
use rusqlite::{Connection, types::Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn is_dimension(key: &str) -> bool {
    matches!(key, "country" | "region" | "city")
}

pub(crate) fn excluded(key: &str) -> &'static [&'static str] {
    match key {
        "country" => &["", "\0\0", "ZZ"],
        "region" => &[""],
        "city" => &["", "0"],
        _ => &[],
    }
}

pub(crate) fn native(
    c: &Connection,
    base: &[Value],
    selected: &BTreeSet<&str>,
    totals: (i64, i64),
) -> rusqlite::Result<BTreeMap<String, Row>> {
    let mut result = BTreeMap::new();
    for key in selected.iter().filter(|key| is_dimension(key)) {
        let mut values = base.to_vec();
        values.extend(
            excluded(key)
                .iter()
                .map(|value| Value::Text((*value).into())),
        );
        let placeholders = (4..=values.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        // Most sites have no unknown countries. Reuse the headline summary
        // in that case; don't scan events just to reproduce the same count.
        let unknown = c.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM daily_reports WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 AND dimension='{key}' AND value IN({placeholders}))"),
            rusqlite::params_from_iter(&values), |row| row.get::<_, bool>(0),
        )?;
        let (visitors, pageviews) = if unknown {
            let known = c.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM daily_reports WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 AND dimension='{key}' AND value NOT IN({placeholders}))"),
                rusqlite::params_from_iter(&values), |row| row.get::<_, bool>(0),
            )?;
            // A visitor may have both known and unknown visits, or several
            // known places. Neither subtraction nor summing rows deduplicates it.
            if known {
                c.query_row(
                    &format!("SELECT COALESCE(SUM(visitors),0),COALESCE(SUM(pageviews),0) FROM (SELECT day,COUNT(DISTINCT visitor) visitors,COUNT(*) pageviews FROM events WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 AND {key} NOT IN({placeholders}) GROUP BY day)"),
                    rusqlite::params_from_iter(&values), |row| Ok((row.get(0)?, row.get(1)?)),
                )?
            } else {
                (0, 0)
            }
        } else {
            totals
        };
        result.insert(
            (*key).into(),
            Row {
                visitors,
                pageviews,
                ..Default::default()
            },
        );
    }
    Ok(result)
}
