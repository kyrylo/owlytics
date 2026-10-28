use chrono::{Datelike, TimeZone};
use owlytics::{
    analytics::{self, Metric, ReportQuery, ReportScope},
    db::Database,
    filters::{Filter, Operator},
};

const NOW: i64 = 1793095320; // 2026-10-27 10:02 UTC; midnight in Kiritimati.
async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute_batch("INSERT INTO sites VALUES(1,'utc.example','UTC',0),(2,'midnight.example','Pacific/Kiritimati',0);")?;
        let events=[
            (1,-2100,"/older","Google","Chrome","US"),
            (1,-1200,"/older","Google","Chrome","US"),
            (2,-1500,"/older","Google","Chrome","US"),
            (2,-240,"/recent","Google","Chrome","US"),
            (3,-60,"/recent","GitHub","Safari","DE"),
            (4,-250,"/cross","","Firefox","PH"),
            (4,0,"/live","","Firefox","PH"),
            (5,60,"/future","Future","Other","UK"),
            (6,-301,"/expired","Bing","Edge","US"),
            (7,-300,"/boundary","Google","Chrome","US"),
            (8,-1801,"/outside","Old","Other","DE"),
            (9,-1800,"/thirty-boundary","DuckDuckGo","Firefox","UK"),
        ];
        for (site,zone) in [(1,chrono_tz::UTC),(2,chrono_tz::Pacific::Kiritimati)] {
            for (index,(person,offset,path,source,browser,country)) in events.iter().enumerate() {
                let stamp=NOW+offset;
                let day=chrono::DateTime::from_timestamp(stamp,0).unwrap().with_timezone(&zone).date_naive();
                let mut visitor=[*person as u8;16];
                visitor[..4].copy_from_slice(&day.num_days_from_ce().to_be_bytes());
                let request=[index as u8;16];
                c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,region,city,browser,browser_version,os,os_version,device) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'1','Linux','6','Desktop')",rusqlite::params![site,stamp,day.to_string(),visitor.as_slice(),request.as_slice(),path,source,format!("{source}/article"),country,format!("{country}: Region"),format!("{country}: City"),browser])?;
            }
        }
        c.execute("INSERT INTO session_acquisition(session_id,channel,utm_source,utm_medium,utm_campaign,utm_content,utm_term) SELECT id,CASE WHEN source='Google' THEN 'Organic Search' ELSE 'Referral' END,CASE WHEN source='Google' THEN 'google' ELSE '' END,'','','','' FROM sessions",[])?;
        Ok(())
    }).await.unwrap();
    db
}
fn query(scope: ReportScope) -> ReportQuery {
    ReportQuery {
        period: Some("realtime".into()),
        scope,
        window: Some((NOW - 1800, NOW + 1)),
        ..Default::default()
    }
}
async fn report(db: &Database, site: i64, q: ReportQuery) -> analytics::Report {
    let site = analytics::site(db, site).await.unwrap().unwrap();
    let zone = site.timezone.parse::<chrono_tz::Tz>().unwrap();
    let today = zone.timestamp_opt(NOW, 0).unwrap().date_naive();
    analytics::report(db, site.id, q.range(today).unwrap(), q)
        .await
        .unwrap()
}
#[tokio::test]
async fn realtime_uses_thirty_minute_headlines_and_five_minute_reports_in_every_scope() {
    let db = fixture().await;
    for (site, visitors, current) in [(1, 7, 4), (2, 8, 5)] {
        let full = report(&db, site, query(ReportScope::All)).await;
        assert!(full.realtime);
        assert_eq!(
            (full.visitors, full.pageviews, full.current_visitors),
            (visitors, 9, current)
        );
        assert_eq!(full.timeline.len(), 30);
        assert_eq!(
            full.timeline.iter().map(|b| b.pageviews).sum::<i64>(),
            8,
            "The unfinished minute is omitted"
        );
        assert!(
            full.timeline
                .iter()
                .filter(|b| b.start >= NOW - 1800 && b.start < NOW - 1200)
                .all(|b| b.visitors >= 1),
            "Observed session activity spans minute buckets"
        );
        let pages = full
            .pages
            .iter()
            .map(|row| row.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            pages,
            std::collections::BTreeSet::from(["/recent", "/cross", "/live", "/boundary"])
        );
        assert!(
            full.entry_pages.iter().any(|row| row.name == "/older"),
            "An active visit keeps its original entrance"
        );
        assert!(
            !full.exit_pages.iter().any(|row| row.name == "/older"),
            "Inactive visits do not contribute exits"
        );
        let google = full
            .sources
            .iter()
            .find(|row| row.name == "Google")
            .unwrap();
        assert_eq!(
            (google.visitors, google.percentage),
            (2, Some((2, current)))
        );
        let campaign = &full.acquisition["utm_source"][0];
        assert_eq!(
            (
                campaign.name.as_str(),
                campaign.visitors,
                campaign.percentage
            ),
            ("google", 2, Some((2, 2)))
        );
        for scope in [ReportScope::Dashboard, ReportScope::Timeline] {
            let r = report(&db, site, query(scope)).await;
            assert_eq!(r.totals, full.totals);
            assert_eq!(r.timeline, full.timeline);
            assert_eq!(r.current_visitors, current);
            if scope == ReportScope::Dashboard {
                assert_eq!(r.pages, full.pages);
                assert_eq!(r.sources, full.sources);
            }
        }
        let options = report(
            &db,
            site,
            ReportQuery {
                filter: Some("page".into()),
                detail: Some("page".into()),
                ..query(ReportScope::Suggestions)
            },
        )
        .await;
        assert_eq!((options.visitors, options.pageviews), (current, 5));
        assert_eq!(options.pages, full.pages);
        assert!(options.timeline.is_empty());
        let exported = report(
            &db,
            site,
            ReportQuery {
                export: true,
                ..query(ReportScope::All)
            },
        )
        .await;
        assert_eq!(exported.totals, full.totals);
        assert_eq!(exported.pages, full.pages);
    }
}

#[tokio::test]
async fn realtime_filters_keep_the_current_visitor_count_global_and_report_percentages_local() {
    let db = fixture().await;
    for q in [
        ReportQuery {
            source: Some("Google".into()),
            ..query(ReportScope::All)
        },
        ReportQuery {
            filters: vec![Filter {
                key: "utm_source".into(),
                op: Operator::Is,
                values: vec!["google".into()],
            }],
            ..query(ReportScope::All)
        },
    ] {
        let r = report(&db, 1, q).await;
        assert_eq!((r.visitors, r.pageviews, r.current_visitors), (3, 4, 4));
        assert_eq!(r.sources[0].percentage, Some((2, 2)));
        assert_eq!(r.pages.len(), 2);
        assert_eq!(r.pages.iter().map(|r| r.pageviews).sum::<i64>(), 2);
        assert_eq!(r.browsers[0].name, "Chrome");
        assert_eq!(r.browser_versions[0].name, "1");
        assert_eq!(r.regions[0].name, "US: Region");
        assert_eq!(r.operating_systems[0].name, "Linux");
        assert_eq!(r.devices[0].visitors, 2);
    }
    let r = report(
        &db,
        1,
        ReportQuery {
            page: Some("/older".into()),
            metric: Metric::Pageviews,
            ..query(ReportScope::All)
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.current_visitors), (2, 2, 4));
    assert!(r.pages.is_empty() && r.sources.is_empty());
    let r = report(
        &db,
        1,
        ReportQuery {
            source: Some("Missing".into()),
            ..query(ReportScope::All)
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.current_visitors), (0, 0, 4));
    assert_eq!(r.timeline.len(), 30);
    assert!(
        r.timeline
            .iter()
            .all(|bucket| bucket.visitors == 0 && bucket.pageviews == 0)
    );
    assert!(r.sources.is_empty());
}

#[tokio::test]
async fn realtime_campaign_coverage_follows_the_headline_window_and_other_filters() {
    let db = fixture().await;
    db.call(|c| {
        c.execute("DELETE FROM session_acquisition WHERE session_id IN(SELECT id FROM sessions WHERE entry_page IN('/outside','/future'))", [])?;
        Ok(())
    }).await.unwrap();
    let q = ReportQuery {
        filters: vec![Filter {
            key: "utm_source".into(),
            op: Operator::Is,
            values: vec!["google".into()],
        }],
        ..query(ReportScope::Dashboard)
    };
    assert!(
        report(&db, 1, q.clone())
            .await
            .acquisition_incomplete
            .is_empty(),
        "Unknown visits outside thirty minutes do not affect coverage"
    );
    db.call(|c| {
        c.execute("DELETE FROM session_acquisition WHERE session_id IN(SELECT id FROM sessions WHERE last_at=?1)", [NOW-1200])?;
        Ok(())
    }).await.unwrap();
    let r = report(&db, 1, q.clone()).await;
    assert!(
        r.acquisition_incomplete.contains("utm_source"),
        "An unknown older visit affects the thirty-minute headline even when every current visit is attributed"
    );
    assert_eq!(r.pages.len(), 2);
    let r = report(
        &db,
        1,
        ReportQuery {
            page: Some("/recent".into()),
            ..q
        },
    )
    .await;
    assert!(
        r.acquisition_incomplete.is_empty(),
        "Other audience filters still restrict the coverage check"
    );
}
