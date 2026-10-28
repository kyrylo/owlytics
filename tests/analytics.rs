mod common;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use chrono::{TimeZone, Utc};
use owlytics::{
    analytics::{self, Collected, Event, ReportQuery},
    db::Database,
};
use std::net::{IpAddr, SocketAddr};
use tower::ServiceExt;

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36";
fn instant(day: u32, hour: u32) -> i64 {
    Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0)
        .unwrap()
        .timestamp()
}
fn event(id: u128, path: &str) -> Event {
    Event {
        domain: "example.com".into(),
        path: path.into(),
        referrer: "https://www.search.example/query?private=secret#fragment".into(),
        id: format!("{id:032x}"),
        campaign: None,
    }
}
async fn fixture(zone: &str) -> Database {
    let db = Database::open(":memory:").unwrap();
    let zone = zone.to_owned();
    db.call(move |c| c.execute("INSERT INTO sites VALUES(1,'example.com',?1,0)", [zone]))
        .await
        .unwrap();
    db
}
async fn add(db: &Database, id: u128, path: &str, ip: &str, now: i64) -> Collected {
    analytics::collect(
        db,
        event(id, path),
        ip.parse().unwrap(),
        UA.into(),
        move || now,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn counts_daily_visitors_deduplicates_retries_and_discards_private_fields() {
    let db = fixture("Asia/Manila").await;
    assert_eq!(
        add(
            &db,
            1,
            "/pricing?email=private@example.com#secret",
            "203.0.113.1",
            instant(26, 17)
        )
        .await,
        Collected::Stored
    );
    assert_eq!(
        add(&db, 1, "/pricing", "203.0.113.1", instant(26, 17)).await,
        Collected::Duplicate
    );
    add(&db, 2, "/", "203.0.113.1", instant(26, 18)).await;
    add(&db, 3, "/pricing", "203.0.113.2", instant(26, 18)).await;
    add(&db, 4, "/pricing", "203.0.113.1", instant(27, 17)).await;
    let query = ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-28".into()),
        ..Default::default()
    };
    let range = query.range(Utc::now().date_naive()).unwrap();
    let report = analytics::report(&db, 1, range.clone(), query.clone())
        .await
        .unwrap();
    assert_eq!((report.visitors, report.pageviews), (3, 4));
    assert_eq!(
        (report.daily[0].visitors, report.daily[0].pageviews),
        (2, 3)
    );
    assert_eq!(report.pages[0].name, "/pricing");
    assert_eq!(report.sources[0].name, "search.example");
    assert_eq!(report.browsers[0].name, "Chrome");
    assert_eq!(report.devices[0].name, "Desktop");
    let filtered = analytics::report(
        &db,
        1,
        range,
        ReportQuery {
            page: Some("/pricing".into()),
            ..query
        },
    )
    .await
    .unwrap();
    assert_eq!((filtered.visitors, filtered.pageviews), (3, 3));
    db.read(|c| {
        let (salts, day): (i64, String) = c.query_row("SELECT COUNT(*),day FROM site_salts", [], |r| Ok((r.get(0)?,r.get(1)?)))?;
        assert_eq!(salts, 1);
        assert_eq!(day, "2026-10-27");
        let distinct: i64 = c.query_row("SELECT COUNT(DISTINCT visitor) FROM events", [], |r| r.get(0))?;
        assert_eq!(distinct, 3);
        let private: i64 = c.query_row("SELECT COUNT(*) FROM events WHERE instr(path,'?')>0 OR instr(path,'secret')>0 OR instr(source,'?')>0", [], |r| r.get(0))?;
        assert_eq!(private, 0);
        let columns: Vec<String> = c.prepare("PRAGMA table_info(events)")?.query_map([], |r| r.get(1))?.collect::<Result<_,_>>()?;
        assert!(!columns.iter().any(|c| c.contains("ip") || c.contains("agent")));
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn geo_lookup_handles_both_families_gaps_and_boundaries() {
    let db = fixture("UTC").await;
    db.call(|c| {
        for (start, end, country) in [
            ("203.0.113.1", "203.0.113.9", "PH"),
            ("2001:db8::1", "2001:db8::9", "GB"),
        ] {
            let (family, start) = analytics::ip_key(start.parse().unwrap());
            let (_, end) = analytics::ip_key(end.parse().unwrap());
            c.execute(
                "INSERT INTO geo_ranges VALUES(?1,?2,?3,?4)",
                rusqlite::params![family, start, end, country],
            )?;
        }
        for (address, expected) in [
            ("203.0.113.0", ""),
            ("203.0.113.1", "PH"),
            ("203.0.113.9", "PH"),
            ("203.0.113.10", ""),
            ("2001:db8::9", "GB"),
            ("2001:db8::a", ""),
        ] {
            assert_eq!(
                analytics::lookup_country(c, address.parse().unwrap())?,
                expected
            );
        }
        Ok(())
    })
    .await
    .unwrap();
    add(&db, 1, "/", "203.0.113.3", instant(27, 0)).await;
    let country: String = db
        .read(|c| c.query_row("SELECT country FROM events", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(country, "PH");
}

#[test]
fn proxy_chain_cannot_be_spoofed_by_an_untrusted_client() {
    let peer: IpAddr = "198.51.100.1".parse().unwrap();
    let trusted = vec!["172.20.0.0/16".parse().unwrap()];
    assert_eq!(
        analytics::client_ip(peer, Some("1.2.3.4"), &trusted),
        Some(peer)
    );
    let proxy = "172.20.0.2".parse().unwrap();
    assert_eq!(
        analytics::client_ip(proxy, Some("1.2.3.4, 198.51.100.1, 172.20.0.3"), &trusted),
        Some(peer)
    );
    assert_eq!(analytics::client_ip(proxy, None, &trusted), None);
    assert_eq!(analytics::client_ip(proxy, Some("invalid"), &trusted), None);
    assert_eq!(
        analytics::client_ip(proxy, Some("172.20.0.3"), &trusted),
        None
    );
    assert_eq!(
        analytics::client_ip(
            "::ffff:198.51.100.1".parse().unwrap(),
            Some("1.2.3.4"),
            &trusted
        ),
        Some(peer)
    );
}

#[test]
fn validates_dimensions_without_retaining_urls_or_query_parameters() {
    assert_eq!(
        analytics::normalize_domain("ExAmPle.COM."),
        Some("example.com".into())
    );
    assert_eq!(
        analytics::normalize_domain("münich.example"),
        Some("xn--mnich-kva.example".into())
    );
    for invalid in [
        "user@example.com",
        "https://example.com",
        "a/b",
        "a:90",
        "a b.com",
        "-a.com",
    ] {
        assert!(analytics::normalize_domain(invalid).is_none(), "{invalid}");
    }
    for invalid in ["//evil.example", "relative", "/a\n", "/a\\b"] {
        assert!(!event(1, invalid).validate());
    }
    assert!(event(1, "/valid?a=b#c").validate());
    assert_eq!(
        analytics::source("https://www.example.com/private?q=a", "example.com"),
        ""
    );
    assert_eq!(analytics::source("javascript:alert(1)", "example.com"), "");
    assert!(analytics::user_agent("Googlebot/1.0").is_none());
    assert_eq!(
        analytics::user_agent("Mozilla iPhone Mobile Safari/123"),
        Some(("Safari", "Mobile"))
    );
    assert_eq!(
        analytics::user_agent("Mozilla Android SamsungBrowser/1"),
        Some(("Samsung Internet", "Tablet"))
    );
}

#[tokio::test]
async fn http_collector_validates_origins_privacy_preferences_and_request_limits() {
    let db = fixture("UTC").await;
    let app = owlytics::router(db.clone(), common::config());
    for (origin, body, dnt, expected) in [
        (
            "https://example.com",
            r#"{"domain":"example.com","path":"/","id":"00000000000000000000000000000001"}"#,
            "0",
            StatusCode::NO_CONTENT,
        ),
        (
            "https://www.example.com",
            r#"{"domain":"example.com","path":"/","id":"00000000000000000000000000000002"}"#,
            "1",
            StatusCode::NO_CONTENT,
        ),
        (
            "https://evil.example",
            r#"{"domain":"example.com","path":"/","id":"00000000000000000000000000000003"}"#,
            "0",
            StatusCode::FORBIDDEN,
        ),
        ("https://example.com", "{}", "0", StatusCode::BAD_REQUEST),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/event")
                    .header("origin", origin)
                    .header("user-agent", UA)
                    .header("dnt", dnt)
                    .header("content-type", "text/plain")
                    .extension(ConnectInfo(
                        "198.51.100.1:1234".parse::<SocketAddr>().unwrap(),
                    ))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert!(response.headers().get("set-cookie").is_none());
        if expected == StatusCode::NO_CONTENT {
            assert_eq!(response.headers()["access-control-allow-origin"], origin);
        }
    }
    let count: i64 = db
        .read(|c| c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(count, 1);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/event")
                .body(Body::from("x".repeat(9000)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn utc_salt_rotation_is_independent_of_dst_and_sites_are_isolated() {
    let db = fixture("America/New_York").await;
    let first = Utc
        .with_ymd_and_hms(2026, 11, 1, 4, 30, 0)
        .unwrap()
        .timestamp();
    let later = Utc
        .with_ymd_and_hms(2026, 11, 2, 4, 30, 0)
        .unwrap()
        .timestamp();
    add(&db, 1, "/", "203.0.113.1", first).await;
    add(&db, 2, "/", "203.0.113.1", later).await;
    let (visitors, expiry): (i64, i64) = db
        .read(|c| {
            Ok((
                c.query_row("SELECT COUNT(DISTINCT visitor) FROM events", [], |r| {
                    r.get(0)
                })?,
                c.query_row("SELECT expires_at FROM site_salts", [], |r| r.get(0))?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(visitors, 2); // UTC rotation occurs inside this 25-hour local date.
    assert_eq!(
        expiry,
        Utc.with_ymd_and_hms(2026, 11, 4, 0, 0, 0)
            .unwrap()
            .timestamp()
    );
    db.call(|c| c.execute("INSERT INTO sites VALUES(2,'other.example','UTC',0)", []))
        .await
        .unwrap();
    let mut other = event(3, "/");
    other.domain = "other.example".into();
    analytics::collect(
        &db,
        other,
        "203.0.113.1".parse().unwrap(),
        UA.into(),
        move || first,
    )
    .await
    .unwrap();
    let count: i64 = db
        .read(|c| {
            c.query_row("SELECT COUNT(DISTINCT visitor) FROM events", [], |r| {
                r.get(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn project_overviews_use_exact_24h_windows_and_deduplicate_visitors() {
    let db = fixture("America/New_York").await;
    // This rolling window crosses the autumn DST change, but is still 86,400 seconds.
    let now = Utc.with_ymd_and_hms(2026, 11, 1, 18, 30, 0).unwrap();
    let end = now.timestamp() + 1;
    let start = end - 86400;
    db.call(move |c| {
        c.execute("INSERT INTO sites VALUES(2,'quiet.example','Pacific/Kiritimati',0)", [])?;
        c.execute("INSERT INTO import_batches VALUES(1,1,'2026-10-31','2026-10-31',0)", [])?;
        c.execute("INSERT INTO import_daily VALUES(1,'2026-10-31',1,999,2000)", [])?;
        for (index, (at, visitor)) in [
            (start - 86401, 1u8), // Outside both windows.
            (start - 86400, 2),  // First second of the previous window.
            (start - 1, 3),
            (start, 4),          // First second of the current window.
            (start + 3599, 4),
            (start + 3600, 4),   // Same visitor, another hourly bucket.
            (end - 1, 5),
            (end, 6),            // Future event excluded.
        ].into_iter().enumerate() {
            let day = chrono::DateTime::from_timestamp(at, 0).unwrap()
                .with_timezone(&chrono_tz::America::New_York).date_naive().to_string();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,?3,?4,'/','','','Safari','Desktop')", rusqlite::params![at,day,vec![visitor;16],vec![index as u8;16]])?;
        }
        Ok(())
    }).await.unwrap();
    let overview = analytics::site_overviews(&db, now).await.unwrap();
    assert_eq!(overview.len(), 2);
    let current = &overview[0];
    assert_eq!((current.visitors, current.previous_visitors), (2, 2));
    assert_eq!(
        (current.hourly[0], current.hourly[1], current.hourly[23]),
        (1, 1, 1)
    );
    assert_eq!(current.hourly.iter().sum::<i64>(), 3);
    let report = analytics::report(
        &db,
        1,
        analytics::Range {
            from: "2026-10-31".parse().unwrap(),
            to: "2026-11-01".parse().unwrap(),
        },
        ReportQuery {
            period: Some("24h".into()),
            window: Some((start, end)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(current.visitors, report.visitors);
    assert_eq!(
        (overview[1].visitors, overview[1].previous_visitors),
        (0, 0)
    );
    assert_eq!(overview[1].hourly, [0; 24]);
}

#[tokio::test]
async fn filtered_reports_preserve_local_days_across_dst_and_combined_filters() {
    let db = fixture("America/New_York").await;
    for (id, day, hour, path) in [
        (1, 1, 3, "/docs"),
        (2, 1, 4, "/docs"),
        (3, 2, 4, "/docs"),
        (4, 2, 5, "/docs"),
        (5, 2, 4, "/other"),
    ] {
        let at = Utc
            .with_ymd_and_hms(2026, 11, day, hour, 30, 0)
            .unwrap()
            .timestamp();
        add(&db, id, path, &format!("203.0.113.{id}"), at).await;
    }
    let query = ReportQuery {
        from: Some("2026-11-01".into()),
        to: Some("2026-11-01".into()),
        page: Some("/docs".into()),
        browser: Some("Chrome".into()),
        ..Default::default()
    };
    let report = analytics::report(
        &db,
        1,
        query.range("2026-11-01".parse().unwrap()).unwrap(),
        query,
    )
    .await
    .unwrap();
    assert_eq!((report.visitors, report.pageviews), (2, 2));
    assert_eq!(report.pages.len(), 1);
    assert_eq!(report.pages[0].name, "/docs");
    assert_eq!(report.browsers[0].name, "Chrome");
}

#[tokio::test]
async fn report_sorting_is_independent_of_chart_metric_and_precedes_pagination() {
    use analytics::Metric;
    let db = fixture("UTC").await;
    let now = instant(27, 12);
    for page in 0..105 {
        for visitor in 0..2 {
            let id = page * 2 + visitor + 1;
            add(
                &db,
                id,
                &format!("/page/{page:02}"),
                &format!("203.0.113.{id}"),
                now,
            )
            .await;
        }
    }
    for id in 1000..1080 {
        add(&db, id, "/returning", "203.0.114.200", now).await;
    }
    let query = ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        ..Default::default()
    };
    let range = query.range("2026-10-27".parse().unwrap()).unwrap();
    for browser in [None, Some("Chrome".to_owned())] {
        let visitors = analytics::report(
            &db,
            1,
            range.clone(),
            ReportQuery {
                browser: browser.clone(),
                ..query.clone()
            },
        )
        .await
        .unwrap();
        assert_eq!(visitors.pages.len(), 100);
        assert!(!visitors.pages.iter().any(|r| r.name == "/returning"));
        let views = analytics::report(
            &db,
            1,
            range.clone(),
            ReportQuery {
                browser,
                metric: Metric::Pageviews,
                ..query.clone()
            },
        )
        .await
        .unwrap();
        assert_eq!((views.visitors, views.pageviews), (211, 290));
        assert_eq!(views.pages.len(), 100);
        assert!(!views.pages.iter().any(|row| row.name == "/returning"));
        let views = analytics::report(
            &db,
            1,
            range.clone(),
            ReportQuery {
                detail: Some("page".into()),
                sort: Some("pageviews".into()),
                ..query.clone()
            },
        )
        .await
        .unwrap();
        assert_eq!(views.pages.len(), 50);
        assert_eq!(views.detail_total, 106);
        assert_eq!(
            (
                views.pages[0].name.as_str(),
                views.pages[0].visitors,
                views.pages[0].pageviews
            ),
            ("/returning", 1, 80)
        );
        assert_eq!(views.browsers[0].visitors, views.visitors);
        assert_eq!(views.browsers[0].pageviews, views.pageviews);
    }
    let tail = analytics::report(
        &db,
        1,
        range.clone(),
        ReportQuery {
            detail: Some("page".into()),
            sort: Some("name".into()),
            order: Some("asc".into()),
            offset: Some(100),
            ..query.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(tail.pages.len(), 6);
    assert_eq!(tail.pages.last().unwrap().name, "/returning");
    let search = analytics::report(
        &db,
        1,
        range.clone(),
        ReportQuery {
            detail: Some("page".into()),
            search: Some("RETURNING".into()),
            ..query.clone()
        },
    )
    .await
    .unwrap();
    assert_eq!(search.detail_total, 1);
    assert_eq!(search.pages[0].name, "/returning");
    let all = ReportQuery {
        page: Some("/returning".into()),
        source: Some("search.example".into()),
        country: Some("".into()),
        browser: Some("Chrome".into()),
        device: Some("Desktop".into()),
        metric: Metric::Pageviews,
        ..query
    };
    let report = analytics::report(&db, 1, range.clone(), all.clone())
        .await
        .unwrap();
    assert_eq!((report.visitors, report.pageviews), (1, 80));
    assert!(report.countries.is_empty());
    for rows in [
        &report.pages,
        &report.sources,
        &report.browsers,
        &report.devices,
    ] {
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].visitors, rows[0].pageviews), (1, 80));
    }
    let empty = analytics::report(
        &db,
        1,
        range,
        ReportQuery {
            page: Some("/missing".into()),
            ..all
        },
    )
    .await
    .unwrap();
    assert_eq!((empty.visitors, empty.pageviews), (0, 0));
    assert!(empty.has_data);
    assert!(empty.pages.is_empty() && empty.sources.is_empty() && empty.browsers.is_empty());
}

#[test]
fn report_dates_reject_extreme_years_without_panicking() {
    let today = "2026-10-27".parse().unwrap();
    for date in [
        "-262144-01-01",
        "+262142-12-31",
        "0000-01-01",
        "2026-1-1",
        "invalid",
    ] {
        assert!(
            ReportQuery {
                to: Some(date.into()),
                ..Default::default()
            }
            .range(today)
            .is_err(),
            "{date}"
        );
    }
    assert!(
        ReportQuery {
            from: Some("0001-01-01".into()),
            to: Some("0001-01-01".into()),
            ..Default::default()
        }
        .range(today)
        .is_ok()
    );
}

#[test]
fn calendar_anchors_preserve_full_past_periods_and_clip_only_the_current_period() {
    let today = "2024-03-15".parse().unwrap();
    for (period, date, from, to) in [
        ("day", "2024-02-29", "2024-02-29", "2024-02-29"),
        ("month", "2024-02-29", "2024-02-01", "2024-02-29"),
        ("month", "2023-02-28", "2023-02-01", "2023-02-28"),
        ("month", "2024-03-31", "2024-03-01", "2024-03-15"),
        ("year", "2023-12-31", "2023-01-01", "2023-12-31"),
        ("year", "2024-02-29", "2024-01-01", "2024-03-15"),
        ("month", "9999-12-01", "9999-12-01", "9999-12-31"),
        ("year", "0001-06-15", "0001-01-01", "0001-12-31"),
    ] {
        let range = ReportQuery {
            period: Some(period.into()),
            date: Some(date.into()),
            ..Default::default()
        }
        .range(today)
        .unwrap();
        assert_eq!(
            (range.from.to_string(), range.to.to_string()),
            (from.into(), to.into()),
            "{period}/{date}"
        );
    }
    let range = ReportQuery {
        date: Some("2024-02-29".into()),
        ..Default::default()
    }
    .range(today)
    .unwrap();
    assert_eq!(range.from.to_string(), "2024-02-29");
    assert_eq!(range.from, range.to);
    for date in [
        "0000-01-01",
        "10000-01-01",
        "2024-2-1",
        "2023-02-29",
        "<script>",
    ] {
        assert!(
            ReportQuery {
                period: Some("month".into()),
                date: Some(date.into()),
                ..Default::default()
            }
            .range(today)
            .is_err(),
            "{date}"
        );
    }
}

#[test]
fn canonical_domains_match_without_accepting_subdomains_or_self_referrals() {
    for host in ["example.com.", "www.example.com.", "WWW.Example.COM"] {
        assert!(analytics::matches_domain(host, "example.com"));
    }
    for host in [
        "other.example.com",
        "www.www.example.com",
        "example.com.evil.test",
    ] {
        assert!(!analytics::matches_domain(host, "example.com"));
    }
    for url in [
        "https://example.com./private",
        "https://www.example.com./path?secret=yes",
    ] {
        assert_eq!(analytics::source(url, "example.com"), "");
    }
    assert_eq!(
        analytics::source("https://www.search.example./path", "example.com"),
        "search.example"
    );
    assert_eq!(
        analytics::source("https://www.www.example.com/path", "example.com"),
        "www.example.com"
    );
}

#[tokio::test]
async fn queued_collection_samples_the_clock_in_the_writer_across_midnight() {
    use std::{
        future::Future,
        sync::{
            Arc,
            atomic::{AtomicI64, AtomicUsize, Ordering},
        },
        task::{Context, Waker},
    };
    let db = fixture("UTC").await;
    add(&db, 1, "/", "203.0.113.1", instant(26, 23)).await;
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let worker_db = db.clone();
    let blocker = tokio::spawn(async move {
        worker_db
            .call(move |_| {
                let _ = started.send(());
                blocked
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                Ok(())
            })
            .await
    });
    ready.await.unwrap();
    let clock = Arc::new(AtomicI64::new(instant(26, 23)));
    let samples = Arc::new(AtomicUsize::new(0));
    let (time, reads) = (clock.clone(), samples.clone());
    let mut queued = Box::pin(analytics::collect(
        &db,
        event(2, "/"),
        "203.0.113.1".parse().unwrap(),
        UA.into(),
        move || {
            reads.fetch_add(1, Ordering::SeqCst);
            time.load(Ordering::SeqCst)
        },
    ));
    assert!(
        queued
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(
        samples.load(Ordering::SeqCst),
        0,
        "Clock was sampled before the queued write"
    );
    clock.store(instant(27, 0), Ordering::SeqCst);
    release.send(()).unwrap();
    blocker.await.unwrap().unwrap();
    assert_eq!(queued.await.unwrap(), Collected::Stored);
    add(&db, 3, "/next", "203.0.113.1", instant(27, 0)).await;
    db.read(|c| {
        let (day, views, visitors): (String, i64, i64) = c.query_row(
            "SELECT day,COUNT(*),COUNT(DISTINCT visitor) FROM events WHERE request_id!=?1",
            [1u128.to_be_bytes().as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        assert_eq!((day.as_str(), views, visitors), ("2026-10-27", 2, 1));
        assert_eq!(
            c.query_row("SELECT day FROM site_salts WHERE site_id=1", [], |r| r
                .get::<_, String>(
                0
            ))?,
            "2026-10-27"
        );
        Ok(())
    })
    .await
    .unwrap();
}
