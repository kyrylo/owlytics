use owlytics::{
    analytics::{self, Collected, Event, ReportQuery},
    db::Database,
    storage,
};
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/130.0.0.0 Safari/537.36";
async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    db
}
async fn add(
    db: &Database,
    id: u128,
    path: &str,
    referrer: &str,
    ip: &str,
    time: i64,
) -> Collected {
    analytics::collect(
        db,
        Event {
            domain: "example.com".into(),
            path: path.into(),
            referrer: referrer.into(),
            id: format!("{id:032x}"),
            campaign: None,
        },
        ip.parse().unwrap(),
        UA.into(),
        move || time,
    )
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
const START: i64 = 1793059200; // 2026-10-27 00:00 UTC

#[tokio::test]
async fn later_pageviews_keep_the_visits_original_location_in_every_report_scope() {
    let db = fixture().await;
    db.call(|c| {
        for (id,seconds,path,country) in [(1,0,"/","US"),(2,30,"/a","PH")] {
            // Refreshing geolocation during a visit must not split its audience.
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,'2026-10-27',zeroblob(16),?2,?3,'',?4,'Chrome','Desktop')",
                rusqlite::params![START+seconds,i128::to_be_bytes(id).as_slice(),path,country])?;
        }
        Ok(())
    }).await.unwrap();
    for scope in [
        analytics::ReportScope::All,
        analytics::ReportScope::Timeline,
    ] {
        for page in [None, Some("/a".to_owned())] {
            for country in ["US", "PH"] {
                let r = report(
                    &db,
                    ReportQuery {
                        country: Some(country.into()),
                        page: page.clone(),
                        interval: Some("day".into()),
                        scope,
                        ..query()
                    },
                )
                .await;
                let expected = if country == "PH" {
                    (0, 0, 0, 0)
                } else {
                    (1, if page.is_some() { 1 } else { 2 }, 1, 1)
                };
                assert_eq!(
                    (
                        r.visitors,
                        r.pageviews,
                        r.totals.visits,
                        r.totals.mean_visits()
                    ),
                    expected,
                    "{scope:?}, {country}, {page:?}"
                );
                if country == "US" {
                    assert_eq!(r.totals.mean_pageviews(), 2);
                    assert_eq!(r.totals.duration, 30);
                }
            }
        }
    }
}

#[tokio::test]
async fn selected_reports_match_full_reports_with_filters_history_and_versions() {
    use analytics::ReportScope;
    let db = fixture().await;
    for (id, path, ip, stamp) in [
        (1, "/", "203.0.113.1", START),
        (2, "/docs", "203.0.113.1", START + 30),
        (3, "/docs", "203.0.113.2", START + 60),
        (4, "/", "203.0.113.1", START + 3600),
    ] {
        add(&db, id, path, "https://google.com/search", ip, stamp).await;
    }
    db.call(|c| {
        c.execute_batch(
            "INSERT INTO import_batches VALUES(1,1,'2026-10-26','2026-10-26',0);
        INSERT INTO import_daily VALUES(1,'2026-10-26',1,3,7);
        INSERT INTO import_breakdowns VALUES(1,'2026-10-26','page','/docs',1,7);
        INSERT INTO import_breakdowns VALUES(1,'2026-10-26','country','US',1,7);",
        )
    })
    .await
    .unwrap();
    for variant in 0..5 {
        let mut q = query();
        match variant {
            0 => q.from = Some("2026-10-26".into()),
            1 => {
                q.browser = Some("Chrome".into());
                q.os = Some("macOS".into());
            }
            2 => q.page = Some("/docs".into()),
            3 => q.source = Some("Google".into()),
            _ => q.window = Some((START, START + 300)),
        }
        let all = report(&db, q.clone()).await;
        let timeline = report(
            &db,
            ReportQuery {
                scope: ReportScope::Timeline,
                ..q.clone()
            },
        )
        .await;
        assert_eq!(timeline.totals, all.totals);
        assert_eq!(timeline.daily, all.daily);
        assert_eq!(timeline.timeline, all.timeline);
        assert!(timeline.pages.is_empty() && timeline.browsers.is_empty());
        for (pages, locations, device) in [
            ("page", "country", "browser"),
            ("entry", "region", "os"),
            ("exit", "city", "device"),
        ] {
            let selected = report(
                &db,
                ReportQuery {
                    scope: ReportScope::Dashboard,
                    pages_tab: Some(pages.into()),
                    locations_tab: Some(locations.into()),
                    devices_tab: Some(device.into()),
                    ..q.clone()
                },
            )
            .await;
            assert_eq!(selected.totals, all.totals);
            assert_eq!(selected.daily, all.daily);
            assert_eq!(selected.timeline, all.timeline);
            assert_eq!(
                selected.session_metrics_available,
                all.session_metrics_available
            );
            assert_eq!(selected.breakdown_pageviews, all.breakdown_pageviews);
            let (a, b) = match pages {
                "entry" => (&selected.entry_pages, &all.entry_pages),
                "exit" => (&selected.exit_pages, &all.exit_pages),
                _ => (&selected.pages, &all.pages),
            };
            assert_eq!(a, b, "{variant}: {pages}");
            let (a, b) = match locations {
                "region" => (&selected.regions, &all.regions),
                "city" => (&selected.cities, &all.cities),
                _ => (&selected.countries, &all.countries),
            };
            assert_eq!(a, b, "{variant}: {locations}");
            let (a, b) = match device {
                "os" if q.os.is_some() => (&selected.os_versions, &all.os_versions),
                "os" => (&selected.operating_systems, &all.operating_systems),
                "device" => (&selected.devices, &all.devices),
                _ if q.browser.is_some() => (&selected.browser_versions, &all.browser_versions),
                _ => (&selected.browsers, &all.browsers),
            };
            assert_eq!(a, b, "{variant}: {device}");
            if q.source.is_some() {
                assert_eq!(selected.referrers, all.referrers);
            } else {
                assert_eq!(selected.sources, all.sources);
            }
        }
    }
    // Unknown URL values never become SQL identifiers. They use the visible
    // default tab, while the HTTP date/options validator handles invalid detail.
    let invalid = report(
        &db,
        ReportQuery {
            browser: Some("Chrome".into()),
            scope: ReportScope::Dashboard,
            devices_tab: Some("day FROM events; DROP TABLE sites; --".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(invalid.pageviews, 4);
    assert!(!invalid.browser_versions.is_empty());
}

#[tokio::test]
async fn sessions_attribute_entry_exit_and_source_with_exact_inactivity_boundary() {
    let db = fixture().await;
    add(
        &db,
        1,
        "/",
        "https://google.com/search",
        "203.0.113.1",
        START,
    )
    .await;
    add(
        &db,
        2,
        "/pricing",
        "https://example.com/",
        "203.0.113.1",
        START + 60,
    )
    .await;
    add(
        &db,
        3,
        "/thanks",
        "https://example.com/pricing",
        "203.0.113.1",
        START + 1860,
    )
    .await;
    add(
        &db,
        4,
        "/blog",
        "https://github.com/",
        "203.0.113.1",
        START + 3720,
    )
    .await;
    add(&db, 5, "/", "", "203.0.113.2", START + 3700).await;
    assert_eq!(
        add(&db, 5, "/bad", "", "203.0.113.2", START + 3800).await,
        Collected::Duplicate
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
        (2, 5, 3, 2, 1860)
    );
    assert_eq!(
        r.entry_pages.iter().find(|p| p.name == "/").unwrap().visits,
        2
    );
    let exit = r.exit_pages.iter().find(|p| p.name == "/thanks").unwrap();
    assert_eq!((exit.exits, exit.pageviews), (1, 1));
    assert_eq!(
        r.sources
            .iter()
            .find(|p| p.name == "Google")
            .unwrap()
            .pageviews,
        3
    );
    assert_eq!(r.operating_systems[0].name, "macOS");
    assert_eq!(r.browser_versions[0].name, "130.0.0.0");
    assert_eq!(r.os_versions[0].name, "10.15.7");
    let filtered = report(
        &db,
        ReportQuery {
            entry: Some("/".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(
        (
            filtered.pageviews,
            filtered.totals.visits,
            filtered.totals.bounces
        ),
        (4, 2, 1)
    );
    let filtered = report(
        &db,
        ReportQuery {
            exit: Some("/thanks".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((filtered.visitors, filtered.pageviews), (1, 3));
}
#[tokio::test]
async fn daily_rotation_keeps_active_sessions_across_midnight() {
    let db = fixture().await;
    add(&db, 1, "/", "", "203.0.113.1", START + 86399).await;
    add(&db, 2, "/pricing", "", "203.0.113.1", START + 86401).await;
    db.read(|c| {
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get::<_, i64>(0))?,
            1
        );
        Ok(())
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn geographic_regions_and_cities_use_sqlite_and_discard_coordinates() {
    let db = fixture().await;
    db.call(|c| {
        storage::import_geo(
            c,
            "203.0.113.0,203.0.113.255,AS,PH,Metro Manila,Manila,14.5995,120.9842\n".as_bytes(),
        )
        .unwrap();
        Ok(())
    })
    .await
    .unwrap();
    add(&db, 1, "/", "", "203.0.113.1", START).await;
    let r = report(&db, query()).await;
    assert_eq!(r.regions[0].name, "PH: PH-00");
    assert_eq!(r.cities[0].name, "PH: 1701668");
    assert_eq!(r.countries[0].name, "PH");
    let filtered = report(
        &db,
        ReportQuery {
            region: Some("PH: Metro Manila".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((filtered.visitors, filtered.totals.visits), (1, 1));
    db.call(|c| {
        storage::import_geo(c, "203.0.113.0,203.0.113.255,US\n".as_bytes()).unwrap();
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM geo_locations", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    })
    .await
    .unwrap();
}
#[test]
fn parses_software_without_confusing_browser_and_os_versions() {
    assert_eq!(
        analytics::software(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) Version/18.1 Mobile Safari/604.1"
        ),
        ("18.1".into(), "iOS".into(), "18.1".into())
    );
    assert_eq!(
        analytics::software("Mozilla/5.0 (Windows NT 10.0) Chrome/130.0 Safari/537.36 Edg/131.0"),
        ("131.0".into(), "Windows".into(), "10".into())
    );
    assert_eq!(analytics::source_name("l.facebook.com"), "Facebook");
    assert_eq!(analytics::source_name("notgoogle.com"), "notgoogle.com");
}

#[test]
fn period_presets_and_weighted_metrics_match_their_definitions() {
    let today = "2026-10-27".parse().unwrap();
    for (period, from, to) in [
        ("day", "2026-10-27", "2026-10-27"),
        ("yesterday", "2026-10-26", "2026-10-26"),
        ("7d", "2026-10-20", "2026-10-26"),
        ("28d", "2026-09-29", "2026-10-26"),
        ("30d", "2026-09-27", "2026-10-26"),
        ("91d", "2026-07-28", "2026-10-26"),
        ("month", "2026-10-01", "2026-10-27"),
        ("lastmonth", "2026-09-01", "2026-09-30"),
        ("year", "2026-01-01", "2026-10-27"),
        ("6mo", "2026-04-01", "2026-09-30"),
        ("12mo", "2025-10-01", "2026-09-30"),
    ] {
        let range = ReportQuery {
            period: Some(period.into()),
            ..Default::default()
        }
        .range(today)
        .unwrap();
        assert_eq!(
            (range.from.to_string(), range.to.to_string()),
            (from.into(), to.into())
        );
    }
    assert!(
        ReportQuery {
            period: Some("bad".into()),
            ..Default::default()
        }
        .range(today)
        .is_err()
    );
    let row = analytics::Row {
        pageviews: 7,
        visits: 3,
        bounces: 2,
        duration: 66,
        ..Default::default()
    };
    assert!((analytics::Metric::ViewsPerVisit.value(&row) - 7. / 3.).abs() < 1e-6);
    assert!((analytics::Metric::BounceRate.value(&row) - 200. / 3.).abs() < 1e-6);
    assert_eq!(analytics::Metric::VisitDuration.value(&row), 22.);
}

#[tokio::test]
async fn rolling_windows_filter_events_but_preserve_session_attribution() {
    let db = fixture().await;
    add(&db, 1, "/", "https://google.com/", "203.0.113.1", START).await;
    add(&db, 2, "/pricing", "", "203.0.113.1", START + 60).await;
    add(&db, 3, "/thanks", "", "203.0.113.1", START + 1800).await;
    let r = report(
        &db,
        ReportQuery {
            window: Some((START + 30, START + 3600)),
            interval: Some("hour".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (1, 2, 1));
    assert_eq!(analytics::Metric::ViewsPerVisit.value(&r.totals), 3.);
    assert_eq!(analytics::Metric::ViewsPerVisit.value(&r.daily[0]), 3.);
    assert_eq!(r.sources[0].name, "Google");
    assert_eq!(r.timeline.len(), 1);
    assert_eq!((r.timeline[0].visitors, r.timeline[0].pageviews), (1, 2));
    assert_eq!(
        (r.timeline_start, r.timeline_end),
        (START + 30, START + 3600)
    );
}

#[tokio::test]
async fn clipped_windows_keep_session_means_independent_of_event_counts() {
    use analytics::Metric;
    let db = fixture().await;
    for (id, path, seconds) in [(1, "/", 0), (2, "/pricing", 60), (3, "/thanks", 1800)] {
        add(
            &db,
            id,
            path,
            "https://google.com/",
            "203.0.113.1",
            START + seconds,
        )
        .await;
    }
    for (start, end, pages) in [(0, 30, 1), (30, 50, 0), (30, 90, 1), (30, 3600, 2)] {
        for source in [None, Some("Google".into())] {
            let r = report(
                &db,
                ReportQuery {
                    window: Some((START + start, START + end)),
                    source,
                    interval: Some("day".into()),
                    ..query()
                },
            )
            .await;
            assert_eq!(
                (r.visitors, r.pageviews, r.totals.visits),
                (i64::from(pages > 0), pages, i64::from(pages > 0))
            );
            assert_eq!(Metric::ViewsPerVisit.value(&r.totals), 3.);
            assert_eq!(Metric::VisitDuration.value(&r.totals), 1800.);
            assert_eq!(Metric::BounceRate.value(&r.totals), 0.);
        }
    }
    let r = report(
        &db,
        ReportQuery {
            window: Some((START + 30, START + 50)),
            page: Some("/pricing".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (0, 0, 0));
    assert_eq!(Metric::ViewsPerVisit.value(&r.totals), 0.);
    assert_eq!(Metric::VisitDuration.value(&r.totals), 0.);
}

#[tokio::test]
async fn raw_reports_deduplicate_retained_identity_across_dates() {
    let db = fixture().await;
    // Model a retained identity directly. The live collector still rotates its
    // salt at midnight; enabling continuity requires the remaining cache work.
    db.call(|c| {
        for (stamp,day) in [(START-10,"2026-10-26"),(START+10,"2026-10-27")] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,zeroblob(16),randomblob(16),'/','Google','US','Chrome','Desktop')",rusqlite::params![stamp,day])?;
        }
        Ok(())
    }).await.unwrap();
    let r = report(
        &db,
        ReportQuery {
            from: Some("2026-10-26".into()),
            to: Some("2026-10-27".into()),
            window: Some((START - 86400, START + 86400)),
            interval: Some("day".into()),
            browser: Some("Chrome".into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (1, 2, 1));
    assert_eq!(analytics::Metric::ViewsPerVisit.value(&r.totals), 2.);
    assert_eq!(analytics::Metric::VisitDuration.value(&r.totals), 20.);
    assert_eq!(r.daily.len(), 2);
    assert_eq!((r.daily[0].visitors, r.daily[1].visitors), (1, 1));
    assert_eq!((r.daily[0].mean_visits(), r.daily[1].mean_visits()), (0, 1));
    assert_eq!(r.sources[0].percentage, Some((1, 1)));
}

#[tokio::test]
async fn hourly_active_visits_and_completed_session_metrics_are_distinct() {
    use analytics::Metric;
    let db = fixture().await;
    for (id, seconds, ip) in [
        (1, 0, "203.0.113.1"),
        (2, 1200, "203.0.113.2"),
        (3, 1860, "203.0.113.1"),
        (4, 2400, "203.0.113.2"),
        (5, 6900, "203.0.113.1"),
        (6, 8100, "203.0.113.1"),
    ] {
        add(&db, id, "/", "", ip, START + seconds).await;
    }
    let r = report(
        &db,
        ReportQuery {
            interval: Some("hour".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (2, 6, 4));
    assert_eq!(
        (
            r.timeline[0].visitors,
            r.timeline[0].visits,
            r.timeline[0].pageviews
        ),
        (2, 3, 4)
    );
    assert_eq!(r.timeline[0].value(Metric::ViewsPerVisit), Some(4. / 3.));
    assert_eq!(r.timeline[0].value(Metric::VisitDuration), Some(400.));
    for bucket in &r.timeline[1..3] {
        assert_eq!(
            (bucket.visitors, bucket.visits, bucket.pageviews),
            (1, 1, 1)
        );
    }
    assert_eq!(r.timeline[1].value(Metric::VisitDuration), None);
    assert_eq!(r.timeline[2].value(Metric::VisitDuration), Some(1200.));
    assert_eq!(r.timeline[2].value(Metric::ViewsPerVisit), Some(2.));
    assert_eq!(
        r.timeline.iter().map(|bucket| bucket.visits).sum::<i64>(),
        5
    );
}

#[tokio::test]
async fn minute_buckets_include_observed_activity_between_pageviews() {
    let db = fixture().await;
    add(&db, 1, "/", "", "203.0.113.1", START).await;
    add(&db, 2, "/next", "", "203.0.113.1", START + 600).await;
    let r = report(
        &db,
        ReportQuery {
            interval: Some("minute".into()),
            window: Some((START + 300, START + 480)),
            ..query()
        },
    )
    .await;
    assert_eq!(r.timeline.len(), 3);
    for bucket in &r.timeline {
        assert_eq!(
            (bucket.visitors, bucket.visits, bucket.pageviews),
            (1, 1, 0)
        );
        assert_eq!(bucket.value(analytics::Metric::VisitDuration), None);
    }
    let filtered = report(
        &db,
        ReportQuery {
            page: Some("/next".into()),
            interval: Some("minute".into()),
            window: Some((START + 300, START + 480)),
            ..query()
        },
    )
    .await;
    assert!(
        filtered
            .timeline
            .iter()
            .all(|bucket| bucket.visitors == 0 && bucket.visits == 0)
    );
}

#[tokio::test]
async fn hourly_counts_use_local_hours_and_distinguish_repeated_dst_hours() {
    let db = fixture().await;
    db.call(|c| c.execute("UPDATE sites SET timezone='Asia/Kolkata'", []))
        .await
        .unwrap();
    add(&db, 1, "/", "", "203.0.113.1", START + 9600).await;
    add(&db, 2, "/next", "", "203.0.113.1", START + 11100).await;
    let r = report(
        &db,
        ReportQuery {
            interval: Some("hour".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(r.timeline_start, START - 19800);
    assert_eq!(r.timeline[8].start, START + 9000);
    assert_eq!((r.timeline[8].pageviews, r.timeline[8].visits), (2, 1));
    assert_eq!(
        r.timeline[8].value(analytics::Metric::VisitDuration),
        Some(1500.)
    );
    db.call(|c| c.execute("UPDATE sites SET timezone='America/New_York'", []))
        .await
        .unwrap();
    let stamp = |value: &str| {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp()
    };
    add(
        &db,
        3,
        "/",
        "",
        "203.0.113.1",
        stamp("2026-11-01T05:50:00Z"),
    )
    .await;
    add(
        &db,
        4,
        "/next",
        "",
        "203.0.113.1",
        stamp("2026-11-01T06:10:00Z"),
    )
    .await;
    let r = report(
        &db,
        ReportQuery {
            from: Some("2026-11-01".into()),
            to: Some("2026-11-01".into()),
            interval: Some("hour".into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(r.timeline.len(), 25);
    assert_eq!((r.timeline[1].pageviews, r.timeline[2].pageviews), (1, 1));
    assert_eq!(r.timeline[1].value(analytics::Metric::VisitDuration), None);
    assert_eq!(
        r.timeline[2].value(analytics::Metric::VisitDuration),
        Some(1200.)
    );
    assert_eq!(r.timeline[1].start, stamp("2026-11-01T05:00:00Z"));
    assert_eq!(r.timeline[2].start, stamp("2026-11-01T06:00:00Z"));
}

#[tokio::test]
async fn source_brands_merge_and_referrer_drilldowns_keep_only_sanitized_paths() {
    let db = fixture().await;
    add(
        &db,
        1,
        "/",
        "https://www.google.com/search?email=secret@example.com#token",
        "203.0.113.1",
        START,
    )
    .await;
    add(
        &db,
        2,
        "/pricing",
        "https://github.com/other",
        "203.0.113.1",
        START + 60,
    )
    .await;
    add(
        &db,
        3,
        "/",
        "https://google.co.uk/search?q=private",
        "203.0.113.2",
        START + 100,
    )
    .await;
    let r = report(
        &db,
        ReportQuery {
            source: Some("Google".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (2, 3));
    assert_eq!(r.sources.len(), 1);
    assert_eq!(r.referrers.len(), 2);
    assert_eq!(
        (r.referrers[0].name.as_str(), r.referrers[0].pageviews),
        ("google.com/search", 2)
    );
    let detail = report(
        &db,
        ReportQuery {
            source: Some("Google".into()),
            referrer: Some("google.com/search".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(
        (detail.visitors, detail.pageviews, detail.totals.visits),
        (1, 2, 1)
    );
    db.read(|c|{assert_eq!(c.query_row("SELECT COUNT(*) FROM events WHERE instr(referrer,'?')>0 OR instr(referrer,'#')>0 OR instr(referrer,'secret')>0",[],|r|r.get::<_,i64>(0))?,0);Ok(())}).await.unwrap();
}

#[tokio::test]
async fn page_visits_include_non_entry_sessions_but_bounce_rate_uses_entrances() {
    let db = fixture().await;
    add(&db, 1, "/", "", "203.0.113.1", START).await;
    add(&db, 2, "/pricing", "", "203.0.113.1", START + 30).await;
    add(&db, 3, "/pricing", "", "203.0.113.1", START + 60).await;
    add(&db, 4, "/pricing", "", "203.0.113.2", START + 90).await;
    let r = report(&db, query()).await;
    let page = r.pages.iter().find(|row| row.name == "/pricing").unwrap();
    assert_eq!(
        (page.visits, page.entrances, page.pageviews, page.bounces),
        (2, Some(1), 3, 1)
    );
    assert_eq!(analytics::Metric::BounceRate.value(page), 100.);
    assert_eq!(analytics::Metric::BounceRate.value(&r.totals), 50.);
}

#[tokio::test]
async fn page_filtered_bounce_rates_use_entrances_without_reducing_visits() {
    use analytics::{Metric, ReportScope};
    let db = fixture().await;
    // Two entrances to /pricing (one bounce), plus a visit arriving from /.
    for (id, path, person, time) in [
        (1, "/", "203.0.113.1", START),
        (2, "/pricing", "203.0.113.1", START + 30),
        (3, "/pricing", "203.0.113.2", START + 60),
        (4, "/pricing", "203.0.113.3", START + 90),
        (5, "/docs", "203.0.113.3", START + 120),
    ] {
        add(&db, id, path, "https://google.com/search", person, time).await;
    }
    for scope in [
        ReportScope::All,
        ReportScope::Dashboard,
        ReportScope::Timeline,
    ] {
        let r = report(
            &db,
            ReportQuery {
                page: Some("/pricing".into()),
                source: Some("Google".into()),
                interval: Some("hour".into()),
                scope,
                ..query()
            },
        )
        .await;
        assert_eq!((r.visitors, r.pageviews, r.totals.visits), (3, 3, 3));
        for row in std::iter::once(&r.totals)
            .chain(&r.daily)
            .chain(
                r.timeline
                    .iter()
                    .filter(|bucket| bucket.completed.visits > 0)
                    .map(|bucket| &bucket.completed),
            )
            .chain(&r.sources)
            .chain(&r.countries)
            .chain(&r.browsers)
            .chain(&r.pages)
        {
            assert_eq!((row.bounces, row.entrances), (1, Some(2)), "{}", row.name);
            assert_eq!(Metric::BounceRate.value(row), 50., "{}", row.name);
        }
    }
    // A page never used as an entrance has a zero bounce rate, not a
    // fabricated denominator, and its traffic remains visible.
    let r = report(
        &db,
        ReportQuery {
            page: Some("/docs".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(
        (r.pageviews, r.totals.visits, r.totals.entrances),
        (1, 1, Some(0))
    );
    assert_eq!(Metric::BounceRate.value(&r.totals), 0.);
}

#[tokio::test]
async fn single_filter_comparisons_match_event_reports_across_days_and_sites() {
    let db = fixture().await;
    for (id, day, path, ip) in [
        (1, 0, "/", "203.0.113.1"),
        (2, 0, "/pricing", "203.0.113.1"),
        (3, 0, "/pricing", "203.0.113.2"),
        (4, 1, "/docs", "203.0.113.1"),
        (5, 1, "/pricing", "203.0.113.1"),
    ] {
        add(
            &db,
            id,
            path,
            "https://google.com/search",
            ip,
            START + day * 86400 + id as i64 * 30,
        )
        .await;
    }
    db.call(|c| c.execute_batch("INSERT INTO sites VALUES(2,'another.example','UTC',0); INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(2,1793059200,'2026-10-27',randomblob(16),randomblob(16),'/','Google','','Chrome','Desktop'); INSERT INTO import_batches VALUES(1,1,'2026-10-26','2026-10-26',0); INSERT INTO import_daily VALUES(1,'2026-10-26',1,100,200);")).await.unwrap();
    for dimension in [
        "source",
        "referrer",
        "country",
        "region",
        "city",
        "browser",
        "browser_version",
        "os",
        "os_version",
        "device",
    ] {
        for matched in [true, false] {
            let mut q = ReportQuery {
                from: Some("2026-10-26".into()),
                to: Some("2026-10-28".into()),
                interval: Some("day".into()),
                ..Default::default()
            };
            let (field, value) = match dimension {
                "source" => (&mut q.source, "Google"),
                "referrer" => (&mut q.referrer, "google.com/search"),
                "country" => (&mut q.country, ""),
                "region" => (&mut q.region, ""),
                "city" => (&mut q.city, ""),
                "browser" => (&mut q.browser, "Chrome"),
                "browser_version" => (&mut q.browser_version, "130.0.0.0"),
                "os" => (&mut q.os, "macOS"),
                "os_version" => (&mut q.os_version, "10.15.7"),
                _ => (&mut q.device, "Desktop"),
            };
            *field = Some(if matched { value } else { "missing" }.into());
            let full = report(&db, q.clone()).await;
            let summary = report(
                &db,
                ReportQuery {
                    scope: analytics::ReportScope::Timeline,
                    ..q
                },
            )
            .await;
            assert_eq!(
                summary.totals, full.totals,
                "{dimension}, matched={matched}"
            );
            assert_eq!(summary.daily, full.daily, "{dimension}, matched={matched}");
            assert_eq!(summary.imports_excluded, full.imports_excluded);
            assert!(summary.imports_excluded);
            assert_eq!(
                summary.pageviews,
                if matched { 5 } else { 0 },
                "{dimension}"
            );
        }
    }
}

fn counts(
    rows: &[analytics::Row],
) -> std::collections::BTreeMap<String, (i64, i64, i64, i64, i64, i64, Option<i64>)> {
    rows.iter()
        .map(|r| {
            (
                r.name.clone(),
                (
                    r.visitors,
                    r.pageviews,
                    r.visits,
                    r.bounces,
                    r.duration,
                    r.exits,
                    r.entrances,
                ),
            )
        })
        .collect()
}

#[tokio::test]
async fn daily_session_summaries_match_raw_reports_after_each_transition_and_retry() {
    let db = fixture().await;
    let q = ReportQuery {
        to: Some("2026-10-28".into()),
        ..query()
    };
    let mut id = 0;
    for day in 0..2 {
        for visit in 0..3 {
            for person in 0..3 {
                for step in 0..1 + (person + visit) % 4 {
                    id += 1;
                    let path = ["/shared", "/pricing", "/shared", "/docs"][step as usize];
                    let ip = format!("203.0.113.{}", person + 1);
                    let stamp = START + day * 86400 + visit * 3601 + person * 10 + step * 2;
                    add(&db, id, path, "https://google.com/search", &ip, stamp).await;
                    if id % 5 == 0 {
                        assert_eq!(
                            add(&db, id, path, "", &ip, stamp + 1).await,
                            Collected::Duplicate
                        );
                    }
                    let cached = report(&db, q.clone()).await;
                    let raw = report(
                        &db,
                        ReportQuery {
                            window: Some((START, START + 2 * 86400)),
                            ..q.clone()
                        },
                    )
                    .await;
                    assert_eq!(
                        counts(&[cached.totals]),
                        counts(&[raw.totals]),
                        "event {id}: totals"
                    );
                    for (dimension, a, b) in [
                        ("daily", cached.daily, raw.daily),
                        ("pages", cached.pages, raw.pages),
                        ("sources", cached.sources, raw.sources),
                        ("referrers", cached.referrers, raw.referrers),
                        ("entry", cached.entry_pages, raw.entry_pages),
                        ("exit", cached.exit_pages, raw.exit_pages),
                        ("country", cached.countries, raw.countries),
                        ("region", cached.regions, raw.regions),
                        ("city", cached.cities, raw.cities),
                        ("browser", cached.browsers, raw.browsers),
                        (
                            "browser_version",
                            cached.browser_versions,
                            raw.browser_versions,
                        ),
                        ("os", cached.operating_systems, raw.operating_systems),
                        ("os_version", cached.os_versions, raw.os_versions),
                        ("device", cached.devices, raw.devices),
                    ] {
                        assert_eq!(counts(&a), counts(&b), "event {id}: {dimension}");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn a_page_filter_requires_a_pageview_inside_the_rolling_window() {
    let db = fixture().await;
    add(&db, 1, "/old", "", "203.0.113.1", START).await;
    add(&db, 2, "/new", "", "203.0.113.1", START + 60).await;
    let r = report(
        &db,
        ReportQuery {
            page: Some("/old".into()),
            window: Some((START + 30, START + 90)),
            ..query()
        },
    )
    .await;
    assert_eq!((r.pageviews, r.totals.visits, r.totals.duration), (0, 0, 0));
    let r = report(
        &db,
        ReportQuery {
            page: Some("/new".into()),
            window: Some((START + 30, START + 90)),
            ..query()
        },
    )
    .await;
    assert_eq!(
        (r.pageviews, r.totals.visits, r.totals.duration),
        (1, 1, 60)
    );
}
