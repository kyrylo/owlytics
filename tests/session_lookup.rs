use owlytics::{auth, db::Database, storage};
use rusqlite::{Connection, StatementStatus, params, types::Value};
use std::path::PathBuf;

const START: i64 = 1_760_486_400; // 2025-10-15 UTC

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-session-lookup-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Count SQLite instructions, including the actual collection triggers. This
// catches day-wide index scans without a machine-dependent timing threshold.
fn page(
    c: &Connection,
    site: i64,
    visitor: u128,
    request: u128,
    time: i64,
    path: &str,
) -> rusqlite::Result<i32> {
    let visitor = visitor.to_be_bytes();
    let request = request.to_be_bytes();
    let mut insert = c.prepare_cached("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?1,?2,date(?2,'unixepoch'),?3,?4,?5,'Google','US','Chrome','Desktop')")?;
    insert.reset_status(StatementStatus::VmStep);
    insert.execute(params![
        site,
        time,
        visitor.as_slice(),
        request.as_slice(),
        path
    ])?;
    let mut work = insert.get_status(StatementStatus::VmStep);
    let mut acquisition = c.prepare_cached("INSERT OR IGNORE INTO session_acquisition SELECT session_id,'Organic Search','google','organic','launch','','',NULL FROM events WHERE site_id=?1 AND request_id=?2")?;
    acquisition.reset_status(StatementStatus::VmStep);
    acquisition.execute(params![site, request.as_slice()])?;
    work += acquisition.get_status(StatementStatus::VmStep);
    Ok(work)
}

fn visit_sequence(c: &Connection, visitor: u128, first_id: u128) -> rusqlite::Result<Vec<i32>> {
    [
        (0, "/entry"),
        (60, "/exit"),
        (120, "/entry"),
        (3600, "/entry"),
        (3660, "/exit"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (time, path))| page(c, 1, visitor, first_id + i as u128, START + time, path))
    .collect()
}

fn verify_summary_counts(c: &Connection) -> rusqlite::Result<()> {
    for (dimension, column) in [("entry", "entry_page"), ("exit", "exit_page")] {
        let expected = c.prepare(&format!("SELECT site_id,day,{column},COUNT(DISTINCT visitor),COUNT(*),SUM(pageviews),SUM(pageviews=1),SUM(last_at-started_at) FROM sessions WHERE pageviews>0 GROUP BY site_id,day,{column} ORDER BY 1,2,3"))?;
        let actual = c.prepare("SELECT site_id,day,value,visitors,visits,pageviews,bounces,duration FROM daily_session_reports WHERE dimension=?1 ORDER BY 1,2,3")?;
        assert_eq!(
            rows(expected, [])?,
            rows(actual, [dimension])?,
            "{dimension}"
        );
    }
    let expected = c.prepare("SELECT s.site_id,s.day,a.utm_campaign,COUNT(DISTINCT s.visitor),COUNT(*),SUM(s.pageviews),SUM(s.pageviews=1),SUM(s.last_at-s.started_at) FROM sessions s JOIN session_acquisition a ON a.session_id=s.id GROUP BY s.site_id,s.day,a.utm_campaign ORDER BY 1,2,3")?;
    let actual = c.prepare("SELECT site_id,day,value,visitors,visits,pageviews,bounces,duration FROM daily_session_reports WHERE dimension='utm_campaign' ORDER BY 1,2,3")?;
    assert_eq!(rows(expected, [])?, rows(actual, [])?, "campaign");
    assert_eq!(
        c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?,
        "ok"
    );
    Ok(())
}

fn rows(
    mut statement: rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> rusqlite::Result<Vec<Vec<Value>>> {
    let columns = statement.column_count();
    statement
        .query_map(params, |r| (0..columns).map(|i| r.get(i)).collect())?
        .collect()
}

type Snapshot = Vec<(String, Vec<Vec<Value>>)>;
fn snapshot(c: &Connection) -> rusqlite::Result<Snapshot> {
    let tables = c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?
        .query_map([], |r| r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    tables
        .into_iter()
        .map(|table| {
            let query = format!("SELECT * FROM \"{table}\"");
            let columns = c.prepare(&query)?.column_count();
            let order = (1..=columns)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",");
            Ok((
                table,
                rows(c.prepare(&format!("{query} ORDER BY {order}"))?, [])?,
            ))
        })
        .collect()
}

#[tokio::test]
async fn fresh_collection_work_does_not_grow_with_unrelated_daily_visitors() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        let before = visit_sequence(c, 1, 1)?;
        let tx = c.transaction()?;
        for id in 100..2100 {
            page(&tx, 1, id, id, START, "/entry")?;
        }
        tx.commit()?;
        let after = visit_sequence(c, 2, 10_000)?;
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='sqlite_stat1'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0,
            "No statistics may hide a poor fresh-database plan"
        );
        for (small, large) in before.iter().zip(&after) {
            assert!(
                *large <= small * 2,
                "Collection scanned unrelated visits: {before:?} -> {after:?}"
            );
        }
        verify_summary_counts(c)
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn schema_ten_upgrade_and_both_backup_versions_preserve_all_data() {
    let fixture = Fixture::new();
    let source = fixture.0.join("v10.sqlite");
    let old = Connection::open(&source).unwrap();
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
        include_str!("../src/import_groups.sql"),
    ] {
        old.execute_batch(sql).unwrap();
    }
    old.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0);INSERT INTO import_batches VALUES(1,1,'2025-10-14','2025-10-14',0);INSERT INTO import_daily VALUES(1,'2025-10-14',1,3,5);").unwrap();
    visit_sequence(&old, 1, 1).unwrap();
    // Reuse visitor bytes across sites and days to verify the full scope of
    // uniqueness checks, even though the collector normally rotates hashes.
    page(&old, 2, 1, 20, START, "/entry").unwrap();
    page(&old, 1, 1, 21, START + 86400, "/entry").unwrap();
    verify_summary_counts(&old).unwrap();
    let last_at = old
        .prepare("SELECT * FROM sessions")
        .unwrap()
        .column_index("last_at")
        .unwrap();
    let mut before = snapshot(&old).unwrap();
    // Schema 13 adds activity initialized from the last pageview. All existing
    // columns and every other table must still survive the full upgrade exactly.
    for (_, rows) in before.iter_mut().filter(|(table, _)| table == "sessions") {
        for row in rows {
            row.push(row[last_at].clone());
        }
    }
    // Schema 15 adds only derived dates for the identity reused on both days.
    // The same bytes at site 2 do not make that site's single date cross-day.
    before.push((
        "report_cross_day_dates".into(),
        ["2025-10-15", "2025-10-16"]
            .into_iter()
            .map(|day| vec![Value::Integer(1), Value::Text(day.into())])
            .collect(),
    ));
    // Schema 17 adds empty owner settings and favicon tables without changing
    // any of the existing traffic, imports, or credentials.
    before.extend([
        ("owner_credentials".into(), Vec::new()),
        ("site_favicons".into(), Vec::new()),
    ]);
    before.sort_by(|a, b| a.0.cmp(&b.0));
    let old_backup = fixture.0.join("backup-v10.sqlite");
    storage::backup(&old, &old_backup).unwrap();
    drop(old);

    let restored_old = fixture.0.join("restored-v10.sqlite");
    storage::restore(&old_backup, &restored_old).unwrap();
    for path in [source, restored_old] {
        let db = Database::open(&path).unwrap();
        let unchanged = db.read(|c| snapshot(c)).await.unwrap();
        assert_eq!(
            unchanged, before,
            "The upgrade must preserve all facts, derive activity and cross-day dates, and add empty settings tables"
        );
        db.call(|c| {
            assert_eq!(c.query_row("PRAGMA user_version", [], |r| r.get::<_,i64>(0))?, owlytics::db::SCHEMA_VERSION);
            let plan = c.prepare("EXPLAIN QUERY PLAN SELECT 1 FROM sessions s WHERE s.site_id=1 AND s.day='2025-10-15' AND s.visitor=zeroblob(16) AND s.entry_page='/entry' AND s.id!=1 AND s.pageviews>0")?
                .query_map([], |r| r.get::<_,String>(3))?.collect::<Result<Vec<_>,_>>()?.join(" ");
            assert!(plan.contains("site_id=? AND day=? AND visitor=?"), "{plan}");
            page(c, 1, 1, 22, START + 86460, "/exit")?;
            page(c, 2, 1, 23, START + 60, "/exit")?;
            page(c, 1, 2, 24, START + 86520, "/entry")?;
            verify_summary_counts(c)
        }).await.unwrap();
        let expected = db.read(|c| snapshot(c)).await.unwrap();
        let backup = path.with_extension("v11.backup");
        let copy = backup.clone();
        db.read(move |c| Ok(storage::backup(c, &copy)))
            .await
            .unwrap()
            .unwrap();
        let restored = path.with_extension("v11.restored");
        storage::restore(&backup, &restored).unwrap();
        let restored = Database::open(restored).unwrap();
        assert_eq!(restored.read(|c| snapshot(c)).await.unwrap(), expected);
        restored.read(|c| verify_summary_counts(c)).await.unwrap();
    }
    assert_eq!(
        Connection::open(old_backup)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        10
    );
}
