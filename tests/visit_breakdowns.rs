use owlytics::{
    analytics::{self, Report, ReportQuery, ReportScope, Row},
    db::Database,
};

const START: i64 = 1793059200;

async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute_batch("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0)")?;
        for (site, visitor, source, country, browser, start, pages) in [
            (1, 1_u128, "Google", "US", "Chrome", 0, vec![(0,"/entry"),(300,"/finish")]),
            (1, 2, "GitHub", "US", "Chrome", 150, vec![(0,"/seen")]),
            (1, 3, "Google", "PH", "Safari", 50, vec![(0,"/entry2"),(110,"/seen"),(200,"/finish")]),
            (1, 1, "Outside", "DE", "Firefox", 4000, vec![(0,"/seen")]),
            (2, 4, "Other site", "DE", "Firefox", 150, vec![(0,"/seen")]),
        ] {
            for (offset, path) in pages {
                c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,region,city,browser,browser_version,os,os_version,device) VALUES(?1,?2,'2026-10-27',?3,randomblob(16),?4,?5,?5,?6,?6||': region',?6||': city',?7,'1','Linux','6','Desktop')",
                    rusqlite::params![site, START+start+offset, visitor.to_be_bytes().as_slice(), path, source, country, browser])?;
            }
        }
        c.execute("INSERT INTO session_acquisition SELECT id,CASE WHEN source='Google' THEN 'Organic Search' ELSE 'Referral' END,source,CASE WHEN source='Google' THEN 'organic' ELSE 'referral' END,CASE WHEN source='Google' THEN 'seed' ELSE '' END,'','',NULL FROM sessions",[])?;
        Ok(())
    }).await.unwrap();
    db
}

fn query(start: i64, end: i64) -> ReportQuery {
    ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        window: Some((START + start, START + end)),
        interval: Some("day".into()),
        ..Default::default()
    }
}

async fn report(db: &Database, q: ReportQuery) -> Report {
    let range = q.range("2026-10-27".parse().unwrap()).unwrap();
    analytics::report(db, 1, range, q).await.unwrap()
}

fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
    rows.iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("missing {name}: {rows:?}"))
}

#[tokio::test]
async fn visit_tables_include_overlapping_visits_independently_of_event_headlines() {
    let db = fixture().await;
    for scope in [ReportScope::All, ReportScope::Dashboard] {
        let r = report(
            &db,
            ReportQuery {
                scope,
                ..query(100, 200)
            },
        )
        .await;
        assert_eq!((r.visitors, r.pageviews, r.totals.visits), (2, 2, 2));
        assert_eq!(
            (
                r.pages.len(),
                r.pages[0].name.as_str(),
                r.pages[0].pageviews
            ),
            (1, "/seen", 2)
        );
        let google = row(&r.sources, "Google");
        assert_eq!(
            (
                google.visitors,
                google.pageviews,
                google.visits,
                google.duration
            ),
            (2, 5, 2, 500)
        );
        assert_eq!(google.percentage, Some((2, 2)));
        assert_eq!(row(&r.countries, "US").percentage, Some((2, 2)));
        assert_eq!(row(&r.browsers, "Chrome").percentage, Some((2, 2)));
        assert!(
            r.sources
                .iter()
                .all(|row| !["Outside", "Other site"].contains(&row.name.as_str()))
        );
    }
    let repeated = report(&db, query(100, 4500)).await;
    assert_eq!(row(&repeated.sources, "Outside").percentage, Some((1, 3)));
    assert_eq!(row(&repeated.sources, "Google").percentage, Some((2, 3)));
    assert_eq!(repeated.devices[0].percentage, Some((3, 3)));
    let r = report(&db, query(30, 40)).await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (0, 0, 0));
    assert!(r.pages.is_empty());
    for rows in [
        &r.sources,
        &r.referrers,
        &r.countries,
        &r.regions,
        &r.cities,
        &r.browsers,
        &r.browser_versions,
        &r.operating_systems,
        &r.os_versions,
        &r.devices,
        &r.entry_pages,
    ] {
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].visitors, rows[0].pageviews, rows[0].visits),
            (1, 2, 1)
        );
        assert_eq!(rows[0].percentage, None);
    }
    assert_eq!(r.exit_pages[0].percentage, None);
    assert_eq!(
        r.exit_pages[0].pageviews, 0,
        "Exit rate still uses matching pageviews, not complete-visit views"
    );
    assert_eq!(r.acquisition["utm_campaign"][0].name, "seed");
    assert_eq!(r.acquisition["utm_campaign"][0].percentage, None);
    assert_eq!(r.acquisition["utm_campaign"][0].pageviews, 2);
}

#[tokio::test]
async fn page_filters_constrain_visit_tables_to_matching_in_window_events() {
    let db = fixture().await;
    let r = report(
        &db,
        ReportQuery {
            page: Some("/seen".into()),
            ..query(100, 200)
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (2, 2, 2));
    let google = row(&r.sources, "Google");
    assert_eq!(
        (
            google.visitors,
            google.pageviews,
            google.visits,
            google.duration
        ),
        (1, 3, 1, 200)
    );
    assert_eq!(google.percentage, Some((1, 2)));
    assert_eq!(row(&r.browsers, "Chrome").percentage, Some((1, 2)));
    assert_eq!(r.acquisition["utm_campaign"][0].percentage, Some((1, 1)));
    let empty = report(
        &db,
        ReportQuery {
            page: Some("/finish".into()),
            ..query(100, 200)
        },
    )
    .await;
    assert!(empty.sources.is_empty() && empty.countries.is_empty() && empty.entry_pages.is_empty());
    let filtered = report(
        &db,
        ReportQuery {
            source: Some("Google".into()),
            ..query(100, 200)
        },
    )
    .await;
    assert_eq!((filtered.visitors, filtered.pageviews), (1, 1));
    assert_eq!(row(&filtered.sources, "Google").percentage, Some((2, 1)));
    assert_eq!(row(&filtered.browsers, "Chrome").percentage, Some((1, 1)));
    assert_eq!(row(&filtered.browsers, "Safari").percentage, Some((1, 1)));
    assert_eq!(
        filtered.acquisition["utm_campaign"][0].percentage,
        Some((2, 1))
    );
}

#[tokio::test]
async fn missing_campaign_attribution_uses_the_overlapping_visit_audience() {
    let db = fixture().await;
    db.call(|c| c.execute("DELETE FROM session_acquisition WHERE session_id=(SELECT id FROM sessions WHERE site_id=1 AND visitor=?1 AND source='Google')",[1_u128.to_be_bytes().as_slice()])).await.unwrap();
    let r = report(
        &db,
        ReportQuery {
            filters: vec![owlytics::filters::Filter {
                key: "utm_campaign".into(),
                op: Default::default(),
                values: vec!["seed".into()],
            }],
            ..query(100, 200)
        },
    )
    .await;
    assert!(r.acquisition_incomplete.contains("utm_campaign"));
    assert_eq!(r.acquisition["utm_campaign"][0].percentage, Some((1, 1)));
    let page = report(
        &db,
        ReportQuery {
            filters: vec![owlytics::filters::Filter {
                key: "utm_campaign".into(),
                op: Default::default(),
                values: vec!["seed".into()],
            }],
            page: Some("/seen".into()),
            ..query(100, 200)
        },
    )
    .await;
    assert!(!page.acquisition_incomplete.contains("utm_campaign"));
    let outside = report(
        &db,
        ReportQuery {
            filters: vec![owlytics::filters::Filter {
                key: "utm_campaign".into(),
                op: Default::default(),
                values: vec!["seed".into()],
            }],
            ..query(400, 500)
        },
    )
    .await;
    assert!(!outside.acquisition_incomplete.contains("utm_campaign"));
}

#[tokio::test]
async fn comparison_names_do_not_reduce_the_percentage_audience() {
    let db = fixture().await;
    for (start, end, expected) in [(100, 200, Some((2, 2))), (30, 40, None)] {
        let r = report(
            &db,
            ReportQuery {
                comparison_names: Some(std::collections::BTreeMap::from([
                    ("source".into(), vec!["Google".into()]),
                    ("utm_campaign".into(), vec!["seed".into()]),
                ])),
                ..query(start, end)
            },
        )
        .await;
        assert_eq!(r.sources.len(), 1);
        assert_eq!(r.sources[0].percentage, expected);
        assert_eq!(
            r.acquisition["utm_campaign"][0].percentage,
            expected.map(|(count, _)| (count, 1))
        );
    }
}
