use owlytics::{auth, db::Database, storage};
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-storage-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

#[tokio::test]
async fn schema_nine_upgrade_preserves_history_and_restores_new_import_relationships() {
    let fixture = Fixture::new();
    let path = fixture.0.join("v9.sqlite");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch("CREATE TABLE owner_sessions(token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER);CREATE TABLE login_limit(id INTEGER,window_start INTEGER,attempts INTEGER);").unwrap();
    for sql in [
        include_str!("../src/schema.sql"),
        include_str!("../src/imports.sql"),
        include_str!("../src/rollups.sql"),
        include_str!("../src/indexes.sql"),
        include_str!("../src/sessions.sql"),
        include_str!("../src/session_rollups.sql"),
        include_str!("../src/engagement.sql"),
        include_str!("../src/acquisition.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0);INSERT INTO import_batches VALUES(1,1,'2026-10-25','2026-10-25',0);INSERT INTO import_daily VALUES(1,'2026-10-25',1,3,5);INSERT INTO import_breakdowns VALUES(1,'2026-10-25','browser','Chrome',1,5);INSERT INTO import_metrics VALUES(1,'2026-10-25','browser','Chrome',1,3,3,1,30,NULL);").unwrap();
    drop(old);
    let db = Database::open(&path).unwrap();
    db.call(|c|{
        assert_eq!(c.query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0))?,owlytics::db::SCHEMA_VERSION);
        assert_eq!(c.query_row("SELECT SUM(pageviews) FROM import_daily",[],|r|r.get::<_,i64>(0))?,5);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM import_groups",[],|r|r.get::<_,i64>(0))?,0,"Upgrade cannot invent lost relationships");
        c.execute("INSERT INTO import_groups(site_id,day,batch_id,kind,dimensions,pageviews,visitors,visits,bounces,duration) VALUES(1,'2026-10-25',1,'browser',?1,5,3,3,1,30)",[r#"{"browser":"Chrome","browser_version":"1"}"#])?;
        c.execute("INSERT INTO import_files VALUES(1,'browsers',?1)",[r#"["browser","browser_version","pageviews","visitors"]"#])?;
        Ok(())
    }).await.unwrap();
    let backup = fixture.0.join("backup.sqlite");
    let copy = backup.clone();
    db.read(move |c| Ok(storage::backup(c, &copy)))
        .await
        .unwrap()
        .unwrap();
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&backup, &restored).unwrap();
    let restored = Database::open(restored).unwrap();
    restored.read(|c|{
        assert_eq!(c.query_row("SELECT json_extract(dimensions,'$.browser_version'),pageviews,visitors FROM import_groups",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)))?,("1".into(),5,3));
        assert_eq!(c.query_row("SELECT json_array_length(columns_json) FROM import_files WHERE report='browsers'",[],|r|r.get::<_,i64>(0))?,4);
        assert_eq!(c.query_row("PRAGMA integrity_check",[],|r|r.get::<_,String>(0))?,"ok");
        Ok(())
    }).await.unwrap();
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn country_replacement_is_atomic_and_rejects_overlapping_or_invalid_ranges() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        assert_eq!(
            storage::import_geo(
                c,
                "203.0.113.1,203.0.113.9,PH\n2001:db8::1,2001:db8::9,GB\n".as_bytes()
            )
            .unwrap(),
            2
        );
        for invalid in [
            "203.0.113.1,203.0.113.9,PH\n203.0.113.8,203.0.113.10,US\n",
            "203.0.113.9,203.0.113.1,PH\n",
            "203.0.113.1,2001:db8::9,PH\n",
            "203.0.113.1,203.0.113.9,xx\n",
            "not-an-ip,203.0.113.9,PH\n",
            "",
        ] {
            assert!(
                storage::import_geo(c, invalid.as_bytes()).is_err(),
                "{invalid}"
            );
            let count: i64 = c.query_row("SELECT COUNT(*) FROM geo_ranges", [], |r| r.get(0))?;
            assert_eq!(count, 2);
        }
        assert_eq!(
            storage::import_geo(c, "198.51.100.1,198.51.100.9,US\n".as_bytes()).unwrap(),
            1
        );
        assert_eq!(
            owlytics::analytics::lookup_country(c, "203.0.113.3".parse().unwrap())?,
            ""
        );
        assert_eq!(
            owlytics::analytics::lookup_country(c, "198.51.100.3".parse().unwrap())?,
            "US"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn backup_includes_wal_data_excludes_secrets_and_never_overwrites() {
    let fixture = Fixture::new();
    let db = Database::open(fixture.0.join("source.sqlite")).unwrap();
    db.call(|c| c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0); INSERT INTO site_salts(site_id,day,secret,expires_at) VALUES(1,'2026-10-27',X'123456',9999999999); INSERT INTO owner_sessions VALUES('token','csrf','hash',9999999999); INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,0,'2026-10-27',zeroblob(16),zeroblob(16),'/','','PH','Safari','Mobile');")).await.unwrap();
    let destination = fixture.0.join("backup.sqlite");
    let copied = destination.clone();
    db.read(move |c| Ok(storage::backup(c, &copied)))
        .await
        .unwrap()
        .unwrap();
    let backup = rusqlite::Connection::open(&destination).unwrap();
    let events: i64 = backup
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(events, 1);
    let secrets: i64 = backup
        .query_row(
            "SELECT (SELECT COUNT(*) FROM owner_sessions)+(SELECT COUNT(*) FROM site_salts)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(secrets, 0);
    let source_salts: i64 = db
        .read(|c| c.query_row("SELECT COUNT(*) FROM site_salts", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(source_salts, 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let before = std::fs::read(&destination).unwrap();
    assert!(storage::backup(&backup, &destination).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), before);
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&destination, &restored).unwrap();
    assert!(storage::restore(&destination, &restored).is_err());
    let restored = rusqlite::Connection::open(restored).unwrap();
    let views: i64 = restored
        .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(views, 1);
    assert_eq!(
        restored
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn schema_upgrade_rebuilds_reports_from_existing_events() {
    let fixture = Fixture::new();
    let path = fixture.0.join("upgrade.sqlite");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch(include_str!("../src/schema.sql"))
        .unwrap();
    old.execute_batch(include_str!("../src/imports.sql"))
        .unwrap();
    old.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0); INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,0,'2026-10-27',zeroblob(16),zeroblob(16),'/','','PH','Safari','Mobile');").unwrap();
    drop(old);
    let reopened = Database::open(&path).unwrap();
    let query = owlytics::analytics::ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        ..Default::default()
    };
    let range = query.range("2026-10-27".parse().unwrap()).unwrap();
    let report = owlytics::analytics::report(&reopened, 1, range, query)
        .await
        .unwrap();
    assert_eq!((report.visitors, report.pageviews), (1, 1));
    assert_eq!(report.pages[0].visitors, 1);
    let schema: i64 = reopened
        .read(|c| c.query_row("PRAGMA user_version", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(schema, owlytics::db::SCHEMA_VERSION);
}

#[tokio::test]
async fn schema_four_backups_restore_and_upgrade_without_changing_counts() {
    let fixture = Fixture::new();
    let source = fixture.0.join("v4.sqlite");
    let old = rusqlite::Connection::open(&source).unwrap();
    old.execute_batch("CREATE TABLE owner_sessions (token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER);CREATE TABLE login_limit (id INTEGER,window_start INTEGER,attempts INTEGER);CREATE TABLE site_salts(site_id INTEGER,day TEXT,secret BLOB,expires_at INTEGER);").unwrap();
    old.execute_batch("DROP TABLE site_salts;").unwrap();
    old.execute_batch(include_str!("../src/schema.sql"))
        .unwrap();
    old.execute_batch(include_str!("../src/imports.sql"))
        .unwrap();
    old.execute_batch(include_str!("../src/rollups.sql"))
        .unwrap();
    old.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0); INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,unixepoch('2026-10-27'),'2026-10-27',zeroblob(16),zeroblob(16),'/','','','Safari','Desktop');").unwrap();
    drop(old);
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&source, &restored).unwrap();
    let upgraded = Database::open(restored).unwrap();
    upgraded.read(|c| {
        assert_eq!(c.query_row("PRAGMA user_version",[],|r|r.get::<_,i64>(0))?,owlytics::db::SCHEMA_VERSION);
        assert_eq!(c.query_row("SELECT SUM(pageviews) FROM daily_reports WHERE dimension='total'",[],|r|r.get::<_,i64>(0))?,1);
        let plan=c.prepare("EXPLAIN QUERY PLAN SELECT visitor FROM events WHERE site_id=1 AND received_at>=1 AND received_at<9999999999")?.query_map([],|r|r.get::<_,String>(3))?.collect::<Result<Vec<_>,_>>()?.join(" ");
        assert!(plan.contains("COVERING INDEX events_site_time"),"{plan}");
        Ok(())
    }).await.unwrap();
    assert_eq!(
        rusqlite::Connection::open(source)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[tokio::test]
async fn schema_six_upgrades_summaries_and_keeps_them_correct_after_restore() {
    let fixture = Fixture::new();
    let path = fixture.0.join("v6.sqlite");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch("CREATE TABLE owner_sessions(token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER);CREATE TABLE login_limit(id INTEGER,window_start INTEGER,attempts INTEGER);").unwrap();
    for sql in [
        include_str!("../src/schema.sql"),
        include_str!("../src/imports.sql"),
        include_str!("../src/rollups.sql"),
        include_str!("../src/indexes.sql"),
        include_str!("../src/sessions.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])
        .unwrap();
    for (id, stamp, path, visitor) in [
        (0_u128, 0, "/", 0_u128),
        (1, 60, "/docs", 0),
        (2, 3601, "/", 0),
        (3, 3631, "/docs", 0),
        (4, 3700, "/docs", 1),
    ] {
        old.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200+?1,'2026-10-27',?2,?3,?4,'Google','PH','Chrome','Desktop')",rusqlite::params![stamp,visitor.to_be_bytes().to_vec(),id.to_be_bytes().to_vec(),path]).unwrap();
    }
    drop(old);
    let db = Database::open(&path).unwrap();
    let query = owlytics::analytics::ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        ..Default::default()
    };
    let range = query.range("2026-10-27".parse().unwrap()).unwrap();
    let report = owlytics::analytics::report(&db, 1, range.clone(), query.clone())
        .await
        .unwrap();
    assert_eq!(
        (
            report.visitors,
            report.pageviews,
            report.totals.visits,
            report.totals.bounces,
            report.totals.duration
        ),
        (2, 5, 3, 1, 90)
    );
    assert_eq!(
        (report.exit_pages[0].visitors, report.exit_pages[0].visits),
        (2, 3)
    );
    let backup = fixture.0.join("backup.sqlite");
    let output = backup.clone();
    db.read(move |c| Ok(storage::backup(c, &output).unwrap()))
        .await
        .unwrap();
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&backup, &restored).unwrap();
    let db = Database::open(restored).unwrap();
    db.call(|c| c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793062910,'2026-10-27',?1,?2,'/last','','PH','Chrome','Desktop')",rusqlite::params![1_u128.to_be_bytes().to_vec(),5_u128.to_be_bytes().to_vec()])).await.unwrap();
    let report = owlytics::analytics::report(&db, 1, range, query)
        .await
        .unwrap();
    assert_eq!(
        (
            report.pageviews,
            report.totals.visits,
            report.totals.bounces,
            report.totals.duration
        ),
        (6, 3, 0, 100)
    );
    let exit = report
        .exit_pages
        .iter()
        .find(|r| r.name == "/docs")
        .unwrap();
    assert_eq!((exit.visitors, exit.visits), (1, 2));
    let exit = report
        .exit_pages
        .iter()
        .find(|r| r.name == "/last")
        .unwrap();
    assert_eq!((exit.visitors, exit.visits), (1, 1));
}

#[tokio::test]
async fn schema_seven_upgrade_preserves_counts_without_inventing_engagement() {
    let fixture = Fixture::new();
    let path = fixture.0.join("v7.sqlite");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch("CREATE TABLE owner_sessions(token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER);CREATE TABLE login_limit(id INTEGER,window_start INTEGER,attempts INTEGER);").unwrap();
    for sql in [
        include_str!("../src/schema.sql"),
        include_str!("../src/imports.sql"),
        include_str!("../src/rollups.sql"),
        include_str!("../src/indexes.sql"),
        include_str!("../src/sessions.sql"),
        include_str!("../src/session_rollups.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])
        .unwrap();
    old.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200,'2026-10-27',zeroblob(16),zeroblob(16),'/old','','','Chrome','Desktop')",[]).unwrap();
    drop(old);
    let db = Database::open(&path).unwrap();
    let q = owlytics::analytics::ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        ..Default::default()
    };
    let r = owlytics::analytics::report(&db, 1, q.range("2026-10-27".parse().unwrap()).unwrap(), q)
        .await
        .unwrap();
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (1, 1, 1));
    assert!(!r.pages[0].available(owlytics::analytics::Metric::TimeOnPage));
    assert!(!r.pages[0].available(owlytics::analytics::Metric::ScrollDepth));
    db.read(|c| {
        assert_eq!(
            c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            owlytics::db::SCHEMA_VERSION
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM page_engagement", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM daily_engagement", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn filtered_aggregate_spills_are_isolated_and_reader_guards_survive_errors() {
    let fixture = Fixture::new();
    let db = Database::open(fixture.0.join("reports.sqlite")).unwrap();
    db.call(|c| {
        c.execute_batch("INSERT INTO sites VALUES(1,'one.example','UTC',0),(2,'two.example','UTC',0);")?;
        for (site,id) in [(1,1_u128),(1,2),(2,1)] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?1,unixepoch('2026-10-27'),'2026-10-27',zeroblob(16),?2,'/','','','Chrome','Desktop')",rusqlite::params![site,id.to_be_bytes().to_vec()])?;
        }
        // Exceed the aggregate cache so both event and session reports use
        // connection-local spill tables on real read-only pooled connections.
        let tx=c.transaction()?;
        for id in 3_u128..=5002 {
            tx.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,unixepoch('2026-10-27'),'2026-10-27',?1,?1,?2,'','','Chrome','Desktop')",rusqlite::params![id.to_be_bytes().as_slice(),format!("/page/{id:05}")])?;
            tx.execute("INSERT INTO page_engagement VALUES(last_insert_rowid(),?1,?2)",rusqlite::params![(id%100) as i64*1000,(id%101) as i64])?;
        }
        tx.commit()?;
        Ok(())
    }).await.unwrap();
    let q = owlytics::analytics::ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        browser: Some("Chrome".into()),
        ..Default::default()
    };
    for browser in [Some("Chrome"), None] {
        for (site, expected) in [(999, None), (1, Some(5002)), (2, Some(1)), (1, Some(5002))] {
            let result = owlytics::analytics::report(
                &db,
                site,
                q.range("2026-10-27".parse().unwrap()).unwrap(),
                owlytics::analytics::ReportQuery {
                    browser: browser.map(str::to_owned),
                    ..q.clone()
                },
            )
            .await;
            if let Some(views) = expected {
                assert_eq!(result.unwrap().pageviews, views);
            } else {
                assert!(result.is_err());
            }
            db.read(|c| {
                assert!(c.is_readonly("main")?);
                assert!(c.pragma_query_value(None, "query_only", |r| r.get::<_, bool>(0))?);
                assert_eq!(
                    c.query_row(
                        "SELECT COUNT(*) FROM sqlite_temp_master WHERE type='table'",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    0
                );
                Ok(())
            })
            .await
            .unwrap();
        }
    }
    let detail = owlytics::analytics::ReportQuery {
        detail: Some("page".into()),
        search: Some("/page/05002".into()),
        ..q
    };
    let result = owlytics::analytics::report(
        &db,
        1,
        detail.range("2026-10-27".parse().unwrap()).unwrap(),
        detail,
    )
    .await
    .unwrap();
    assert_eq!(result.detail_total, 1);
    assert_eq!(result.pages[0].name, "/page/05002");
    assert_eq!(
        (
            result.pages[0].visitors,
            result.pages[0].pageviews,
            result.pages[0].visits
        ),
        (1, 1, 1)
    );
    assert_eq!(
        (
            result.pages[0].time_on_page_ms,
            result.pages[0].time_on_page_visits,
            result.pages[0].total_scroll_depth,
            result.pages[0].scroll_depth_visits
        ),
        (2000, 1, 53, 1)
    );
}

#[tokio::test]
async fn schema_eight_upgrade_keeps_unknown_attribution_and_backup_restores_new_campaigns() {
    let fixture = Fixture::new();
    let path = fixture.0.join("v8.sqlite");
    let old = rusqlite::Connection::open(&path).unwrap();
    old.execute_batch("CREATE TABLE owner_sessions(token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER);CREATE TABLE login_limit(id INTEGER,window_start INTEGER,attempts INTEGER);").unwrap();
    for sql in [
        include_str!("../src/schema.sql"),
        include_str!("../src/imports.sql"),
        include_str!("../src/rollups.sql"),
        include_str!("../src/indexes.sql"),
        include_str!("../src/sessions.sql"),
        include_str!("../src/session_rollups.sql"),
        include_str!("../src/engagement.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])
        .unwrap();
    old.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200,'2026-10-27',zeroblob(16),zeroblob(16),'/old','Google','','Chrome','Desktop')",[]).unwrap();
    old.execute_batch("INSERT INTO import_batches VALUES(1,1,'2026-10-25','2026-10-25',0);INSERT INTO import_daily VALUES(1,'2026-10-25',1,3,5);INSERT INTO import_breakdowns VALUES(1,'2026-10-25','source','Google',1,5);").unwrap();
    drop(old);
    let db = Database::open(&path).unwrap();
    db.read(|c| {
        assert_eq!(
            c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            owlytics::db::SCHEMA_VERSION
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM session_acquisition", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM import_acquisition", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        assert_eq!(
            c.query_row(
                "SELECT SUM(pageviews) FROM daily_reports WHERE dimension='total'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        assert_eq!(
            c.query_row("SELECT SUM(pageviews) FROM import_daily", [], |r| r
                .get::<_, i64>(0))?,
            5
        );
        Ok(())
    })
    .await
    .unwrap();
    use owlytics::{acquisition::Campaign, analytics};
    assert_eq!(
        analytics::collect(
            &db,
            analytics::Event {
                domain: "example.com".into(),
                path: "/new".into(),
                referrer: String::new(),
                id: format!("{:032x}", 1),
                campaign: Some(Campaign {
                    source: Some("FB-Ads".into()),
                    ..Default::default()
                })
            },
            "192.0.2.1".parse().unwrap(),
            "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36".into(),
            || 1793059230
        )
        .await
        .unwrap(),
        analytics::Collected::Stored
    );
    let backup = fixture.0.join("backup.sqlite");
    let destination = backup.clone();
    db.call(move |c| Ok(storage::backup(c, &destination)))
        .await
        .unwrap()
        .unwrap();
    drop(db);
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&backup, &restored).unwrap();
    let db = Database::open(&restored).unwrap();
    db.read(|c| {
        assert_eq!(c.query_row("SELECT utm_source FROM session_acquisition",[],|r|r.get::<_,String>(0))?,"FB-Ads");
        assert_eq!(c.query_row("SELECT pageviews FROM daily_reports WHERE dimension='channel' AND value='Paid Social'",[],|r|r.get::<_,i64>(0))?,1);
        assert_eq!(c.query_row("SELECT SUM(pageviews) FROM daily_reports WHERE dimension='total'",[],|r|r.get::<_,i64>(0))?,2);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM site_salts",[],|r|r.get::<_,i64>(0))?,0);
        assert_eq!(c.query_row("PRAGMA quick_check",[],|r|r.get::<_,String>(0))?,"ok");
        Ok(())
    }).await.unwrap();
}
