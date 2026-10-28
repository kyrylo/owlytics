mod common;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use owlytics::{
    analytics::{self, Metric, ReportQuery},
    db::Database,
    engagement::{self, Collected, Event},
    storage,
};
use tower::ServiceExt;
const START: i64 = 1793059200;
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36";
async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute(
            "INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0)",
            [],
        )
    })
    .await
    .unwrap();
    db
}
async fn view(db: &Database, id: u128, path: &str, ip: &str, now: i64) {
    let event = analytics::Event {
        domain: "example.com".into(),
        path: path.into(),
        referrer: String::new(),
        id: format!("{id:032x}"),
        campaign: None,
    };
    assert_eq!(
        analytics::collect(db, event, ip.parse().unwrap(), UA.into(), move || now)
            .await
            .unwrap(),
        analytics::Collected::Stored
    );
}
fn measurement(id: u128, time_ms: i64, scroll_depth: i64) -> Event {
    Event {
        domain: "example.com".into(),
        id: format!("{id:032x}"),
        time_ms,
        scroll_depth,
    }
}
async fn measure(db: &Database, id: u128, time_ms: i64, scroll: i64, now: i64) -> Collected {
    engagement::collect(db, measurement(id, time_ms, scroll), UA.into(), move || now)
        .await
        .unwrap()
}
fn query() -> ReportQuery {
    ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        ..Default::default()
    }
}
async fn report(db: &Database, q: ReportQuery) -> analytics::Report {
    let range = q.range("2026-10-27".parse().unwrap()).unwrap();
    analytics::report(db, 1, range, q).await.unwrap()
}

#[tokio::test]
async fn engagement_keeps_a_visit_active_without_changing_traffic_metrics() {
    let db = fixture().await;
    view(&db, 1, "/article", "203.0.113.1", START).await;
    for minutes in [20, 40, 60] {
        assert_eq!(
            measure(&db, 1, minutes * 60_000, 50, START + minutes * 60).await,
            Collected::Stored
        );
        let r = report(&db, query()).await;
        assert_eq!(
            (
                r.visitors,
                r.pageviews,
                r.totals.visits,
                r.totals.bounces,
                r.totals.duration
            ),
            (1, 1, 1, 1, 0)
        );
    }
    view(&db, 2, "/next", "203.0.113.1", START + 80 * 60).await;
    for filter in [None, Some("Chrome")] {
        let r = report(
            &db,
            ReportQuery {
                browser: filter.map(str::to_owned),
                ..query()
            },
        )
        .await;
        assert_eq!(
            (
                r.visitors,
                r.pageviews,
                r.totals.visits,
                r.totals.bounces,
                r.totals.duration
            ),
            (1, 2, 1, 0, 4800)
        );
        assert_eq!(r.entry_pages[0].name, "/article");
        assert_eq!(r.exit_pages[0].name, "/next");
        assert_eq!(r.daily[0].duration, 4800);
    }
    db.read(|c| {
        assert_eq!(
            c.query_row("SELECT COUNT(DISTINCT session_id) FROM events", [], |r| r
                .get::<_, i64>(
                0
            ))?,
            1
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn late_engagement_and_retries_cannot_resurrect_an_expired_visit() {
    let db = fixture().await;
    view(&db, 1, "/article", "203.0.113.1", START).await;
    assert_eq!(
        measure(&db, 1, 600_000, 50, START + 600).await,
        Collected::Stored
    );
    assert_eq!(
        measure(&db, 1, 600_000, 50, START + 1500).await,
        Collected::Duplicate
    );
    view(&db, 2, "/second-visit", "203.0.113.1", START + 2461).await;
    // A delayed cumulative measurement may still update its page's engagement,
    // but it must not make the old visit eligible for subsequent pageviews.
    assert_eq!(
        measure(&db, 1, 650_000, 60, START + 2462).await,
        Collected::Stored
    );
    view(&db, 3, "/done", "203.0.113.1", START + 2463).await;
    let r = report(&db, query()).await;
    assert_eq!(
        (
            r.pageviews,
            r.totals.visits,
            r.totals.bounces,
            r.totals.duration
        ),
        (3, 2, 1, 2)
    );
    db.read(|c| {
        let sessions = c
            .prepare("SELECT session_id FROM events ORDER BY id")?
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(sessions, vec![1, 2, 2]);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn retry_safe_engagement_uses_per_page_visits_and_preserves_traffic_metrics() {
    let db = fixture().await;
    assert_eq!(
        measure(&db, 1, 1000, 20, START + 1).await,
        Collected::PendingPageview
    );
    view(&db, 1, "/", "203.0.113.1", START).await;
    assert_eq!(
        measure(&db, 1, 30000, 40, START + 30).await,
        Collected::Stored
    );
    assert_eq!(
        measure(&db, 1, 30000, 40, START + 31).await,
        Collected::Duplicate
    );
    view(&db, 2, "/", "203.0.113.1", START + 60).await;
    assert_eq!(
        measure(&db, 2, 20000, 80, START + 80).await,
        Collected::Stored
    );
    assert_eq!(
        measure(&db, 2, 10000, 50, START + 81).await,
        Collected::Duplicate
    );
    view(&db, 3, "/", "203.0.113.2", START + 10).await;
    measure(&db, 3, 10000, 20, START + 20).await;
    view(&db, 4, "/other", "203.0.113.1", START + 120).await;
    let all = report(&db, query()).await;
    assert_eq!(
        (
            all.visitors,
            all.pageviews,
            all.totals.visits,
            all.totals.bounces,
            all.totals.duration
        ),
        (2, 4, 2, 1, 120)
    );
    let page = all.pages.iter().find(|p| p.name == "/").unwrap();
    assert_eq!(
        (
            page.time_on_page_ms,
            page.time_on_page_visits,
            page.total_scroll_depth,
            page.scroll_depth_visits
        ),
        (60000, 2, 100, 2)
    );
    assert_eq!(Metric::TimeOnPage.value(page), 30.);
    assert_eq!(Metric::ScrollDepth.value(page), 50.);
    let other = all.pages.iter().find(|p| p.name == "/other").unwrap();
    assert!(!other.available(Metric::TimeOnPage));
    assert!(!other.available(Metric::ScrollDepth));
    for scope in [
        analytics::ReportScope::All,
        analytics::ReportScope::Dashboard,
        analytics::ReportScope::Timeline,
    ] {
        let filtered = report(
            &db,
            ReportQuery {
                scope,
                page: Some("/".into()),
                browser: Some("Chrome".into()),
                ..query()
            },
        )
        .await;
        assert_eq!(Metric::TimeOnPage.value(&filtered.totals), 30.);
        assert_eq!(Metric::ScrollDepth.value(&filtered.totals), 50.);
        assert_eq!(Metric::TimeOnPage.value(&filtered.daily[0]), 30.);
        assert_eq!(filtered.timeline[0].value(Metric::ScrollDepth), Some(50.));
    }
    let short = report(
        &db,
        ReportQuery {
            page: Some("/".into()),
            window: Some((START, START + 50)),
            ..query()
        },
    )
    .await;
    assert_eq!(short.pageviews, 2);
    assert_eq!(Metric::TimeOnPage.value(&short.totals), 20.);
    assert_eq!(Metric::ScrollDepth.value(&short.totals), 30.);
    // Returning after the inactivity boundary is a new visit to this page.
    view(&db, 5, "/", "203.0.113.1", START + 3600).await;
    measure(&db, 5, 30000, 50, START + 3630).await;
    let later = report(&db, query()).await;
    let page = later.pages.iter().find(|p| p.name == "/").unwrap();
    assert_eq!((page.time_on_page_visits, page.time_on_page_ms), (3, 90000));
    assert_eq!(Metric::ScrollDepth.value(page), 50.);
}

#[tokio::test]
async fn engagement_validates_bounds_site_and_age_without_creating_traffic() {
    let db = fixture().await;
    view(&db, 1, "/", "203.0.113.1", START).await;
    for (time, scroll) in [(-1, 50), (86400001, 50), (1, -1), (1, 101)] {
        assert_eq!(
            measure(&db, 1, time, scroll, START + 1).await,
            Collected::Invalid
        );
    }
    assert_eq!(
        measure(&db, 1, 1000, 50, START - 1).await,
        Collected::Invalid
    );
    assert_eq!(
        measure(&db, 1, 1000, 50, START + 86401).await,
        Collected::Invalid
    );
    let mut wrong_site = measurement(1, 1000, 50);
    wrong_site.domain = "other.example".into();
    assert_eq!(
        engagement::collect(&db, wrong_site, UA.into(), || START + 1)
            .await
            .unwrap(),
        Collected::PendingPageview
    );
    assert_eq!(
        measure(&db, 1, 86400000, 100, START + 2).await,
        Collected::Stored
    );
    let r = report(&db, query()).await;
    assert_eq!(r.pageviews, 1);
    assert_eq!(r.pages[0].time_on_page_ms, 3000); // Wall-clock bound with one second of rounding tolerance.
    assert_eq!(
        (r.totals.visits, r.totals.bounces, r.totals.duration),
        (1, 1, 0)
    );
}

#[tokio::test]
async fn engagement_endpoint_requires_matching_origin_and_honors_privacy_headers() {
    let db = fixture().await;
    let now = owlytics::auth::now();
    view(&db, 1, "/", "203.0.113.1", now - 10).await;
    let app = owlytics::router(db.clone(), common::config());
    let body=serde_json::json!({"domain":"example.com","id":format!("{:032x}",1),"time_ms":5000,"scroll_depth":75}).to_string();
    for (origin, privacy, expected) in [
        ("https://other.example", None, StatusCode::FORBIDDEN),
        ("https://example.com", Some("dnt"), StatusCode::NO_CONTENT),
        (
            "https://example.com",
            Some("sec-gpc"),
            StatusCode::NO_CONTENT,
        ),
        ("https://example.com", None, StatusCode::NO_CONTENT),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/engagement")
            .header("origin", origin)
            .header("user-agent", UA)
            .extension(ConnectInfo(
                "203.0.113.1:1000".parse::<std::net::SocketAddr>().unwrap(),
            ));
        if let Some(header) = privacy {
            request = request.header(header, "1");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.clone())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()["cache-control"], "no-store");
        if privacy.is_some() {
            assert_eq!(
                db.read(
                    |c| c.query_row("SELECT COUNT(*) FROM page_engagement", [], |r| r
                        .get::<_, i64>(0))
                )
                .await
                .unwrap(),
                0
            );
        }
    }
    assert_eq!(
        db.read(
            |c| c.query_row("SELECT time_ms FROM page_engagement", [], |r| r
                .get::<_, i64>(0))
        )
        .await
        .unwrap(),
        5000
    );
}

#[tokio::test]
async fn engagement_survives_backup_restore_and_later_updates_without_double_counting() {
    let db = fixture().await;
    view(&db, 1, "/", "203.0.113.1", START).await;
    measure(&db, 1, 10000, 40, START + 10).await;
    let folder = std::env::temp_dir().join(format!(
        "owlytics-engagement-{}",
        owlytics::auth::random_token().unwrap()
    ));
    std::fs::create_dir(&folder).unwrap();
    let backup = folder.join("backup.sqlite");
    let output = backup.clone();
    db.read(move |c| {
        storage::backup(c, &output).unwrap();
        Ok(())
    })
    .await
    .unwrap();
    let restored = folder.join("restored.sqlite");
    storage::restore(&backup, &restored).unwrap();
    let restored = Database::open(restored).unwrap();
    assert_eq!(
        measure(&restored, 1, 10000, 40, START + 11).await,
        Collected::Duplicate
    );
    assert_eq!(
        measure(&restored, 1, 20000, 80, START + 20).await,
        Collected::Stored
    );
    let r = report(&restored, query()).await;
    assert_eq!(
        (
            r.pages[0].time_on_page_ms,
            r.pages[0].time_on_page_visits,
            r.pages[0].total_scroll_depth
        ),
        (20000, 1, 80)
    );
    drop(restored);
    std::fs::remove_dir_all(folder).unwrap();
}
