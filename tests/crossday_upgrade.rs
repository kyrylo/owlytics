use owlytics::{auth, db::Database, storage};
use rusqlite::Connection;
use std::path::PathBuf;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-crossday-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn old_database(&self) -> PathBuf {
        let path = self.0.join("schema-14.sqlite");
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
            include_str!("../src/session_activity.sql"),
            include_str!("../src/visit_attributes.sql"),
        ] {
            c.execute_batch(sql).unwrap();
        }
        // No legacy location names require the schema-12 normalization.
        c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0);
          INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES
          (1,1793059140,'2026-10-26',zeroblob(16),randomblob(16),'/before','Google','US','Chrome','Desktop'),
          (1,1793059260,'2026-10-27',zeroblob(16),randomblob(16),'/seen','Google','US','Chrome','Desktop'),
          (1,1793149200,'2026-10-28',zeroblob(16),randomblob(16),'/seen','Google','US','Chrome','Desktop'),
          (2,1793059260,'2026-10-27',zeroblob(16),randomblob(16),'/','Google','US','Chrome','Desktop');
          INSERT INTO import_batches VALUES(1,1,'2026-10-25','2026-10-25',0);
          INSERT INTO import_daily VALUES(1,'2026-10-25',1,8,10);
          INSERT INTO page_engagement VALUES(1,2000,30);").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn facts(c: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    [
        "sites",
        "events",
        "sessions",
        "daily_reports",
        "daily_session_reports",
        "page_engagement",
        "daily_engagement",
        "import_batches",
        "import_daily",
    ]
    .into_iter()
    .map(|table| {
        let mut s = c
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
            .unwrap();
        let n = s.column_count();
        let rows = s
            .query_map([], |r| (0..n).map(|i| r.get(i)).collect())
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        (table.into(), rows)
    })
    .collect()
}

#[tokio::test]
async fn schema_fourteen_upgrade_and_backup_restore_preserve_facts_and_crossday_dates() {
    let fixture = Fixture::new();
    let path = fixture.old_database();
    let before = facts(&Connection::open(&path).unwrap());
    let db = Database::open(&path).unwrap();
    db.call(move |c| {
        assert_eq!(facts(c), before);
        let dates = c
            .prepare("SELECT site_id,day FROM report_cross_day_dates ORDER BY site_id,day")?
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            dates,
            [
                (1, "2026-10-26".into()),
                (1, "2026-10-27".into()),
                (1, "2026-10-28".into())
            ]
        );
        assert_eq!(
            c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            owlytics::db::SCHEMA_VERSION
        );
        assert_eq!(
            c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?,
            "ok"
        );
        Ok(())
    })
    .await
    .unwrap();
    let backup = fixture.0.join("backup.sqlite");
    let output = backup.clone();
    db.read(move |c| Ok(storage::backup(c, &output)))
        .await
        .unwrap()
        .unwrap();
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&backup, &restored).unwrap();
    let restored = Database::open(restored).unwrap();
    restored
        .call(|c| {
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM report_cross_day_dates", [], |r| r
                    .get::<_, i64>(0))?,
                3
            );
            c.execute("DELETE FROM sites WHERE id=1", [])?;
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM report_cross_day_dates", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[test]
fn failed_crossday_upgrade_rolls_back_the_backfill_and_keeps_old_facts() {
    let fixture = Fixture::new();
    let path = fixture.old_database();
    let c = Connection::open(&path).unwrap();
    let before = facts(&c);
    // Fail after the new table has been created and populated.
    c.execute_batch(
        "CREATE TRIGGER events_cross_day_reports AFTER INSERT ON sites BEGIN SELECT 1; END;",
    )
    .unwrap();
    assert!(Database::open(&path).is_err());
    assert_eq!(facts(&c), before);
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        14
    );
    assert_eq!(c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='report_cross_day_dates'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    c.execute_batch("DROP TRIGGER events_cross_day_reports")
        .unwrap();
    assert!(Database::open(&path).is_ok());
}

#[tokio::test]
async fn live_markers_cover_event_free_dates_and_are_isolated_by_site() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0)",[])?;
        for site in [1,2] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?1,1793059140,'2026-10-26',zeroblob(16),randomblob(16),'/','Google','US','Chrome','Desktop')",[site])?;
        }
        assert_eq!(c.query_row("SELECT COUNT(*) FROM report_cross_day_dates",[],|r|r.get::<_,i64>(0))?,0);
        c.execute("UPDATE sessions SET active_at=1793145600 WHERE site_id=1",[])?;
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793145660,'2026-10-28',zeroblob(16),randomblob(16),'/','Google','US','Chrome','Desktop')",[])?;
        let dates=c.prepare("SELECT day FROM report_cross_day_dates WHERE site_id=1 ORDER BY day")?.query_map([],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
        assert_eq!(dates,["2026-10-26","2026-10-27","2026-10-28"]);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM report_cross_day_dates WHERE site_id=2",[],|r|r.get::<_,i64>(0))?,0);
        let trigger:String=c.query_row("SELECT sql FROM sqlite_master WHERE name='events_cross_day_reports'",[],|r|r.get(0))?;
        let predicate=trigger.split_once("AND EXISTS(").unwrap().1.split_once(")\nBEGIN").unwrap().0
            .replace("NEW.site_id","1").replace("NEW.visitor","zeroblob(16)").replace("NEW.day","'2026-10-28'");
        let plan=c.prepare(&format!("EXPLAIN QUERY PLAN {predicate}"))?.query_map([],|r|r.get::<_,String>(3))?.collect::<Result<Vec<_>,_>>()?;
        assert!(plan.iter().any(|s|s.contains("sessions_visitor_time")),"{plan:?}");
        Ok(())
    }).await.unwrap();
}
