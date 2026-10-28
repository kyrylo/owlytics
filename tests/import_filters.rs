use owlytics::{
    analytics::{self, Metric, Report, ReportQuery, ReportScope},
    auth,
    db::Database,
    filters::{Filter, Operator},
    migration,
};
use std::path::PathBuf;

struct Export(PathBuf);
impl Export {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "owlytics-import-filters-{}",
            auth::random_token().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, table: &str, contents: &str) {
        std::fs::write(
            self.0
                .join(format!("imported_{table}_20261025_20261026.csv")),
            contents,
        )
        .unwrap();
    }
}
impl Drop for Export {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn fixture() -> Database {
    let export = Export::new();
    export.write("visitors","date,visitors,pageviews,visits,bounces,visit_duration\n2026-10-25,10,20,12,3,120\n2026-10-26,4,8,4,2,40\n");
    export.write("sources","date,source,referrer,utm_source,utm_medium,utm_campaign,utm_content,utm_term,visitors,pageviews,visits,bounces,visit_duration\n2026-10-25,Google,google.com/x,google,cpc,A%_🦉,,,3,8,4,1,40\n2026-10-25,Google,google.com/y,google,organic,B,,,2,4,2,0,30\n2026-10-25,Facebook,facebook.com,fb,cpc,A%_🦉,,,2,4,2,1,10\n2026-10-25,Direct / None,,,,,,,3,4,4,1,40\n2026-10-26,Google,google.com/x,google,cpc,A%_🦉,,,4,8,4,2,40\n");
    export.write("locations","date,country,region,city,visitors,pageviews,visits,bounces,visit_duration\n2026-10-25,US,US-CA,101,3,8,4,1,40\n2026-10-25,US,US-NY,102,2,4,2,0,30\n2026-10-25,DE,DE-BE,201,5,8,6,2,50\n2026-10-26,US,US-CA,101,4,8,4,2,40\n");
    export.write("browsers","date,browser,browser_version,visitors,pageviews,visits,bounces,visit_duration\n2026-10-25,Chrome,1,3,8,4,1,40\n2026-10-25,Firefox,1,2,4,2,0,30\n2026-10-25,Chrome,2,5,8,6,2,50\n2026-10-26,Chrome,1,4,8,4,2,40\n");
    export.write("operating_systems","date,operating_system,operating_system_version,visitors,pageviews,visits,bounces,visit_duration\n2026-10-25,macOS,14,3,8,4,1,40\n2026-10-25,Android,14,2,4,2,0,30\n2026-10-25,macOS,15,5,8,6,2,50\n2026-10-26,macOS,14,4,8,4,2,40\n");
    export.write("pages","date,hostname,page,visitors,pageviews,visits,total_time_on_page,total_time_on_page_visits,total_scroll_depth,total_scroll_depth_visits\n2026-10-25,example.com,/a,3,8,4,80,2,120,2\n2026-10-25,example.com,/b,7,12,8,90,3,150,3\n2026-10-26,example.com,/a,4,8,4,40,2,100,2\n");
    export.write("entry_pages","date,entry_page,visitors,pageviews,entrances,bounces,visit_duration\n2026-10-25,/a,2,5,3,1,20\n2026-10-25,/b,8,15,9,2,100\n2026-10-26,/a,4,8,4,2,40\n");
    export.write("exit_pages","date,exit_page,visitors,pageviews,exits,bounces,visit_duration\n2026-10-25,/a,2,5,2,1,20\n2026-10-25,/b,8,15,10,2,100\n2026-10-26,/a,4,8,4,2,40\n");
    export.write("devices","date,device,visitors,pageviews,visits,bounces,visit_duration\n2026-10-25,Desktop,5,12,6,1,70\n2026-10-25,Mobile,5,8,6,2,50\n2026-10-26,Desktop,4,8,4,2,40\n");
    let db = Database::open(":memory:").unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let path = export.0.clone();
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
    db.call(|c| {
        for (i,path) in ["/a","/b"].iter().enumerate() {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,region,city,browser,browser_version,os,os_version,device) VALUES(1,?1,'2026-10-27',zeroblob(16),?2,?3,'Google','google.com/x','US','US: US-CA','US: 101','Chrome','1','macOS','14','Desktop')",rusqlite::params![1793059200+i as i64*20,vec![i as u8;16],path])?;
        }
        c.execute("INSERT INTO session_acquisition(session_id,channel,utm_source,utm_medium,utm_campaign,utm_content,utm_term) SELECT id,'Paid Search','google','cpc','A%_🦉','','' FROM sessions",[])?;
        Ok(())
    }).await.unwrap();
    db
}
fn filter(key: &str, op: Operator, values: &[&str]) -> Filter {
    Filter {
        key: key.into(),
        op,
        values: values.iter().map(|v| (*v).into()).collect(),
    }
}
fn query(filters: Vec<Filter>) -> ReportQuery {
    ReportQuery {
        from: Some("2026-10-25".into()),
        to: Some("2026-10-27".into()),
        interval: Some("day".into()),
        filters,
        ..Default::default()
    }
}
async fn report(db: &Database, q: ReportQuery) -> Report {
    analytics::report(db, 1, q.range("2026-10-28".parse().unwrap()).unwrap(), q)
        .await
        .unwrap()
}

#[tokio::test]
async fn compatible_history_is_included_per_report_without_inventing_intersections() {
    let db = fixture().await;
    let q = query(vec![filter("source", Operator::Is, &["Google"])]);
    let r = report(&db, q.clone()).await;
    assert_eq!(
        (
            r.visitors,
            r.pageviews,
            r.totals.visits,
            r.totals.bounces,
            r.totals.duration
        ),
        (10, 22, 11, 3, 130)
    );
    assert!(r.has_imports);
    assert!(!r.imports_excluded);
    assert_eq!((r.sources[0].visitors, r.sources[0].pageviews), (10, 22));
    assert_eq!(r.referrers.iter().map(|r| r.pageviews).sum::<i64>(), 22);
    assert_eq!(r.pages.iter().map(|r| r.pageviews).sum::<i64>(), 2);
    assert!(r.pages.iter().all(|r| r.percentage == Some((1, 1))));
    for key in ["page", "country", "browser", "os", "device"] {
        assert!(r.imports_excluded_dimensions.contains(key), "{key}");
    }
    assert!(!r.imports_excluded_dimensions.contains("referrer"));
    for scope in [ReportScope::Timeline, ReportScope::Dashboard] {
        let other = report(&db, ReportQuery { scope, ..q.clone() }).await;
        assert_eq!(other.totals, r.totals);
        assert_eq!(other.daily, r.daily);
    }
    let r = report(
        &db,
        query(vec![
            filter("source", Operator::Is, &["Google"]),
            filter("utm_medium", Operator::Is, &["cpc"]),
        ]),
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (8, 18));
    assert_eq!(
        r.acquisition_rows("utm_campaign")[0].percentage,
        Some((8, 8))
    );
    assert_eq!(r.sources.len(), 1);
    let r = report(
        &db,
        query(vec![
            filter("source", Operator::Is, &["Google"]),
            filter("country", Operator::Is, &["US"]),
        ]),
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (1, 2));
    assert!(!r.has_imports);
    assert!(r.imports_excluded);
}

#[tokio::test]
async fn location_and_version_filters_keep_the_original_exported_relationships() {
    let db = fixture().await;
    for filters in [
        vec![
            filter("country", Operator::Is, &["US"]),
            filter("region", Operator::Is, &["US: US-CA"]),
        ],
        vec![
            filter("browser", Operator::Is, &["Chrome"]),
            filter("browser_version", Operator::Is, &["1"]),
        ],
        vec![
            filter("os", Operator::Is, &["macOS"]),
            filter("os_version", Operator::Is, &["14"]),
        ],
    ] {
        let r = report(&db, query(filters)).await;
        assert_eq!((r.visitors, r.pageviews), (8, 18));
        assert!(!r.imports_excluded);
    }
    let r = report(
        &db,
        query(vec![filter("browser_version", Operator::Is, &["1"])]),
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (10, 22));
    assert_eq!(
        r.browsers
            .iter()
            .map(|r| (r.name.as_str(), r.pageviews))
            .collect::<Vec<_>>(),
        vec![("Chrome", 18), ("Firefox", 4)]
    );
    let mut q = query(vec![filter("country", Operator::Is, &["US"])]);
    q.scope = ReportScope::Suggestions;
    q.filter = Some("region".into());
    let r = report(&db, q).await;
    assert_eq!(
        r.regions
            .iter()
            .map(|r| (r.name.as_str(), r.pageviews))
            .collect::<Vec<_>>(),
        vec![("US: US-CA", 18), ("US: US-NY", 4)]
    );
}

#[tokio::test]
async fn negative_multi_value_literal_and_zero_match_filters_share_native_semantics() {
    let db = fixture().await;
    for (filters, visitors, views) in [
        (vec![filter("source", Operator::IsNot, &["Google"])], 5, 8),
        (
            vec![filter("utm_campaign", Operator::Contains, &["%_"])],
            10,
            22,
        ),
        (
            vec![filter("utm_campaign", Operator::ContainsNot, &["%_"])],
            5,
            8,
        ),
        (
            vec![filter("source", Operator::Is, &["Google", "Facebook"])],
            12,
            26,
        ),
        (vec![filter("utm_source", Operator::Is, &["GOOGLE"])], 0, 0),
        (vec![filter("utm_medium", Operator::Is, &[""])], 3, 4),
        (vec![filter("source", Operator::Is, &["' OR 1=1 --"])], 0, 0),
    ] {
        let r = report(&db, query(filters)).await;
        assert_eq!((r.visitors, r.pageviews), (visitors, views));
        assert!(r.has_imports);
        assert!(!r.imports_excluded);
    }
    let r = report(
        &db,
        ReportQuery {
            window: Some((1793059200, 1793145600)),
            ..query(vec![filter("source", Operator::Is, &["Google"])])
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (1, 2));
    assert!(r.imports_excluded);
}

#[tokio::test]
async fn page_filters_preserve_engagement_and_use_entrances_for_bounce_rates() {
    let db = fixture().await;
    let r = report(&db, query(vec![filter("page", Operator::Is, &["/a"])])).await;
    assert_eq!(
        (
            r.visitors,
            r.pageviews,
            r.totals.visits,
            r.totals.entrances,
            r.totals.bounces
        ),
        (8, 17, 9, Some(8), 3)
    );
    assert_eq!(Metric::BounceRate.value(&r.totals), 37.5);
    assert!(r.totals.available(Metric::BounceRate));
    assert_eq!(Metric::TimeOnPage.value(&r.totals), 30.);
    assert_eq!(Metric::ScrollDepth.value(&r.totals), 55.);
    assert!(r.imports_excluded_dimensions.contains("entry"));
    assert_eq!(r.pages[0].pageviews, 17);
    let r = report(
        &db,
        query(vec![filter("device", Operator::Is, &["Desktop"])]),
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (10, 22));
    let r = report(&db, query(vec![filter("entry", Operator::Is, &["/a"])])).await;
    assert_eq!((r.visitors, r.pageviews, r.totals.visits), (7, 15, 8));
    let r = report(&db, query(vec![filter("exit", Operator::Is, &["/a"])])).await;
    assert_eq!((r.visitors, r.pageviews), (6, 13));
    assert_eq!(
        r.exit_pages[0].unavailable & 16,
        16,
        "Independent page totals cannot supply a filtered exit rate"
    );
}

#[tokio::test]
async fn legacy_marginals_survive_without_reconstructing_lost_versions_or_locations() {
    let db = fixture().await;
    db.call(|c| c.execute_batch("DELETE FROM import_groups; DELETE FROM import_files;"))
        .await
        .unwrap();
    let r = report(
        &db,
        query(vec![filter("browser", Operator::Is, &["Chrome"])]),
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (13, 26));
    assert!(!r.imports_excluded);
    assert!(r.imports_excluded_dimensions.contains("browser_version"));
    assert_eq!(r.browser_versions[0].visitors, 1);
    let r = report(
        &db,
        query(vec![
            filter("browser", Operator::Is, &["Chrome"]),
            filter("browser_version", Operator::Is, &["1"]),
        ]),
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (1, 2));
    assert!(r.imports_excluded);
}

#[tokio::test]
async fn partial_legacy_history_has_panel_specific_coverage_and_denominators() {
    let db = fixture().await;
    db.call(|c|c.execute_batch("INSERT INTO import_batches VALUES(2,1,'2026-10-24','2026-10-24',0);INSERT INTO import_daily VALUES(1,'2026-10-24',2,30,60);INSERT INTO import_breakdowns VALUES(1,'2026-10-24','browser','Chrome',2,60);INSERT INTO import_metrics VALUES(1,'2026-10-24','browser','Chrome',2,30,30,10,300,NULL);")).await.unwrap();
    let q = ReportQuery {
        from: Some("2026-10-24".into()),
        ..query(vec![filter("browser", Operator::Is, &["Chrome"])])
    };
    let r = report(&db, q.clone()).await;
    assert_eq!((r.visitors, r.pageviews), (43, 86));
    assert!(!r.imports_excluded);
    assert!(r.imports_excluded_dimensions.contains("browser_version"));
    let one = r.browser_versions.iter().find(|r| r.name == "1").unwrap();
    assert_eq!(
        one.percentage,
        Some((8, 13)),
        "Excluded legacy visitors must not dilute known version percentages"
    );
    let r = report(
        &db,
        ReportQuery {
            filters: vec![
                filter("browser", Operator::Is, &["Chrome"]),
                filter("browser_version", Operator::Is, &["1"]),
            ],
            ..q
        },
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (8, 18));
    assert!(r.has_imports);
    assert!(r.imports_excluded);
}

#[tokio::test]
async fn missing_columns_and_counts_remain_unknown_and_group_imports_are_atomic() {
    let export = Export::new();
    export.write("visitors", "date,visitors,pageviews\n2026-10-25,3,5\n");
    export.write("browsers", "date,browser,pageviews\n2026-10-25,Chrome,5\n");
    export.write("locations", "date,country,pageviews\n2026-10-25,US,5\n");
    let db = Database::open(":memory:").unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let path = export.0.clone();
    db.call(move |c| {
        Ok(migration::preview(
            c,
            "example.com",
            &path,
            "2026-10-27".parse().unwrap(),
        ))
    })
    .await
    .unwrap()
    .unwrap();
    db.read(|c| {
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM import_groups", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    })
    .await
    .unwrap();
    export.write(
        "operating_systems",
        "date,operating_system,visitors,pageviews\n2026-10-25,bad\0field,3,5\n",
    );
    let path = export.0.clone();
    assert!(
        db.call(move |c| Ok(migration::import(
            c,
            "example.com",
            &path,
            "2026-10-27".parse().unwrap()
        )))
        .await
        .unwrap()
        .is_err()
    );
    db.read(|c| {
        assert_eq!(
            c.query_row(
                "SELECT (SELECT COUNT(*) FROM import_groups)+(SELECT COUNT(*) FROM import_batches)",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        Ok(())
    })
    .await
    .unwrap();
    std::fs::remove_file(
        export
            .0
            .join("imported_operating_systems_20261025_20261026.csv"),
    )
    .unwrap();
    let path = export.0.clone();
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
    let r = report(
        &db,
        query(vec![filter("browser", Operator::Is, &["Chrome"])]),
    )
    .await;
    assert_eq!(r.pageviews, 5);
    assert!(!r.totals.available(Metric::Visitors));
    assert!(!r.totals.available(Metric::BounceRate));
    assert!(r.breakdown_pageviews);
    assert!(r.browser_versions.is_empty());
    assert!(r.imports_excluded_dimensions.contains("browser_version"));
    let r = report(
        &db,
        query(vec![
            filter("country", Operator::Is, &["US"]),
            filter("region", Operator::Is, &[""]),
        ]),
    )
    .await;
    assert_eq!(r.pageviews, 0);
    assert!(r.imports_excluded);
    db.call(|c| {
        c.execute("DELETE FROM sites WHERE id=1", [])?;
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM import_groups", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn complete_empty_exports_and_pages_without_entrances_are_known_zeroes() {
    let db = fixture().await;
    db.call(|c| {
        c.execute(
            "DELETE FROM import_metrics WHERE dimension='entry' AND value='/a'",
            [],
        )
    })
    .await
    .unwrap();
    let r = report(&db, query(vec![filter("page", Operator::Is, &["/a"])])).await;
    assert_eq!(r.totals.entrances, Some(1)); // Only the native visit entered here.
    assert_eq!(r.totals.bounces, 0);
    assert!(r.totals.available(Metric::BounceRate));
    let export = Export::new();
    export.write("visitors", "date,visitors,pageviews\n2026-10-25,1,2\n");
    export.write(
        "browsers",
        "date,browser,browser_version,visitors,pageviews,visits,bounces,visit_duration\n",
    );
    let empty = Database::open(":memory:").unwrap();
    empty
        .call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let path = export.0.clone();
    empty
        .call(move |c| {
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
    let r = report(
        &empty,
        query(vec![
            filter("browser", Operator::Is, &["Chrome"]),
            filter("browser_version", Operator::Is, &["1"]),
        ]),
    )
    .await;
    assert_eq!((r.visitors, r.pageviews), (0, 0));
    assert!(r.has_imports);
    assert!(!r.imports_excluded);
}

#[tokio::test]
async fn import_visibility_applies_to_all_report_scopes_without_changing_saved_history() {
    let db = fixture().await;
    for filters in [
        vec![],
        vec![filter("source", Operator::Is, &["Google"])],
        vec![filter("browser", Operator::Is, &["Chrome"])],
        vec![
            filter("browser", Operator::Is, &["Chrome"]),
            filter("country", Operator::Is, &["US"]),
        ],
    ] {
        for scope in [
            ReportScope::All,
            ReportScope::Dashboard,
            ReportScope::Timeline,
            ReportScope::Suggestions,
        ] {
            let hidden = report(
                &db,
                ReportQuery {
                    with_imported: Some(false),
                    scope,
                    ..query(filters.clone())
                },
            )
            .await;
            assert_eq!((hidden.visitors, hidden.pageviews), (1, 2));
            assert!(hidden.imports_available && hidden.imports_disabled);
            assert!(!hidden.has_imports && !hidden.imports_excluded);
            assert!(hidden.imports_excluded_dimensions.is_empty());
            assert!(hidden.daily.iter().all(|row| row.name == "2026-10-27"));
            assert!(
                hidden
                    .sources
                    .iter()
                    .all(|row| row.name == "Google" && row.visitors == 1)
            );
            assert!(hidden.countries.iter().all(|row| row.name == "US"));
        }
    }
    let native = report(
        &db,
        ReportQuery {
            with_imported: Some(false),
            from: Some("2026-10-27".into()),
            to: Some("2026-10-27".into()),
            interval: Some("hour".into()),
            ..Default::default()
        },
    )
    .await;
    assert!(native.imports_available && !native.has_imports);
    assert_eq!(native.timeline.len(), 24);
    for setting in [None, Some(true)] {
        let included = report(
            &db,
            ReportQuery {
                with_imported: setting,
                ..query(vec![])
            },
        )
        .await;
        assert_eq!((included.visitors, included.pageviews), (15, 30));
        assert!(included.has_imports && !included.imports_disabled);
    }
    let counts = db
        .read(|c| {
            c.query_row(
                "SELECT COUNT(*),SUM(pageviews) FROM import_daily",
                [],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
        })
        .await
        .unwrap();
    assert_eq!(counts, (2, 28));
}
