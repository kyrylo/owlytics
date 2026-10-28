use chrono::{Duration, TimeZone};
use owlytics::{
    analytics::{self, Metric, Report, ReportQuery},
    db::Database,
    filters::Filter,
};

async fn fixture(zone: &str, day: &str) -> (Database, i64, i64) {
    let zone: chrono_tz::Tz = zone.parse().unwrap();
    let day: chrono::NaiveDate = day.parse().unwrap();
    let start = zone
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap()
        .timestamp();
    let end = zone
        .from_local_datetime(&day.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap()
        .timestamp();
    let db = Database::open(":memory:").unwrap();
    db.call(move |c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com',?1,0),(2,'other.example',?1,0)",[zone.name()])?;
        for (site, person, source, path, stamp) in [
            (1, 1_u128, "Google", "/entry", start-60),
            (1, 1, "Google", "/seen", start+60),
            (1, 2, "GitHub", "/seen", start+120),
            (1, 3, "Outside", "/seen", start-61),
            (1, 4, "After", "/seen", end),
            (2, 5, "Other site", "/seen", start+60),
        ] {
            let date=chrono::DateTime::from_timestamp(stamp,0).unwrap().with_timezone(&zone).date_naive().to_string();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(?1,?2,?3,?4,randomblob(16),?5,?6,'US','Chrome','Desktop')",
                rusqlite::params![site,stamp,date,person.to_be_bytes().as_slice(),path,source])?;
        }
        c.execute("INSERT INTO session_acquisition SELECT id,'Referral',source,'referral','seed','','',NULL FROM sessions",[])?;
        Ok(())
    }).await.unwrap();
    (db, start, end)
}

fn query(day: &str) -> ReportQuery {
    ReportQuery {
        from: Some(day.into()),
        to: Some(day.into()),
        browser: Some("Chrome".into()),
        interval: Some("day".into()),
        ..Default::default()
    }
}

async fn report(db: &Database, q: ReportQuery) -> Report {
    let range = q.range("2026-10-28".parse().unwrap()).unwrap();
    analytics::report(db, 1, range, q).await.unwrap()
}

#[tokio::test]
async fn filtered_and_rolling_reports_keep_visits_that_began_before_the_local_date() {
    for (zone, day, hours) in [
        ("UTC", "2026-10-27", 24),
        ("Asia/Kathmandu", "2026-10-27", 24),
        ("America/New_York", "2026-03-08", 23),
        ("America/New_York", "2026-11-01", 25),
    ] {
        let (db, start, end) = fixture(zone, day).await;
        assert_eq!(end - start, hours * 3600);
        for window in [None, Some((start, end)), Some((start, start + 180))] {
            for page in [None, Some("/seen".into())] {
                let r = report(
                    &db,
                    ReportQuery {
                        window,
                        page,
                        ..query(day)
                    },
                )
                .await;
                assert_eq!(
                    (r.visitors, r.pageviews, r.totals.visits),
                    (2, 2, 2),
                    "{zone}/{window:?}"
                );
                assert_eq!(Metric::ViewsPerVisit.value(&r.totals), 1.5);
                assert_eq!(Metric::VisitDuration.value(&r.totals), 60.);
                assert_eq!(r.sources.len(), 2);
                let google = r.sources.iter().find(|r| r.name == "Google").unwrap();
                assert_eq!(
                    (
                        google.visitors,
                        google.pageviews,
                        google.visits,
                        google.duration
                    ),
                    (1, 2, 1, 120)
                );
                assert_eq!(google.percentage, Some((1, 2)));
                assert_eq!(r.daily.len(), 1);
                assert_eq!(r.daily[0].name, day);
                assert_eq!(r.daily[0].mean_visits(), 2);
            }
        }
        // The hourly path also selects overlapping visits when summaries are
        // used for other parts of the report. Its first hour includes both.
        let r = report(
            &db,
            ReportQuery {
                browser: None,
                interval: Some("hour".into()),
                ..query(day)
            },
        )
        .await;
        assert_eq!((r.timeline[0].visitors, r.timeline[0].visits), (2, 2));
        assert_eq!(r.timeline[0].completed.duration, 120);
    }
}

#[tokio::test]
async fn missing_campaign_coverage_observes_the_same_cross_midnight_visits() {
    let day = "2026-10-27";
    let (db, start, _) = fixture("UTC", day).await;
    db.call(|c|c.execute("DELETE FROM session_acquisition WHERE session_id=(SELECT id FROM sessions WHERE site_id=1 AND source='Google')",[])).await.unwrap();
    for page in [None, Some("/seen".into())] {
        let r = report(
            &db,
            ReportQuery {
                page,
                filters: vec![Filter {
                    key: "utm_campaign".into(),
                    op: Default::default(),
                    values: vec!["seed".into()],
                }],
                ..query(day)
            },
        )
        .await;
        assert!(r.acquisition_incomplete.contains("utm_campaign"));
    }
    let later = report(
        &db,
        ReportQuery {
            window: Some((start + 180, start + 300)),
            filters: vec![Filter {
                key: "utm_campaign".into(),
                op: Default::default(),
                values: vec!["seed".into()],
            }],
            ..query(day)
        },
    )
    .await;
    assert!(!later.acquisition_incomplete.contains("utm_campaign"));
}

#[tokio::test]
async fn selected_dates_bound_rolling_visits_and_keep_empty_intersections_empty() {
    let day = "2026-10-27";
    let (db, start, end) = fixture("UTC", day).await;
    for window in [(start - 3600, start), (end, end + 3600)] {
        let r = report(
            &db,
            ReportQuery {
                window: Some(window),
                ..query(day)
            },
        )
        .await;
        assert_eq!((r.visitors, r.pageviews, r.totals.mean_visits()), (0, 0, 0));
        assert!(r.sources.is_empty());
    }
    let r = report(
        &db,
        ReportQuery {
            window: Some((start - 3600, end + 3600)),
            ..query(day)
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.mean_visits()), (2, 2, 2));
    assert!(
        !r.sources
            .iter()
            .any(|r| r.name == "Outside" || r.name == "After")
    );
    // Clipping the report before a visit ends keeps its full duration.
    let previous =
        ("2026-10-27".parse::<chrono::NaiveDate>().unwrap() - Duration::days(1)).to_string();
    let r = report(
        &db,
        ReportQuery {
            window: Some((start - 60, start)),
            ..query(&previous)
        },
    )
    .await;
    assert_eq!(
        (r.visitors, r.pageviews, r.sources[0].duration),
        (1, 1, 120)
    );
}

#[tokio::test]
async fn visit_lookback_is_bounded_by_instants_instead_of_local_dates() {
    let day = "2026-10-27";
    let (db, start, _) = fixture("Asia/Kathmandu", day).await;
    db.call(move |c| {
        // Model long-lived visits directly to exercise the report boundary,
        // independently of the collector's shorter visitor-identity lifetime.
        for (person, offset, source) in [(10_u128, 0, "Boundary"), (11, -1, "Too old")] {
            let stamp = start - 604800 + offset;
            let date = chrono::DateTime::from_timestamp(stamp,0).unwrap().with_timezone(&chrono_tz::Asia::Kathmandu).date_naive().to_string();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,browser,country,device) VALUES(1,?1,?4,?2,randomblob(16),'/entry',?3,'Chrome','US','Desktop')",
                rusqlite::params![stamp,person.to_be_bytes().as_slice(),source,date])?;
            c.execute("UPDATE sessions SET active_at=?1 WHERE site_id=1 AND source=?2",rusqlite::params![start,source])?;
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,browser,country,device) VALUES(1,?1,'2026-10-27',?2,randomblob(16),'/long',?3,'Chrome','US','Desktop')",
                rusqlite::params![start+30,person.to_be_bytes().as_slice(),source])?;
        }
        Ok(())
    }).await.unwrap();
    let r = report(
        &db,
        ReportQuery {
            page: Some("/long".into()),
            ..query(day)
        },
    )
    .await;
    // Both matching pageviews exist, but only the visit starting within the
    // reference's seven-day lookback contributes visit-based breakdowns.
    assert_eq!((r.visitors, r.pageviews), (2, 2));
    assert_eq!(r.sources.len(), 1);
    assert_eq!(r.sources[0].name, "Boundary");
    assert_eq!(r.sources[0].duration, 604830);
}
