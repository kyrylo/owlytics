use owlytics::{
    analytics::{self, Event, ReportQuery},
    auth,
    db::Database,
    migration, storage,
};
use rusqlite::{Connection, params};
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-location-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn legacy(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE owner_sessions(token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER);CREATE TABLE login_limit(id INTEGER,window_start INTEGER,attempts INTEGER);").unwrap();
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
        ] {
            c.execute_batch(sql).unwrap();
        }
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])
            .unwrap();
        for (i, region, city) in [
            (0, "US: California", "US: San Francisco, California"),
            (1, "US: US-CA", "US: 5391959"),
            (2, "US: California", "US: San Francisco, California"),
        ] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device,region,city) VALUES(1,1793059200+?1*60,'2026-10-27',zeroblob(16),?2,'/','','US','Chrome','Desktop',?3,?4)",params![i,(i as u128).to_be_bytes().as_slice(),region,city]).unwrap();
        }
        c.execute_batch("INSERT INTO import_batches VALUES(1,1,'2026-10-26','2026-10-26',0); INSERT INTO import_daily VALUES(1,'2026-10-26',1,3,6);").unwrap();
        for (dimension, old, new) in [
            ("region", "US: California", "US: US-CA"),
            ("city", "US: San Francisco, California", "US: 5391959"),
        ] {
            for (name, views, visitors) in [(old, 2, Some(1)), (new, 4, None)] {
                c.execute(
                    "INSERT INTO import_breakdowns VALUES(1,'2026-10-26',?1,?2,1,?3)",
                    params![dimension, name, views],
                )
                .unwrap();
                c.execute(
                    "INSERT INTO import_metrics VALUES(1,'2026-10-26',?1,?2,1,?3,2,1,30,NULL)",
                    params![dimension, name, visitors],
                )
                .unwrap();
            }
        }
        c.execute("INSERT INTO import_groups(site_id,day,batch_id,kind,dimensions,pageviews,visitors,visits,bounces,duration) VALUES(1,'2026-10-26',1,'location',?1,6,3,4,2,60)",[r#"{"country":"US","region":"US: California","city":"US: San Francisco, California"}"#]).unwrap();
        c.execute("INSERT INTO import_files VALUES(1,'locations',?1)",[r#"["country","region","city","pageviews","visitors","visits","bounces","visit_duration"]"#]).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn upgrade_merges_keys_without_double_counting_native_visitors_or_inventing_history() {
    let fixture = Fixture::new();
    let path = fixture.legacy("v11.sqlite");
    let old = Connection::open(&path).unwrap();
    let backup = fixture.0.join("backup.sqlite");
    storage::backup(&old, &backup).unwrap();
    drop(old);
    let restore = fixture.0.join("restored.sqlite");
    storage::restore(&backup, &restore).unwrap();
    for path in [path, restore] {
        let db = Database::open(&path).unwrap();
        db.read(|c| {
            assert_eq!(c.query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0))?,owlytics::db::SCHEMA_VERSION);
            assert_eq!(c.query_row("SELECT COUNT(*),COUNT(DISTINCT visitor),COUNT(DISTINCT session_id) FROM events",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?,(3,1,1));
            for (dimension,key) in [("region","US: US-CA"),("city","US: 5391959")] {
                assert_eq!(c.query_row("SELECT value,visitors,pageviews FROM daily_reports WHERE dimension=?1",[dimension],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?,(key.into(),1,3));
                assert_eq!(c.query_row("SELECT value,visits,pageviews,bounces,duration FROM daily_session_reports WHERE dimension=?1",[dimension],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?)))?,(key.into(),1,3,0,120));
                assert_eq!(c.query_row("SELECT value,pageviews FROM import_breakdowns WHERE dimension=?1",[dimension],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))?,(key.into(),6));
                assert_eq!(c.query_row("SELECT value,visitors,visits,bounces,duration,exits FROM import_metrics WHERE dimension=?1",[dimension],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,Option<i64>>(5)?)))?,(key.into(),None,4,2,60,None));
            }
            assert_eq!(c.query_row("SELECT json_extract(dimensions,'$.region'),json_extract(dimensions,'$.city') FROM import_groups",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?,("US: US-CA".into(),"US: 5391959".into()));
            assert_eq!(c.query_row("PRAGMA integrity_check",[],|r|r.get::<_,String>(0))?,"ok");Ok(())
        }).await.unwrap();
        let q = ReportQuery {
            from: Some("2026-10-26".into()),
            to: Some("2026-10-27".into()),
            region: Some("US: California".into()),
            city: Some("US: San Francisco, California".into()),
            ..Default::default()
        };
        let r = analytics::report(&db, 1, q.range("2026-10-28".parse().unwrap()).unwrap(), q)
            .await
            .unwrap();
        assert_eq!((r.visitors, r.pageviews, r.totals.visits), (4, 9, 5));
        assert_eq!(r.cities.len(), 1);
        assert_eq!(r.regions.len(), 1);
        // Reopening must not run a second normalization or change any counts.
        let reopened = Database::open(path).unwrap();
        reopened
            .read(|c| {
                assert_eq!(
                    c.query_row(
                        "SELECT SUM(pageviews) FROM import_breakdowns WHERE dimension='city'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    6
                );
                Ok(())
            })
            .await
            .unwrap();
    }
}

#[test]
fn failed_location_upgrade_rolls_back_facts_and_schema_version() {
    let fixture = Fixture::new();
    let path = fixture.legacy("failure.sqlite");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TRIGGER reject_location BEFORE UPDATE OF city ON events BEGIN SELECT RAISE(ABORT,'injected migration failure'); END;").unwrap();
    drop(c);
    assert!(Database::open(&path).is_err());
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(
        c.query_row(
            "SELECT region,city FROM events ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        )
        .unwrap(),
        (
            "US: California".into(),
            "US: San Francisco, California".into()
        )
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM import_breakdowns", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    c.execute_batch("DROP TRIGGER reject_location;").unwrap();
    drop(c);
    assert!(Database::open(path).is_ok());
}

#[tokio::test]
async fn dbip_collection_merges_with_plausible_locations_and_legacy_filter_links() {
    let fixture = Fixture::new();
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        storage::import_geo(
            c,
            "203.0.113.0,203.0.113.255,NA,US,California,San Francisco,37.7,-122.4\n".as_bytes(),
        )
        .unwrap();
        Ok(())
    })
    .await
    .unwrap();
    for i in 0..2 {
        analytics::collect(
            &db,
            Event {
                domain: "example.com".into(),
                path: "/".into(),
                referrer: String::new(),
                id: format!("{i:032x}"),
                campaign: None,
            },
            "203.0.113.5".parse().unwrap(),
            "Mozilla/5.0 Chrome/130.0 Safari/537.36".into(),
            move || 1793059200 + i as i64 * 30,
        )
        .await
        .unwrap();
    }
    std::fs::write(
        fixture.0.join("imported_visitors_20261026_20261026.csv"),
        "date,visitors,pageviews,visits,bounces,visit_duration\n2026-10-26,2,3,2,1,20\n",
    )
    .unwrap();
    std::fs::write(fixture.0.join("imported_locations_20261026_20261026.csv"),"date,country,region,city,visitors,pageviews,visits,bounces,visit_duration\n2026-10-26,US,US-CA,5391959,2,3,2,1,20\n").unwrap();
    let path = fixture.0.clone();
    db.call(move |c| {
        Ok(migration::import(
            c,
            "example.com",
            &path,
            "2026-10-28".parse().unwrap(),
        ))
    })
    .await
    .unwrap()
    .unwrap();
    for (region, city) in [
        (None, None),
        (
            Some("US: California"),
            Some("US: San Francisco, California"),
        ),
        (Some("US: US-CA"), Some("US: 5391959")),
    ] {
        let q = ReportQuery {
            from: Some("2026-10-26".into()),
            to: Some("2026-10-27".into()),
            region: region.map(str::to_owned),
            city: city.map(str::to_owned),
            ..Default::default()
        };
        let r = analytics::report(&db, 1, q.range("2026-10-28".parse().unwrap()).unwrap(), q)
            .await
            .unwrap();
        assert_eq!((r.visitors, r.pageviews, r.totals.visits), (3, 5, 3));
        assert_eq!(r.regions.len(), 1);
        assert_eq!(r.regions[0].name, "US: US-CA");
        assert_eq!(r.cities.len(), 1);
        assert_eq!(r.cities[0].name, "US: 5391959");
        assert_eq!(
            (
                r.cities[0].visitors,
                r.cities[0].pageviews,
                r.cities[0].bounces,
                r.cities[0].duration
            ),
            (3, 5, 1, 50)
        );
    }
    for (key, names) in [
        ("region", ["US: California", "US: US-CA"]),
        ("city", ["US: San Francisco, California", "US: 5391959"]),
    ] {
        for op in [
            owlytics::filters::Operator::Is,
            owlytics::filters::Operator::IsNot,
        ] {
            let q = ReportQuery {
                from: Some("2026-10-26".into()),
                to: Some("2026-10-27".into()),
                filters: vec![owlytics::filters::Filter {
                    key: key.into(),
                    op,
                    values: names.map(str::to_owned).to_vec(),
                }],
                ..Default::default()
            };
            let r = analytics::report(&db, 1, q.range("2026-10-28".parse().unwrap()).unwrap(), q)
                .await
                .unwrap();
            assert_eq!(
                (r.visitors, r.pageviews),
                if op == owlytics::filters::Operator::Is {
                    (3, 5)
                } else {
                    (0, 0)
                }
            );
        }
    }
}
