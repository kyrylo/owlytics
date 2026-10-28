use owlytics::{
    analytics::{self, Event, ReportQuery, ReportScope},
    auth,
    db::Database,
    filters::{Filter, Operator},
    storage,
};
use rusqlite::{Connection, params, types::Value};
use std::{collections::BTreeMap, path::PathBuf};

const START: i64 = 1793059200;
const ATTRIBUTES: [&str; 8] = [
    "country",
    "region",
    "city",
    "browser",
    "browser_version",
    "os",
    "os_version",
    "device",
];
const FIRST: [&str; 8] = [
    "US",
    "US: US-CA",
    "US: 5391959",
    "Chrome",
    "130",
    "macOS",
    "14",
    "Desktop",
];
const LATER: [&str; 8] = [
    "PH",
    "PH: PH-00",
    "PH: 1701668",
    "Firefox",
    "131",
    "Linux",
    "1",
    "Mobile",
];

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-visit-attributes-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn legacy(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE owner_sessions(token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER); CREATE TABLE login_limit(id INTEGER,window_start INTEGER,attempts INTEGER);").unwrap();
        for sql in [
            include_str!("../src/schema.sql"),
            include_str!("../src/imports.sql"),
            include_str!("../src/rollups.sql"),
            include_str!("../src/indexes.sql"),
            include_str!("../src/sessions.sql"),
            include_str!("../src/session_rollups.sql"),
            include_str!("../src/engagement.sql"),
            include_str!("../src/acquisition.sql"),
            include_str!("../src/import_groups.sql"),
            include_str!("../src/session_lookup.sql"),
            // All fixture location keys are already canonical (schema 12).
            include_str!("../src/session_activity.sql"),
        ] {
            c.execute_batch(sql).unwrap();
        }
        seed(&c).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn page(
    c: &Connection,
    id: i64,
    site: i64,
    visitor: u128,
    seconds: i64,
    labels: [&str; 8],
) -> rusqlite::Result<()> {
    c.execute("INSERT INTO events(id,site_id,received_at,day,visitor,request_id,path,source,referrer,country,region,city,browser,browser_version,os,os_version,device) VALUES(?1,?2,?3,date(?3,'unixepoch'),?4,?5,?6,'Google','google.com',?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(site_id,request_id) DO NOTHING",
        params![id,site,START+seconds,visitor.to_be_bytes().as_slice(),(id as u128).to_be_bytes().as_slice(),format!("/page/{id}"),labels[0],labels[1],labels[2],labels[3],labels[4],labels[5],labels[6],labels[7]])?;
    Ok(())
}

fn seed(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'another.example','UTC',0);",
    )?;
    // One visitor uses both labels in two visits. Rebuilding by adding the
    // old aggregate rows would over-count that visitor after normalization.
    for (id, seconds, visitor, labels) in [
        (1, 0, 0, FIRST),
        (2, 60, 0, LATER),
        (3, 120, 0, LATER),
        (4, 2400, 0, LATER),
        (5, 2460, 0, FIRST),
        (6, 30, 1, FIRST),
        (7, 40, 2, LATER),
    ] {
        page(c, id, 1, visitor, seconds, labels)?;
    }
    // Unknown first-touch labels must remain unknown, and other sites/dates
    // must keep their own visitor populations.
    page(c, 8, 2, 0, 0, [""; 8])?;
    page(c, 9, 2, 0, 60, FIRST)?;
    page(c, 10, 1, 3, 86400, FIRST)?;
    c.execute_batch("INSERT INTO page_engagement VALUES(1,30000,50);
        INSERT INTO daily_engagement VALUES(1,'2026-10-27','/page/1',30000,1,50);
        INSERT INTO session_acquisition VALUES(1,'Organic Search','google','organic','launch','','',NULL);
        INSERT INTO import_batches VALUES(1,1,'2026-10-26','2026-10-26',0);
        INSERT INTO import_daily VALUES(1,'2026-10-26',1,5,9);
        INSERT INTO import_breakdowns VALUES(1,'2026-10-26','country','PH',1,9);
        INSERT INTO import_metrics VALUES(1,'2026-10-26','country','PH',1,NULL,5,2,180,NULL);
        INSERT INTO import_groups(site_id,day,batch_id,kind,dimensions,pageviews,visitors,visits,bounces,duration) VALUES(1,'2026-10-26',1,'location','{\"country\":\"PH\"}',9,5,5,2,180);
        INSERT INTO import_files VALUES(1,'locations','[\"country\",\"pageviews\",\"visitors\"]');")?;
    Ok(())
}

fn rows(c: &Connection, query: &str) -> rusqlite::Result<Vec<Vec<Value>>> {
    let mut statement = c.prepare(query)?;
    let columns = statement.column_count();
    statement
        .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())?
        .collect()
}
fn snapshot(
    c: &Connection,
    omit_repaired: bool,
) -> rusqlite::Result<BTreeMap<String, Vec<Vec<Value>>>> {
    let tables=c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?.query_map([], |r| r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    tables.into_iter().map(|table| {
        let columns=c.prepare(&format!("PRAGMA table_info(\"{table}\")"))?.query_map([], |r| r.get::<_,String>(1))?.collect::<Result<Vec<_>,_>>()?;
        let columns=columns.into_iter().filter(|column| !(omit_repaired && table=="events" && ATTRIBUTES.contains(&column.as_str()))).collect::<Vec<_>>();
        let fields=columns.iter().map(|column| format!("\"{column}\"")).collect::<Vec<_>>().join(",");
        let condition=if omit_repaired && table=="daily_reports" {"WHERE dimension NOT IN('country','region','city','browser','browser_version','os','os_version','device')"} else {""};
        let order=(1..=columns.len()).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let result=rows(c,&format!("SELECT {fields} FROM \"{table}\" {condition} ORDER BY {order}"))?;
        Ok((table,result))
    }).collect()
}

fn verify(c: &Connection) -> rusqlite::Result<()> {
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        owlytics::db::SCHEMA_VERSION
    );
    for id in 1..=10 {
        let expected = match id {
            4 | 5 | 7 => LATER,
            8 | 9 => [""; 8],
            _ => FIRST,
        };
        let labels=c.query_row("SELECT country,region,city,browser,browser_version,os,os_version,device FROM events WHERE id=?1",[id],|r| (0..8).map(|i|r.get::<_,String>(i)).collect::<Result<Vec<_>,_>>())?;
        assert_eq!(labels, expected, "event {id}");
    }
    for dimension in ATTRIBUTES {
        let expected = rows(
            c,
            &format!(
                "SELECT site_id,day,{dimension},COUNT(DISTINCT visitor),COUNT(*) FROM events GROUP BY site_id,day,{dimension} ORDER BY 1,2,3"
            ),
        )?;
        let actual = rows(
            c,
            &format!(
                "SELECT site_id,day,value,visitors,pageviews FROM daily_reports WHERE dimension='{dimension}' ORDER BY 1,2,3"
            ),
        )?;
        assert_eq!(actual, expected, "{dimension}");
    }
    assert_eq!(
        rows(
            c,
            "SELECT value,visitors,pageviews FROM daily_reports WHERE site_id=1 AND day='2026-10-27' AND dimension='country' ORDER BY value"
        )?,
        vec![
            vec!["PH".to_owned().into(), 2.into(), 3.into()],
            vec!["US".to_owned().into(), 2.into(), 4.into()]
        ]
    );
    assert_eq!(
        c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?,
        "ok"
    );
    Ok(())
}

async fn verify_reports(db: &Database) {
    for (i, key) in ATTRIBUTES.into_iter().enumerate() {
        let mut previous = None;
        for scope in [ReportScope::All, ReportScope::Timeline] {
            let q = ReportQuery {
                from: Some("2026-10-27".into()),
                to: Some("2026-10-27".into()),
                interval: Some("day".into()),
                scope,
                filters: vec![Filter {
                    key: key.into(),
                    op: Operator::Is,
                    values: vec![FIRST[i].into()],
                }],
                ..Default::default()
            };
            let report =
                analytics::report(db, 1, q.range("2026-10-27".parse().unwrap()).unwrap(), q)
                    .await
                    .unwrap();
            assert_eq!(
                (
                    report.visitors,
                    report.pageviews,
                    report.totals.visits,
                    report.totals.mean_visits(),
                    report.totals.duration,
                    report.totals.bounces
                ),
                (2, 4, 2, 2, 120, 1),
                "{key}, {scope:?}"
            );
            if let Some((totals, daily)) = previous {
                assert_eq!(report.totals, totals);
                assert_eq!(report.daily, daily);
            }
            previous = Some((report.totals, report.daily));
        }
    }
}

#[tokio::test]
async fn fresh_and_upgraded_attribution_preserves_facts_imports_and_backup_restore() {
    let fixture = Fixture::new();
    let path = fixture.legacy("schema-13.sqlite");
    let old = Connection::open(&path).unwrap();
    let mut unchanged = snapshot(&old, true).unwrap();
    // This fixture has no visitor reused across dates. Schema 15 adds an empty
    // derived date table; every pre-existing fact remains in the comparison.
    unchanged.insert("report_cross_day_dates".into(), Vec::new());
    // Schema 17 introduces settings and favicon storage; both start empty.
    unchanged.insert("owner_credentials".into(), Vec::new());
    unchanged.insert("site_favicons".into(), Vec::new());
    let old_backup = fixture.0.join("old-backup.sqlite");
    storage::backup(&old, &old_backup).unwrap();
    drop(old);
    let restored_old = fixture.0.join("restored-old.sqlite");
    storage::restore(&old_backup, &restored_old).unwrap();
    for path in [path, restored_old] {
        let db = Database::open(&path).unwrap();
        let expected = unchanged.clone();
        db.read(move |c| {
            verify(c)?;
            assert_eq!(snapshot(c, true)?, expected);
            Ok(())
        })
        .await
        .unwrap();
        verify_reports(&db).await;
        let backup = path.with_extension("backup");
        let output = backup.clone();
        db.read(move |c| Ok(storage::backup(c, &output)))
            .await
            .unwrap()
            .unwrap();
        let restored = path.with_extension("restored");
        storage::restore(&backup, &restored).unwrap();
        for db in [
            Database::open(path).unwrap(),
            Database::open(restored).unwrap(),
        ] {
            db.call(|c| {
                verify(c)?;
                page(c, 11, 1, 0, 2520, FIRST)?;
                // The second visit retains PH/Firefox despite the new labels.
                assert_eq!(
                    c.query_row(
                        "SELECT country,browser,session_id FROM events WHERE id=11",
                        [],
                        |r| Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?
                        ))
                    )?,
                    ("PH".into(), "Firefox".into(), 4)
                );
                let before = snapshot(c, false)?;
                page(c, 11, 1, 0, 2580, LATER)?;
                assert_eq!(
                    snapshot(c, false)?,
                    before,
                    "A duplicate cannot change labels or aggregates"
                );
                Ok(())
            })
            .await
            .unwrap();
        }
    }
    let fresh = Database::open(":memory:").unwrap();
    fresh
        .call(|c| {
            seed(c)?;
            verify(c)
        })
        .await
        .unwrap();
    verify_reports(&fresh).await;
}

#[test]
fn failed_attribute_or_aggregate_repair_rolls_back_the_entire_upgrade() {
    let fixture = Fixture::new();
    for (name, trigger) in [
        ("events", "BEFORE UPDATE OF country ON events"),
        (
            "summaries",
            "BEFORE DELETE ON daily_reports WHEN OLD.dimension='country'",
        ),
    ] {
        let path = fixture.legacy(&format!("{name}.sqlite"));
        let c = Connection::open(&path).unwrap();
        let before = snapshot(&c, false).unwrap();
        c.execute_batch(&format!("CREATE TRIGGER reject_repair {trigger} BEGIN SELECT RAISE(ABORT,'injected attribute repair failure'); END;")).unwrap();
        assert!(Database::open(&path).is_err());
        assert_eq!(
            c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            13
        );
        assert_eq!(snapshot(&c, false).unwrap(), before);
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='events_visit_attributes'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        c.execute_batch("DROP TRIGGER reject_repair;").unwrap();
        drop(c);
        assert!(Database::open(path).is_ok());
    }
}

#[tokio::test]
async fn a_geolocation_refresh_affects_new_visits_without_relabeling_active_ones() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        Ok(storage::import_geo(
            c,
            "203.0.113.1,203.0.113.9,NA,US,California,San Francisco,0,0\n".as_bytes(),
        ))
    })
    .await
    .unwrap()
    .unwrap();
    for (id, seconds) in [(1, 0), (2, 60), (3, 2400)] {
        if id == 2 {
            db.call(|c| {
                Ok(storage::import_geo(
                    c,
                    "203.0.113.1,203.0.113.9,AS,PH,Metro Manila,Manila,0,0\n".as_bytes(),
                ))
            })
            .await
            .unwrap()
            .unwrap();
        }
        assert_eq!(
            analytics::collect(
                &db,
                Event {
                    domain: "example.com".into(),
                    path: format!("/{id}"),
                    referrer: "https://google.com".into(),
                    id: format!("{id:032x}"),
                    campaign: None
                },
                "203.0.113.1".parse().unwrap(),
                "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36".into(),
                move || START + seconds
            )
            .await
            .unwrap(),
            analytics::Collected::Stored
        );
    }
    db.read(|c| {
        let labels = c
            .prepare("SELECT country,region,city FROM events ORDER BY id")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            labels[0],
            ("US".into(), "US: US-CA".into(), "US: 5391959".into())
        );
        assert_eq!(labels[1], labels[0]);
        assert_eq!(labels[2].0, "PH");
        assert_eq!(
            c.query_row("SELECT COUNT(*),SUM(pageviews) FROM sessions", [], |r| Ok(
                (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)
            ))?,
            (2, 3)
        );
        Ok(())
    })
    .await
    .unwrap();
}
