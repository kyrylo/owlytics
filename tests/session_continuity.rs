use chrono::DateTime;
use owlytics::{
    analytics::{self, Event, ReportQuery},
    db::Database,
    engagement,
};
use rusqlite::params;

const MIDNIGHT: i64 = 1_793_059_200;
const UA: &str = "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36";

async fn fixture(zone: &str) -> Database {
    let db = Database::open(":memory:").unwrap();
    let zone = zone.to_owned();
    db.call(move |c| c.execute("INSERT INTO sites VALUES(1,'example.com',?1,0)", [zone]))
        .await
        .unwrap();
    db
}
async fn view(db: &Database, id: u128, stamp: i64) {
    let event = Event {
        domain: "example.com".into(),
        path: format!("/page/{id}"),
        referrer: if id == 1 {
            "https://google.com/".into()
        } else {
            "https://github.com/".into()
        },
        id: format!("{id:032x}"),
        campaign: None,
    };
    assert_eq!(
        analytics::collect(
            db,
            event,
            "203.0.113.1".parse().unwrap(),
            UA.into(),
            move || stamp
        )
        .await
        .unwrap(),
        analytics::Collected::Stored
    );
}
async fn identities(db: &Database) -> Vec<(Vec<u8>, i64, String)> {
    db.read(|c| {
        c.prepare("SELECT visitor,session_id,day FROM events ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn active_visit_bridges_utc_rotation_and_expired_visit_uses_the_new_identity() {
    let db = fixture("UTC").await;
    view(&db, 1, MIDNIGHT - 10).await;
    view(&db, 2, MIDNIGHT + 10).await;
    view(&db, 3, MIDNIGHT + 1870).await;
    let rows = identities(&db).await;
    assert_eq!((&rows[0].0, rows[0].1), (&rows[1].0, rows[1].1));
    assert_ne!(rows[1].0, rows[2].0);
    assert_ne!(rows[1].1, rows[2].1);
    db.read(|c| {
        assert_eq!(
            c.query_row(
                "SELECT pageviews,last_at-started_at,source FROM sessions WHERE id=1",
                [],
                |r| Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?
                ))
            )?,
            (2, 20, "Google".into())
        );
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM report_cross_day_dates", [], |r| r
                .get::<_, i64>(0))?,
            2
        );
        Ok(())
    })
    .await
    .unwrap();
    let query = ReportQuery {
        from: Some("2026-10-26".into()),
        to: Some("2026-10-27".into()),
        interval: Some("week".into()),
        ..Default::default()
    };
    let report = analytics::report(
        &db,
        1,
        query
            .range(
                DateTime::from_timestamp(MIDNIGHT + 86400, 0)
                    .unwrap()
                    .date_naive(),
            )
            .unwrap(),
        query,
    )
    .await
    .unwrap();
    assert_eq!(
        (report.visitors, report.pageviews, report.totals.visits),
        (2, 3, 2)
    );
    assert_eq!(report.calendar.unwrap()[0].visitors, 2);
}

#[tokio::test]
async fn site_midnight_keeps_identity_and_utc_midnight_rotates_inactive_visitors() {
    let db = fixture("Asia/Manila").await;
    view(&db, 1, MIDNIGHT - 8 * 3600 - 10).await;
    view(&db, 2, MIDNIGHT - 8 * 3600 + 10).await;
    let rows = identities(&db).await;
    assert_eq!(rows[0].0, rows[1].0);
    assert_eq!(rows[0].1, rows[1].1);
    assert_ne!(rows[0].2, rows[1].2);
    view(&db, 3, MIDNIGHT + 10).await;
    let rows = identities(&db).await;
    assert_ne!(rows[1].0, rows[2].0);
    assert_eq!(rows[1].2, rows[2].2);
}

#[tokio::test]
async fn whole_minute_timeout_applies_to_pageviews_and_engagement() {
    for gap in [1800, 1859, 1860] {
        let db = fixture("UTC").await;
        view(&db, 1, MIDNIGHT).await;
        view(&db, 2, MIDNIGHT + gap).await;
        let rows = identities(&db).await;
        assert_eq!(rows[0].1 == rows[1].1, gap < 1860, "pageview gap {gap}");
        let db = fixture("UTC").await;
        view(&db, 1, MIDNIGHT).await;
        let event = engagement::Event {
            domain: "example.com".into(),
            id: format!("{:032x}", 1),
            time_ms: 1000,
            scroll_depth: 50,
        };
        assert_eq!(
            engagement::collect(&db, event, UA.into(), move || MIDNIGHT + gap)
                .await
                .unwrap(),
            engagement::Collected::Stored
        );
        view(&db, 2, MIDNIGHT + 2000).await;
        let rows = identities(&db).await;
        assert_eq!(rows[0].1 == rows[1].1, gap < 1860, "engagement gap {gap}");
    }
}

#[tokio::test]
async fn previous_secret_expires_and_cannot_bridge_a_second_rotation() {
    let db = fixture("UTC").await;
    view(&db, 1, MIDNIGHT - 10).await;
    view(&db, 2, MIDNIGHT + 10).await;
    let original = identities(&db).await[0].0.clone();
    let old_key: Vec<u8> = db
        .read(|c| c.query_row("SELECT previous_secret FROM site_salts", [], |r| r.get(0)))
        .await
        .unwrap();
    db.call(|c| c.execute("UPDATE sessions SET active_at=?1", [MIDNIGHT + 86400 - 10]))
        .await
        .unwrap();
    view(&db, 3, MIDNIGHT + 86400 - 1).await;
    assert_eq!(identities(&db).await[2].0, original);
    view(&db, 4, MIDNIGHT + 86400 + 1).await;
    assert_ne!(identities(&db).await[3].0, original);
    db.call(move|c| {
        assert_eq!(c.query_row("SELECT COUNT(*) FROM site_salts WHERE secret=?1 OR previous_secret=?1",[old_key],|r|r.get::<_,i64>(0))?,0);
        analytics::prune_salts(c,MIDNIGHT+172800)?;
        assert_eq!(c.query_row("SELECT COUNT(*) FROM site_salts WHERE previous_secret IS NOT NULL OR previous_expires_at IS NOT NULL",[],|r|r.get::<_,i64>(0))?,0);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM site_salts",[],|r|r.get::<_,i64>(0))?,1);
        analytics::prune_salts(c,MIDNIGHT+259200)?;
        assert_eq!(c.query_row("SELECT COUNT(*) FROM site_salts",[],|r|r.get::<_,i64>(0))?,0);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn current_identity_takes_priority_over_an_older_active_visit() {
    let db = fixture("UTC").await;
    view(&db, 1, MIDNIGHT - 4000).await;
    view(&db, 2, MIDNIGHT + 1).await;
    db.call(|c| {
        c.execute(
            "UPDATE sessions SET active_at=?1 WHERE id=1",
            params![MIDNIGHT + 2],
        )
    })
    .await
    .unwrap();
    view(&db, 3, MIDNIGHT + 3).await;
    let rows = identities(&db).await;
    assert_ne!(rows[0].1, rows[1].1);
    assert_eq!(rows[1].1, rows[2].1);
}
