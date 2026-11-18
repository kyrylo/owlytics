mod common;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use owlytics::{
    analytics::{self, ReportQuery},
    db::Database,
    outbound::{self, Collected, Event},
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
        referrer: "https://google.com/".into(),
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
fn click(id: u128, page: u128, url: &str) -> Event {
    Event {
        domain: "example.com".into(),
        id: format!("{id:032x}"),
        page_id: format!("{page:032x}"),
        url: url.into(),
    }
}
async fn record(db: &Database, id: u128, page: u128, url: &str, now: i64) -> Collected {
    outbound::collect(db, click(id, page, url), UA.into(), move || now)
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
async fn clicks_deduplicate_and_inherit_visitors_and_filters_without_changing_traffic() {
    let db = fixture().await;
    assert_eq!(
        record(&db, 10, 1, "https://outside.example/", START).await,
        Collected::PendingPageview
    );
    view(&db, 1, "/article", "203.0.113.1", START).await;
    view(&db, 2, "/article", "203.0.113.2", START).await;
    view(&db, 3, "/next", "203.0.113.1", START + 10).await;
    assert_eq!(
        record(
            &db,
            10,
            1,
            "https://user:secret@outside.example/docs?q=private#fragment",
            START + 1
        )
        .await,
        Collected::Stored
    );
    assert_eq!(
        record(&db, 10, 1, "https://outside.example/docs", START + 2).await,
        Collected::Duplicate
    );
    assert_eq!(
        record(&db, 11, 1, "https://outside.example/docs", START + 3).await,
        Collected::Stored
    );
    assert_eq!(
        record(&db, 12, 2, "https://outside.example/docs", START + 4).await,
        Collected::Stored
    );
    assert_eq!(
        record(&db, 13, 3, "https://outside.example/docs", START + 11).await,
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
        (2, 3, 2, 1, 10)
    );
    assert_eq!(r.outbound.total, 1);
    let row = &r.outbound.rows[0];
    assert_eq!(
        (&*row.url, row.visitors, row.clicks),
        ("https://outside.example/docs", 2, 4)
    );
    let r = report(
        &db,
        ReportQuery {
            page: Some("/next".into()),
            browser: Some("Chrome".into()),
            source: Some("Google".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((r.outbound.total, r.outbound.rows[0].clicks), (1, 1));
    let r = report(
        &db,
        ReportQuery {
            source: Some("Bing".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(r.outbound.total, 0);
    let r = report(
        &db,
        ReportQuery {
            window: Some((START + 3, START + 5)),
            ..query()
        },
    )
    .await;
    assert_eq!((r.outbound.total, r.outbound.rows[0].clicks), (1, 2));
    let r = report(
        &db,
        ReportQuery {
            filters: vec![owlytics::filters::Filter {
                key: "page".into(),
                op: owlytics::filters::Operator::IsNot,
                values: vec!["/article".into()],
            }],
            ..query()
        },
    )
    .await;
    assert_eq!((r.outbound.total, r.outbound.rows[0].clicks), (1, 1));
}

#[tokio::test]
async fn validates_destination_site_capability_age_and_bots() {
    let db = fixture().await;
    view(&db, 1, "/", "203.0.113.1", START).await;
    for url in [
        "https://example.com/",
        "http://www.example.com./",
        "javascript:alert(1)",
        "mailto:a@outside.example",
        "/relative",
        "https://[invalid",
        "https://outside.example/\nprivate",
    ] {
        assert_eq!(
            record(&db, 10, 1, url, START + 1).await,
            Collected::Invalid,
            "{url}"
        );
    }
    for now in [START - 1, START + 86401] {
        assert_eq!(
            record(&db, 10, 1, "https://outside.example/", now).await,
            Collected::Invalid
        );
    }
    let mut event = click(10, 1, "https://outside.example/");
    event.domain = "other.example".into();
    assert_eq!(
        outbound::collect(&db, event, UA.into(), || START + 1)
            .await
            .unwrap(),
        Collected::PendingPageview
    );
    let mut event = click(10, 1, "https://outside.example/");
    event.id = "bad".into();
    assert_eq!(
        outbound::collect(&db, event, UA.into(), || START + 1)
            .await
            .unwrap(),
        Collected::Invalid
    );
    assert_eq!(
        outbound::collect(
            &db,
            click(10, 1, "https://outside.example/"),
            "Googlebot".into(),
            || START + 1
        )
        .await
        .unwrap(),
        Collected::Duplicate
    );
    assert_eq!(report(&db, query()).await.outbound.total, 0);
}

#[tokio::test]
async fn click_dates_follow_click_time_in_the_site_timezone_not_parent_page_date() {
    let db = fixture().await;
    db.call(|c| c.execute("UPDATE sites SET timezone='Asia/Manila' WHERE id=1", []))
        .await
        .unwrap();
    let boundary = START + 16 * 3600;
    view(&db, 1, "/", "203.0.113.1", boundary - 5).await;
    record(&db, 10, 1, "https://outside.example/", boundary + 5).await;
    assert_eq!(report(&db, query()).await.outbound.total, 0);
    let r = report(
        &db,
        ReportQuery {
            from: Some("2026-10-28".into()),
            to: Some("2026-10-28".into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(r.pageviews, 0);
    assert_eq!((r.outbound.total, r.outbound.rows[0].visitors), (1, 1));
}

#[tokio::test]
async fn outbound_details_search_sort_paginate_and_exports_ignore_detail_options() {
    let db = fixture().await;
    view(&db, 1, "/article", "203.0.113.1", START).await;
    for id in 1..=61 {
        record(
            &db,
            id + 100,
            1,
            &format!("https://outside.example/{id:03}"),
            START + 1,
        )
        .await;
    }
    let q = ReportQuery {
        detail: Some("outbound".into()),
        sort: Some("name".into()),
        order: Some("asc".into()),
        ..query()
    };
    let r = report(&db, q.clone()).await;
    assert_eq!((r.outbound.rows.len(), r.outbound.total), (50, 61));
    assert_eq!(r.outbound.rows[0].url, "https://outside.example/001");
    let r = report(
        &db,
        ReportQuery {
            offset: Some(50),
            ..q.clone()
        },
    )
    .await;
    assert_eq!((r.outbound.rows.len(), r.outbound.total), (11, 61));
    assert_eq!(r.outbound.rows[0].url, "https://outside.example/051");
    let r = report(
        &db,
        ReportQuery {
            search: Some("/061".into()),
            ..q.clone()
        },
    )
    .await;
    assert_eq!((r.outbound.rows.len(), r.outbound.total), (1, 1));
    let r = report(
        &db,
        ReportQuery {
            search: Some("/ARTICLE".into()),
            ..q.clone()
        },
    )
    .await;
    assert_eq!(r.outbound.total, 0);
    let r = report(
        &db,
        ReportQuery {
            export: true,
            offset: Some(100),
            search: Some("missing".into()),
            ..q
        },
    )
    .await;
    assert_eq!((r.outbound.rows.len(), r.outbound.total), (61, 61));
}

#[tokio::test]
async fn endpoint_checks_origin_privacy_and_retry_contract() {
    let db = fixture().await;
    let now = owlytics::auth::now();
    view(&db, 1, "/", "203.0.113.1", now - 10).await;
    for (origin, privacy, ua, page, respect, status, stored) in [
        (
            "https://attacker.example",
            None,
            UA,
            1,
            true,
            StatusCode::FORBIDDEN,
            0,
        ),
        (
            "https://example.com",
            Some("dnt"),
            UA,
            1,
            true,
            StatusCode::NO_CONTENT,
            0,
        ),
        (
            "https://example.com",
            Some("sec-gpc"),
            UA,
            1,
            true,
            StatusCode::NO_CONTENT,
            0,
        ),
        (
            "https://example.com",
            None,
            "Googlebot",
            1,
            true,
            StatusCode::NO_CONTENT,
            0,
        ),
        (
            "https://example.com",
            None,
            UA,
            99,
            true,
            StatusCode::SERVICE_UNAVAILABLE,
            0,
        ),
        (
            "https://example.com",
            Some("sec-gpc"),
            UA,
            1,
            false,
            StatusCode::NO_CONTENT,
            1,
        ),
        (
            "https://www.example.com",
            None,
            UA,
            1,
            true,
            StatusCode::NO_CONTENT,
            1,
        ),
    ] {
        let mut config = common::config();
        config.respect_gpc = respect;
        let app = owlytics::router(db.clone(), config);
        let payload = serde_json::json!({"domain":"example.com","id":format!("{:032x}",10),"page_id":format!("{page:032x}"),"url":"https://outside.example/"});
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/outbound")
            .header("origin", origin)
            .header("user-agent", ua)
            .header("content-type", "text/plain")
            .extension(ConnectInfo(
                "203.0.113.1:1000".parse::<std::net::SocketAddr>().unwrap(),
            ));
        if let Some(privacy) = privacy {
            request = request.header(privacy, "1");
        }
        let response = app
            .oneshot(request.body(Body::from(payload.to_string())).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        if status != StatusCode::FORBIDDEN {
            assert_eq!(response.headers()["access-control-allow-origin"], origin);
        }
        if status == StatusCode::SERVICE_UNAVAILABLE {
            assert_eq!(response.headers()["retry-after"], "1");
        }
        assert_eq!(
            db.read(
                |c| c.query_row("SELECT COUNT(*) FROM outbound_clicks", [], |r| r
                    .get::<_, i64>(0))
            )
            .await
            .unwrap(),
            stored
        );
    }
}

#[tokio::test]
async fn schema_upgrade_and_backup_restore_preserve_clicks_and_deduplication() {
    let folder = std::env::temp_dir().join(format!(
        "owlytics-outbound-{}",
        owlytics::auth::random_token().unwrap()
    ));
    std::fs::create_dir(&folder).unwrap();
    let path = folder.join("old.sqlite");
    let db = Database::open(&path).unwrap();
    db.call(|c|c.execute_batch("DROP TABLE outbound_clicks; PRAGMA user_version=17; INSERT INTO sites VALUES(1,'example.com','UTC',0);")).await.unwrap();
    view(&db, 1, "/", "203.0.113.1", START).await;
    drop(db);
    let db = Database::open(&path).unwrap();
    assert_eq!(
        record(&db, 10, 1, "https://outside.example/", START + 1).await,
        Collected::Stored
    );
    let backup = folder.join("backup.sqlite");
    let output = backup.clone();
    db.read(move |c| {
        owlytics::storage::backup(c, &output).unwrap();
        Ok(())
    })
    .await
    .unwrap();
    let restored = folder.join("restored.sqlite");
    owlytics::storage::restore(&backup, &restored).unwrap();
    let restored = Database::open(&restored).unwrap();
    assert_eq!(
        record(&restored, 10, 1, "https://outside.example/", START + 2).await,
        Collected::Duplicate
    );
    assert_eq!(report(&restored, query()).await.outbound.rows[0].clicks, 1);
    assert_eq!(report(&restored, query()).await.pageviews, 1);
    drop(restored);
    drop(db);
    std::fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
async fn immediate_exit_waits_for_an_in_flight_pageview_without_client_retries() {
    let db = fixture().await;
    let app = owlytics::router(db.clone(), common::config());
    let payload = serde_json::json!({"domain":"example.com","id":format!("{:032x}",10),"page_id":format!("{:032x}",1),"url":"https://outside.example/"});
    let request = Request::builder()
        .method("POST")
        .uri("/api/outbound")
        .header("origin", "https://example.com")
        .header("user-agent", UA)
        .extension(ConnectInfo(
            "203.0.113.1:1000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::from(payload.to_string()))
        .unwrap();
    let arrives_later = async {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        view(
            &db,
            1,
            "/immediate-exit",
            "203.0.113.1",
            owlytics::auth::now(),
        )
        .await;
    };
    let (response, ()) = tokio::join!(app.oneshot(request), arrives_later);
    assert_eq!(response.unwrap().status(), StatusCode::NO_CONTENT);
    assert_eq!(
        db.read(
            |c| c.query_row("SELECT COUNT(*) FROM outbound_clicks", [], |r| r
                .get::<_, i64>(0))
        )
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn goal_and_url_filters_drive_conversions_chart_breakdowns_and_denominators() {
    use owlytics::analytics::Metric;
    let db = fixture().await;
    view(&db, 1, "/article", "203.0.113.1", START).await;
    view(&db, 2, "/pricing", "203.0.113.1", START + 10).await;
    view(&db, 3, "/article", "203.0.113.2", START).await;
    view(&db, 4, "/article", "203.0.113.3", START).await;
    record(&db, 10, 1, "https://outside.example/a", START + 1).await;
    record(&db, 11, 2, "https://outside.example/a", START + 11).await;
    record(&db, 12, 2, "https://outside.example/b", START + 12).await;
    record(&db, 13, 3, "https://outside.example/b", START + 13).await;
    let q = ReportQuery {
        goal: Some(outbound::GOAL.into()),
        ..query()
    };
    let r = report(&db, q.clone()).await;
    assert!(r.goal);
    assert_eq!(
        (r.totals.visitors, r.totals.pageviews, r.totals.visits),
        (2, 4, 3)
    );
    assert!((Metric::ConversionRate.value(&r.totals) - 200. / 3.).abs() < 0.001);
    assert_eq!(
        (r.outbound.rows[0].visitors, r.outbound.rows[0].clicks),
        (2, 2)
    );
    assert_eq!(
        (r.daily[0].visitors, r.daily[0].pageviews, r.daily[0].visits),
        (2, 4, 3)
    );
    assert_eq!(
        r.timeline
            .iter()
            .map(|b| b.value(Metric::TotalConversions).unwrap() as i64)
            .sum::<i64>(),
        4
    );
    let r = report(
        &db,
        ReportQuery {
            url: Some("https://outside.example/a".into()),
            ..q.clone()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (1, 2, 3));
    assert_eq!(r.pages.len(), 2);
    assert!(r.pages.iter().all(|p| p.visitors == 1));
    let r = report(
        &db,
        ReportQuery {
            page: Some("/article".into()),
            url: Some("https://outside.example/a".into()),
            ..q.clone()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (1, 1, 3));
    let r = report(
        &db,
        ReportQuery {
            detail: Some("page".into()),
            search: Some("pricing".into()),
            sort: Some("conversion_rate".into()),
            order: Some("asc".into()),
            ..q.clone()
        },
    )
    .await;
    assert_eq!(r.detail_total, 1);
    assert_eq!(r.pages[0].name, "/pricing");
    assert_eq!(Metric::ConversionRate.value(&r.pages[0]), 100.);
    let r = report(
        &db,
        ReportQuery {
            source: Some("Bing".into()),
            ..q.clone()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (0, 0, 0));
    assert_eq!(Metric::ConversionRate.value(&r.totals), 0.);
    let r = report(
        &db,
        ReportQuery {
            interval: Some("week".into()),
            ..q
        },
    )
    .await;
    assert_eq!(
        (
            r.calendar.as_ref().unwrap()[0].visitors,
            r.calendar.as_ref().unwrap()[0].pageviews
        ),
        (2, 4)
    );
}

#[tokio::test]
async fn goal_conversion_after_midnight_counts_in_new_day_with_eligible_visitor() {
    use owlytics::analytics::Metric;
    let db = fixture().await;
    view(&db, 1, "/article", "203.0.113.1", START - 1).await;
    record(&db, 10, 1, "https://outside.example/", START + 1).await;
    let r = report(
        &db,
        ReportQuery {
            goal: Some(outbound::GOAL.into()),
            ..query()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (1, 1, 1));
    assert_eq!(Metric::ConversionRate.value(&r.totals), 100.);
    assert_eq!(r.daily[0].name, "2026-10-27");
}
