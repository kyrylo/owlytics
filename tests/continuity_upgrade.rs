use hmac::{Hmac, Mac};
use owlytics::{
    analytics::{self, Event},
    auth,
    db::Database,
    storage,
};
use rusqlite::{Connection, params, types::Value};
use sha2::Sha256;
use std::path::PathBuf;

const MIDNIGHT: i64 = 1_793_059_200;
const UA: &str = "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36";
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-continuity-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn legacy(&self) -> PathBuf {
        let path = self.0.join("schema15.sqlite");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON;CREATE TABLE owner_sessions(token_hash TEXT,csrf TEXT,credential_hash TEXT,expires_at INTEGER);CREATE TABLE login_limit(id INTEGER,window_start INTEGER,attempts INTEGER);").unwrap();
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
            include_str!("../src/cross_day_reports.sql"),
        ] {
            c.execute_batch(sql).unwrap();
        }
        c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0);")
            .unwrap();
        let secret = [42u8; 32];
        let mut hmac = Hmac::<Sha256>::new_from_slice(&secret).unwrap();
        hmac.update(&1i64.to_be_bytes());
        hmac.update(b"203.0.113.1\0");
        hmac.update(UA.as_bytes());
        let visitor = hmac.finalize().into_bytes();
        c.execute(
            "INSERT INTO site_salts VALUES(1,'2026-10-26',?1,?2)",
            params![secret.as_slice(), MIDNIGHT],
        )
        .unwrap();
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,'2026-10-26',?2,randomblob(16),'/before','Google','US','Chrome','Desktop')",params![MIDNIGHT-10,&visitor[..16]]).unwrap();
        c.execute_batch("INSERT INTO import_batches VALUES(1,1,'2026-10-25','2026-10-25',0);INSERT INTO import_daily VALUES(1,'2026-10-25',1,5,10);INSERT INTO page_engagement VALUES(1,1000,30);").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn facts(c: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
    [
        "events",
        "sessions",
        "daily_reports",
        "daily_session_reports",
        "report_cross_day_dates",
        "import_batches",
        "import_daily",
        "page_engagement",
        "daily_engagement",
    ]
    .into_iter()
    .map(|name| {
        let mut s = c
            .prepare(&format!("SELECT * FROM {name} ORDER BY 1,2"))
            .unwrap();
        let n = s.column_count();
        (
            name.into(),
            s.query_map([], |r| (0..n).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap(),
        )
    })
    .collect()
}

#[tokio::test]
async fn upgrade_preserves_facts_bridges_legacy_secret_and_backup_removes_both_keys() {
    let fixture = Fixture::new();
    let path = fixture.legacy();
    let before = facts(&Connection::open(&path).unwrap());
    let db = Database::open(&path).unwrap();
    db.read(move |c| {
        assert_eq!(facts(c), before);
        assert_eq!(
            c.query_row(
                "SELECT day,expires_at,previous_secret FROM site_salts",
                [],
                |r| Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<Vec<u8>>>(2)?
                ))
            )?,
            ("".into(), MIDNIGHT, None)
        );
        Ok(())
    })
    .await
    .unwrap();
    let event = Event {
        domain: "example.com".into(),
        path: "/after".into(),
        referrer: "https://github.com/".into(),
        id: format!("{:032x}", 2),
        campaign: None,
    };
    assert_eq!(
        analytics::collect(
            &db,
            event,
            "203.0.113.1".parse().unwrap(),
            UA.into(),
            || MIDNIGHT - 5
        )
        .await
        .unwrap(),
        analytics::Collected::Stored
    );
    db.read(|c| {
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*),SUM(pageviews),SUM(last_at-started_at) FROM sessions",
                [],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?
                ))
            )?,
            (1, 2, 5)
        );
        assert_eq!(
            c.query_row("SELECT COUNT(DISTINCT visitor) FROM events", [], |r| r
                .get::<_, i64>(0))?,
            1
        );
        assert_eq!(
            c.query_row(
                "SELECT length(secret),length(previous_secret),previous_expires_at FROM site_salts",
                [],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?
                ))
            )?,
            (32, 32, MIDNIGHT)
        );
        Ok(())
    })
    .await
    .unwrap();
    let expected = db.read(|c| Ok(facts(c))).await.unwrap();
    let backup = fixture.0.join("backup.sqlite");
    let target = backup.clone();
    db.read(move |c| Ok(storage::backup(c, &target)))
        .await
        .unwrap()
        .unwrap();
    let restored = fixture.0.join("restored.sqlite");
    storage::restore(&backup, &restored).unwrap();
    let restored = Database::open(restored).unwrap();
    restored
        .read(move |c| {
            assert_eq!(facts(c), expected);
            assert_eq!(
                c.query_row("SELECT COUNT(*) FROM site_salts", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            assert_eq!(
                c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?,
                "ok"
            );
            Ok(())
        })
        .await
        .unwrap();
    db.read(|c| {
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM site_salts WHERE previous_secret IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            1
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[test]
fn failure_rolls_back_secret_columns_and_keeps_the_old_session_trigger() {
    let fixture = Fixture::new();
    let path = fixture.legacy();
    let c = Connection::open(&path).unwrap();
    let before = facts(&c);
    let trigger: String = c
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='events_daily_reports'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    c.execute_batch("CREATE TRIGGER fail_salt_upgrade BEFORE UPDATE ON site_salts BEGIN SELECT RAISE(ABORT,'injected failure');END;").unwrap();
    assert!(Database::open(&path).is_err());
    assert_eq!(facts(&c), before);
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        15
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('site_salts')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4
    );
    assert_eq!(
        c.query_row(
            "SELECT sql FROM sqlite_master WHERE name='events_daily_reports'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        trigger
    );
    c.execute_batch("DROP TRIGGER fail_salt_upgrade;").unwrap();
    assert!(Database::open(path).is_ok());
}
