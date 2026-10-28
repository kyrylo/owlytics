use owlytics::{auth, db::Database, storage};
use rusqlite::Connection;
use std::path::PathBuf;

const START: i64 = 1793059200;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-activity-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn old_database(&self) -> PathBuf {
        let path = self.0.join("schema-12.sqlite");
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
        ] {
            c.execute_batch(sql).unwrap();
        }
        // Schema 12 only normalizes location keys. These records have no location.
        c.execute_batch("PRAGMA user_version=12; INSERT INTO sites VALUES(1,'example.com','UTC',0);
            INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES
            (1,1793059200,'2026-10-27',zeroblob(16),randomblob(16),'/article','','','Chrome','Desktop'),
            (1,1793059260,'2026-10-27',zeroblob(16),randomblob(16),'/next','','','Chrome','Desktop');
            INSERT INTO page_engagement VALUES(1,30000,50);
            INSERT INTO daily_engagement VALUES(1,'2026-10-27','/article',30000,1,50);").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn schema_twelve_upgrade_preserves_reports_and_activity_survives_backup_restore() {
    let fixture = Fixture::new();
    let db = Database::open(fixture.old_database()).unwrap();
    db.call(|c| {
        assert_eq!(c.query_row("PRAGMA user_version", [], |r| r.get::<_,i64>(0))?, owlytics::db::SCHEMA_VERSION);
        assert_eq!(c.query_row("SELECT COUNT(*),SUM(pageviews),SUM(last_at-started_at),SUM(active_at=last_at) FROM sessions", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?)))?, (1,2,60,1));
        assert_eq!(c.query_row("SELECT visitors,pageviews FROM daily_reports WHERE dimension='total'", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?, (1,2));
        assert_eq!(c.query_row("SELECT visits,pageviews,bounces,duration FROM daily_session_reports WHERE dimension='total'", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?)))?, (1,2,0,60));
        assert_eq!(c.query_row("SELECT time_ms FROM page_engagement", [], |r| r.get::<_,i64>(0))?, 30000);
        let columns=c.prepare("SELECT name FROM pragma_index_info('sessions_visitor_time') ORDER BY seqno")?.query_map([], |r| r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
        assert_eq!(columns, ["site_id","visitor","active_at"]);
        let plans=c.prepare("EXPLAIN QUERY PLAN SELECT id FROM sessions WHERE site_id=1 AND visitor=zeroblob(16) AND active_at>=1793059200 ORDER BY active_at DESC,id DESC LIMIT 1")?.query_map([], |r| r.get::<_,String>(3))?.collect::<Result<Vec<_>,_>>()?;
        assert!(plans.iter().any(|plan| plan.contains("sessions_visitor_time") && plan.contains("active_at>")), "{plans:?}");
        c.execute("UPDATE sessions SET active_at=?1", [START+1200])?;
        Ok(())
    }).await.unwrap();
    let backup = fixture.0.join("backup.sqlite");
    let output = backup.clone();
    db.read(move |c| Ok(storage::backup(c, &output)))
        .await
        .unwrap()
        .unwrap();
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&backup, &restored).unwrap();
    let db = Database::open(restored).unwrap();
    db.call(|c| {
        assert_eq!(c.query_row("SELECT active_at FROM sessions", [], |r| r.get::<_,i64>(0))?, START+1200);
        // Forty minutes since the last pageview, twenty since engagement.
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,'2026-10-27',zeroblob(16),randomblob(16),'/done','','','Chrome','Desktop')", [START+2400])?;
        assert_eq!(c.query_row("SELECT COUNT(*),SUM(pageviews),SUM(last_at-started_at),SUM(active_at) FROM sessions", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?)))?, (1,3,2400,START+2400));
        assert_eq!(c.query_row("PRAGMA integrity_check", [], |r| r.get::<_,String>(0))?, "ok");
        Ok(())
    }).await.unwrap();
}

#[test]
fn failed_activity_backfill_rolls_back_schema_and_existing_facts() {
    let fixture = Fixture::new();
    let path = fixture.old_database();
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TRIGGER reject_activity BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT,'injected activity failure'); END;").unwrap();
    assert!(Database::open(&path).is_err());
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name='active_at'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT name FROM pragma_index_info('sessions_visitor_time') WHERE seqno=2",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "last_at"
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*),SUM(pageviews),SUM(last_at-started_at) FROM sessions",
            [],
            |r| Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?
            ))
        )
        .unwrap(),
        (1, 2, 60)
    );
    c.execute_batch("DROP TRIGGER reject_activity;").unwrap();
    drop(c);
    assert!(Database::open(path).is_ok());
}
