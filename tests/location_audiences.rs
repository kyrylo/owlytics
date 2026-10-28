use owlytics::{
    analytics::{self, Report, ReportQuery, ReportScope, Row},
    auth,
    db::Database,
    migration,
};
use std::collections::BTreeMap;

async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        for (visitor, time, country, region, city) in [
            (1_u128, 0, "US", "US: US-CA", "US: 5391959"),
            (1, 3600, "PH", "PH: PH-00", "PH: 1701668"),
            (1, 7200, "", "", ""),
            (2, 10, "US", "", ""),
            (3, 20, "", "", ""),
            (4, 30, "ZZ", "", "0"),
            (5, 40, "AQ", "", ""),
            (6, 50, "\0\0", "", ""),
            (7, 60, "US", "US: Uncharted", "US: Uncharted"),
        ] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,region,city,browser,device) VALUES(1,1793059200+?1,'2026-10-27',?2,randomblob(16),'/','Google',?3,?4,?5,'Chrome','Desktop')",
                rusqlite::params![time,visitor.to_be_bytes().as_slice(),country,region,city])?;
        }
        Ok(())
    }).await.unwrap();
    db
}

fn query() -> ReportQuery {
    ReportQuery {
        from: Some("2026-10-26".into()),
        to: Some("2026-10-27".into()),
        interval: Some("day".into()),
        ..Default::default()
    }
}

async fn report(db: &Database, q: ReportQuery) -> Report {
    analytics::report(db, 1, q.range("2026-10-28".parse().unwrap()).unwrap(), q)
        .await
        .unwrap()
}

fn row<'a>(rows: &'a [Row], name: &str) -> &'a Row {
    rows.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("Missing {name}: {rows:?}"))
}

#[tokio::test]
async fn location_audiences_deduplicate_known_places_without_removing_headline_traffic() {
    let db = fixture().await;
    for q in [
        query(),
        ReportQuery {
            browser: Some("Chrome".into()),
            ..query()
        },
        ReportQuery {
            window: Some((1793059200, 1793145600)),
            ..query()
        },
    ] {
        let r = report(&db, q).await;
        assert_eq!((r.visitors, r.pageviews), (7, 9));
        assert_eq!(r.sources[0].percentage, Some((7, 7)));
        assert_eq!(r.countries.len(), 3);
        assert_eq!(row(&r.countries, "US").percentage, Some((3, 4)));
        assert_eq!(row(&r.countries, "PH").percentage, Some((1, 4)));
        assert_eq!(row(&r.countries, "AQ").percentage, Some((1, 4)));
        assert_eq!(r.regions.len(), 3);
        assert_eq!(r.cities.len(), 3);
        for rows in [&r.regions, &r.cities] {
            assert!(rows.iter().all(|r| r.percentage == Some((1, 2))));
            assert!(rows.iter().any(|r| r.name == "US: Uncharted"));
        }
    }
    // Search, ranking and comparison lookups must use the original audience.
    let detail = report(
        &db,
        ReportQuery {
            detail: Some("country".into()),
            search: Some("United".into()),
            sort: Some("percentage".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(detail.detail_total, 1);
    assert_eq!(detail.countries[0].percentage, Some((3, 4)));
    let compared = report(
        &db,
        ReportQuery {
            comparison_names: Some(BTreeMap::from([
                ("country".into(), vec!["US".into(), "".into(), "ZZ".into()]),
                (
                    "city".into(),
                    vec!["US: 5391959".into(), "".into(), "0".into()],
                ),
            ])),
            ..query()
        },
    )
    .await;
    assert_eq!(compared.countries.len(), 1);
    assert_eq!(compared.countries[0].percentage, Some((3, 4)));
    assert_eq!(compared.cities.len(), 1);
    assert_eq!(compared.cities[0].percentage, Some((1, 2)));
    // Unknown values remain available for explicit audience filters.
    let suggested = report(
        &db,
        ReportQuery {
            scope: ReportScope::Suggestions,
            filter: Some("country".into()),
            ..query()
        },
    )
    .await;
    assert!(suggested.countries.iter().any(|r| r.name.is_empty()));
    let unknown = report(
        &db,
        ReportQuery {
            country: Some("".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((unknown.visitors, unknown.pageviews), (2, 2));
    assert!(
        unknown.countries.is_empty() && unknown.regions.is_empty() && unknown.cities.is_empty()
    );
}

async fn import_history(db: &Database) {
    let folder = std::env::temp_dir().join(format!(
        "owlytics-location-audience-{}",
        auth::random_token().unwrap()
    ));
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(
        folder.join("imported_visitors_20261026_20261026.csv"),
        "date,visitors,pageviews,visits,bounces,visit_duration\n2026-10-26,15,30,15,0,300\n",
    )
    .unwrap();
    std::fs::write(folder.join("imported_locations_20261026_20261026.csv"), "date,country,region,city,visitors,pageviews,visits,bounces,visit_duration\n2026-10-26,US,US-CA,5391959,2,4,2,0,40\n2026-10-26,US,,0,3,6,3,0,60\n2026-10-26,AQ,,0,1,2,1,0,20\n2026-10-26,ZZ,,0,4,8,4,0,80\n2026-10-26,,,0,5,10,5,0,100\n").unwrap();
    let path = folder.clone();
    db.call(move |c| {
        Ok(migration::import(
            c,
            "example.com",
            &path,
            "2026-10-27".parse().unwrap(),
        ))
    })
    .await
    .unwrap()
    .unwrap();
    std::fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
async fn imported_location_cohorts_keep_missing_places_separate_from_missing_history() {
    let db = fixture().await;
    import_history(&db).await;
    let r = report(&db, query()).await;
    assert_eq!((r.visitors, r.pageviews), (22, 39));
    assert_eq!(r.countries.len(), 3);
    assert_eq!(row(&r.countries, "US").percentage, Some((8, 10)));
    assert_eq!(row(&r.regions, "US: US-CA").percentage, Some((3, 4)));
    assert_eq!(row(&r.cities, "US: 5391959").percentage, Some((3, 4)));
    let filtered = report(
        &db,
        ReportQuery {
            country: Some("US".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((filtered.visitors, filtered.pageviews), (8, 13));
    assert_eq!(row(&filtered.countries, "US").percentage, Some((8, 8)));
    assert_eq!(row(&filtered.regions, "US: US-CA").percentage, Some((3, 4)));
    assert_eq!(
        row(&filtered.cities, "US: 5391959").percentage,
        Some((3, 4))
    );
    let unknown = report(
        &db,
        ReportQuery {
            country: Some("".into()),
            ..query()
        },
    )
    .await;
    assert_eq!((unknown.visitors, unknown.pageviews), (7, 12));
    assert!(
        unknown.countries.is_empty() && unknown.regions.is_empty() && unknown.cities.is_empty()
    );
    assert!(!unknown.imports_excluded_dimensions.contains("city"));
    let incompatible = report(
        &db,
        ReportQuery {
            browser: Some("Chrome".into()),
            ..query()
        },
    )
    .await;
    assert!(incompatible.imports_excluded_dimensions.contains("country"));
    assert_eq!(row(&incompatible.countries, "US").percentage, Some((3, 4)));
    // Exclusions affect reports only, never the stored export or native facts.
    db.read(|c| {
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))?,
            9
        );
        assert_eq!(
            c.query_row(
                "SELECT SUM(pageviews) FROM import_breakdowns WHERE dimension='country'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            30
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn legacy_pageview_only_history_uses_known_location_pageviews() {
    let db = fixture().await;
    import_history(&db).await;
    db.call(|c| {
        c.execute(
            "UPDATE import_metrics SET visitors=NULL WHERE dimension='country'",
            [],
        )
    })
    .await
    .unwrap();
    let r = report(&db, query()).await;
    assert!(r.breakdown_pageviews);
    assert_eq!(row(&r.countries, "US").percentage, Some((13, 17)));
    assert_eq!((r.visitors, r.pageviews), (22, 39));
}
