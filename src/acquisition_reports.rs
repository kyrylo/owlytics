//! Acquisition denominators use the tagged audience, not a sum of table rows.
use crate::analytics::Row;
use rusqlite::{Connection, types::Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(crate) struct Coverage {
    pub cohorts: BTreeMap<String, Row>,
    pub incomplete: BTreeSet<String>,
}

pub(crate) fn native(
    c: &Connection,
    base: &[Value],
    selected: &BTreeSet<&str>,
) -> rusqlite::Result<Coverage> {
    let keys: Vec<_> = selected
        .iter()
        .filter(|key| crate::acquisition::is_dimension(key))
        .copied()
        .collect();
    let mut result = Coverage::default();
    if keys.is_empty() {
        return Ok(result);
    }
    // Old databases need no session scan for each newly introduced dimension.
    if !c.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_acquisition)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        let missing = c.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 AND pageviews>0)",rusqlite::params_from_iter(base),|r|r.get::<_,bool>(0))?;
        if missing {
            result
                .incomplete
                .extend(keys.into_iter().map(str::to_owned));
        }
        return Ok(result);
    }
    for key in keys {
        let sql = format!(
            "SELECT COALESCE(SUM(visitors),0),COALESCE(SUM(pageviews),0),COALESCE(MAX(missing),0) FROM (SELECT s.day,COUNT(DISTINCT CASE WHEN a.{key}!='' THEN s.visitor END) AS visitors,SUM(CASE WHEN a.{key}!='' THEN s.pageviews ELSE 0 END) AS pageviews,MAX(a.session_id IS NULL) AS missing FROM sessions s LEFT JOIN session_acquisition a ON a.session_id=s.id WHERE s.site_id=?1 AND s.day BETWEEN ?2 AND ?3 AND s.pageviews>0 GROUP BY s.day)"
        );
        let (visitors, pageviews, missing) =
            c.query_row(&sql, rusqlite::params_from_iter(base), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, bool>(2)?))
            })?;
        result.cohorts.insert(
            key.to_owned(),
            Row {
                visitors,
                pageviews,
                ..Default::default()
            },
        );
        if missing {
            result.incomplete.insert(key.to_owned());
        }
    }
    Ok(result)
}

pub(crate) fn imports(
    c: &Connection,
    base: &[Value],
    selected: &BTreeSet<&str>,
    coverage: &mut Coverage,
) -> rusqlite::Result<()> {
    for key in selected
        .iter()
        .filter(|key| crate::acquisition::is_dimension(key))
    {
        // Exported visitor counters are additive only at their original
        // granularity; the export contains no identities for deduplication.
        let (visitors,pageviews): (i64,i64)=c.query_row(&format!("SELECT COALESCE(SUM(visitors),0),COALESCE(SUM(pageviews),0) FROM import_acquisition WHERE site_id=?1 AND day BETWEEN ?2 AND ?3 AND {key}!=''"),rusqlite::params_from_iter(base),|r|Ok((r.get(0)?,r.get(1)?)))?;
        let cohort = coverage.cohorts.entry((*key).to_owned()).or_default();
        cohort.visitors += visitors;
        cohort.pageviews += pageviews;
        let file = crate::import_reports::file_coverage(&BTreeSet::from([*key]), "d.batch_id");
        let missing=c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM import_daily d WHERE d.site_id=?1 AND d.day BETWEEN ?2 AND ?3 AND d.pageviews>0 AND NOT ({file}) AND NOT EXISTS(SELECT 1 FROM import_acquisition a WHERE a.batch_id=d.batch_id AND a.{key} IS NOT NULL))"),rusqlite::params_from_iter(base),|r|r.get::<_,bool>(0))?;
        if missing {
            coverage.incomplete.insert((*key).to_owned());
        }
    }
    Ok(())
}
