use chrono::NaiveDate;
use owlytics::{
    analytics::{self, ReportQuery},
    auth,
    db::Database,
    migration,
};
use std::path::PathBuf;

struct Export(PathBuf);
impl Export {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("owlytics-export-{}", auth::random_token().unwrap()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, table: &str, contents: &str) {
        std::fs::write(
            self.0
                .join(format!("imported_{table}_20261025_20261027.csv")),
            contents,
        )
        .unwrap();
    }
    fn full(&self) {
        std::fs::write(
            self.0.join("._imported_visitors_20261025_20261027.csv"),
            [0, 5, 22, 7],
        )
        .unwrap();
        self.write("visitors", "date,visitors,pageviews,bounces,visits,visit_duration\n2026-10-25,5,10,0,5,100\n2026-10-26,4,7,0,4,50\n2026-10-27,40,99,0,40,1000\n");
        self.write("pages", "date,hostname,page,visits,visitors,pageviews\n2026-10-25,example.com,/one?private=discard,3,3,6\n2026-10-25,www.example.com,/two,2,2,4\n2026-10-26,example.com,/one,4,4,7\n");
        self.write("sources", "date,source,referrer,utm_source,utm_medium,utm_campaign,utm_content,utm_term,pageviews,visitors,visits,visit_duration,bounces\n2026-10-25,Search,search.example/private?q=secret,a,,,,,6,3,3,30,0\n2026-10-25,Search,search.example/other,b,,,,,4,2,2,20,0\n");
        self.write("locations", "date,country,region,city,visitors,visits,visit_duration,bounces,pageviews\n2026-10-25,PH,PH-00,1,3,3,30,0,6\n2026-10-25,PH,PH-01,2,2,2,20,0,4\n");
        self.write("browsers", "date,browser,browser_version,visitors,visits,visit_duration,bounces,pageviews\n2026-10-25,Safari,17,3,3,30,0,6\n2026-10-25,Safari,18,2,2,20,0,4\n2026-10-26,Chrome,130,4,4,50,0,7\n");
        self.write("devices", "date,device,visitors,visits,visit_duration,bounces,pageviews\n2026-10-25,Desktop,5,5,100,0,10\n");
        self.write("custom_events", "outside scope\n");
        self.write("custom_props", "outside scope\n");
        self.write("entry_pages", "date,entry_page,visitors,entrances,visit_duration,bounces,pageviews\n2026-10-25,/one,5,5,100,0,10\n");
        self.write("exit_pages", "date,exit_page,visitors,visit_duration,exits,bounces,pageviews\n2026-10-25,/two,5,100,5,0,10\n");
        self.write("operating_systems", "date,operating_system,operating_system_version,visitors,visits,visit_duration,bounces,pageviews\n2026-10-25,macOS,14.0,5,5,100,0,10\n");
    }
}
impl Drop for Export {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn date(value: &str) -> NaiveDate {
    value.parse().unwrap()
}
async fn db() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    db
}

#[tokio::test]
async fn imports_full_reports_and_merges_only_additive_metrics_without_double_counting() {
    let export = Export::new();
    export.full();
    let db = db().await;
    db.call(|c| c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,unixepoch('2026-10-27'),'2026-10-27',zeroblob(16),zeroblob(16),'/one','','PH','Safari','Mobile')", [])).await.unwrap();
    let path = export.0.clone();
    let result = db
        .call(move |c| {
            Ok(migration::import(
                c,
                "example.com",
                &path,
                date("2026-10-28"),
            ))
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!((result.visitors, result.pageviews), (9, 17));
    assert_eq!(
        (result.from, result.to),
        (date("2026-10-25"), date("2026-10-26"))
    );
    assert_eq!(result.skipped_rows, 1);
    assert_eq!(result.ignored, ["custom_events", "custom_props",]);
    assert!(result.missing.is_empty());
    let query = ReportQuery {
        from: Some("2026-10-25".into()),
        to: Some("2026-10-27".into()),
        ..Default::default()
    };
    let range = query.range(date("2026-10-27")).unwrap();
    let report = analytics::report(&db, 1, range.clone(), query.clone())
        .await
        .unwrap();
    assert_eq!((report.visitors, report.pageviews), (10, 18));
    assert_eq!(report.daily.len(), 3);
    assert!(!report.breakdown_pageviews);
    assert!(report.session_metrics_available);
    assert_eq!(
        (
            report.totals.visits,
            report.totals.bounces,
            report.totals.duration
        ),
        (10, 1, 150)
    );
    assert_eq!(report.operating_systems[0].name, "macOS");
    assert_eq!(report.entry_pages[0].name, "/one");
    assert_eq!(
        report
            .exit_pages
            .iter()
            .find(|r| r.name == "/two")
            .unwrap()
            .exits,
        5
    );
    assert_eq!(report.regions.len(), 2);
    assert!(report.regions.iter().all(|row| !row.name.is_empty()));
    assert_eq!(
        (report.pages[0].name.as_str(), report.pages[0].pageviews),
        ("/one", 14)
    );
    assert_eq!(
        (
            report.browsers[0].name.as_str(),
            report.browsers[0].pageviews
        ),
        ("Safari", 11)
    );
    assert_eq!(
        (
            report.countries[0].name.as_str(),
            report.countries[0].pageviews
        ),
        ("PH", 11)
    );
    assert_eq!(
        (report.sources[0].name.as_str(), report.sources[0].pageviews),
        ("Search", 10)
    );
    let filtered = analytics::report(
        &db,
        1,
        range,
        ReportQuery {
            page: Some("/one".into()),
            ..query
        },
    )
    .await
    .unwrap();
    assert_eq!((filtered.visitors, filtered.pageviews), (8, 14));
    assert!(!filtered.imports_excluded);
    assert!(filtered.imports_excluded_dimensions.contains("source"));
    assert!(!filtered.breakdown_pageviews);
    let path = export.0.clone();
    assert!(
        db.call(move |c| Ok(migration::import(
            c,
            "example.com",
            &path,
            date("2026-10-28")
        )))
        .await
        .unwrap()
        .is_err()
    );
    let count: i64 = db
        .read(|c| c.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r.get(0)))
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn preserves_exported_campaign_source_independently_of_sanitized_referrer() {
    let export = Export::new();
    export.write(
        "visitors",
        "date,visitors,pageviews,bounces,visits,visit_duration\n2026-10-25,4,9,2,4,70\n",
    );
    export.write("sources", "date,source,referrer,utm_source,utm_medium,utm_campaign,pageviews,visitors,visits,visit_duration,bounces\n2026-10-25,Autumn Launch,https://user:password@www.google.com/search?q=private#fragment,Autumn Launch,email,October,4,2,2,60,1\n2026-10-25,Autumn Launch,,Autumn Launch,email,October,3,1,1,10,0\n2026-10-25,Direct / None,,,,,2,1,1,0,1\n");
    let db = db().await;
    let path = export.0.clone();
    db.call(move |c| {
        Ok(migration::import(
            c,
            "example.com",
            &path,
            date("2026-10-28"),
        ))
    })
    .await
    .unwrap()
    .unwrap();
    let query = ReportQuery {
        from: Some("2026-10-25".into()),
        to: Some("2026-10-25".into()),
        ..Default::default()
    };
    let report = analytics::report(&db, 1, query.range(date("2026-10-28")).unwrap(), query)
        .await
        .unwrap();
    assert_eq!((report.visitors, report.pageviews), (4, 9));
    assert_eq!(report.sources.len(), 2);
    for (dimension, value) in [
        ("utm_source", "Autumn Launch"),
        ("utm_medium", "email"),
        ("utm_campaign", "October"),
    ] {
        let rows = report.acquisition_rows(dimension);
        assert_eq!(rows.len(), 1, "{dimension}: untagged rows are omitted");
        let row = &rows[0];
        assert_eq!(row.name, value);
        assert_eq!(
            (
                row.visitors,
                row.pageviews,
                row.visits,
                row.bounces,
                row.duration
            ),
            (3, 7, 3, 1, 70)
        );
        assert_eq!(row.percentage, Some((3, 3)));
        assert!(!report.acquisition_incomplete.contains(dimension));
    }
    let email = report
        .acquisition_rows("channel")
        .iter()
        .find(|r| r.name == "Email")
        .unwrap();
    assert_eq!(email.percentage, Some((3, 4)));
    for dimension in ["utm_content", "utm_term"] {
        assert!(report.acquisition_rows(dimension).is_empty());
        assert!(report.acquisition_incomplete.contains(dimension));
    }
    let campaign = report
        .sources
        .iter()
        .find(|row| row.name == "Autumn Launch")
        .unwrap();
    assert_eq!(
        (
            campaign.visitors,
            campaign.pageviews,
            campaign.visits,
            campaign.bounces,
            campaign.duration
        ),
        (3, 7, 3, 1, 70)
    );
    assert_eq!(
        report
            .sources
            .iter()
            .find(|row| row.name.is_empty())
            .unwrap()
            .pageviews,
        2
    );
    let referrers: std::collections::BTreeMap<_, _> = report
        .referrers
        .iter()
        .map(|row| (row.name.as_str(), row.pageviews))
        .collect();
    assert_eq!(
        referrers,
        std::collections::BTreeMap::from([("", 5), ("google.com/search", 4)])
    );
    db.read(|c| {
        let rows=c.prepare("SELECT source,referrer,channel,utm_source,utm_medium,utm_campaign,pageviews,visitors,visits,bounces,duration FROM import_acquisition ORDER BY id")?
            .query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,i64>(6)?,r.get::<_,i64>(7)?,r.get::<_,i64>(8)?,r.get::<_,i64>(9)?,r.get::<_,i64>(10)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        assert_eq!(rows,vec![
            ("Autumn Launch".into(),"google.com/search".into(),"Email".into(),"Autumn Launch".into(),"email".into(),"October".into(),4,2,2,1,60),
            ("Autumn Launch".into(),"".into(),"Email".into(),"Autumn Launch".into(),"email".into(),"October".into(),3,1,1,0,10),
            ("".into(),"".into(),"Direct".into(),"".into(),"".into(),"".into(),2,1,1,1,0),
        ]);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM import_acquisition WHERE utm_content IS NULL AND utm_term IS NULL",[],|r|r.get::<_,i64>(0))?,3);
        assert_eq!(c.query_row("SELECT pageviews FROM import_breakdowns WHERE dimension='channel' AND value='Email'",[],|r|r.get::<_,i64>(0))?,7);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn campaign_import_distinguishes_missing_columns_and_rolls_back_previews_and_invalid_tags() {
    let export = Export::new();
    export.write("visitors", "date,visitors,pageviews\n2026-10-25,1,2\n");
    export.write(
        "sources",
        "date,source,referrer,pageviews\n2026-10-25,Google,google.com,2\n",
    );
    let db = db().await;
    let path = export.0.clone();
    db.call(move |c| {
        Ok(migration::preview(
            c,
            "example.com",
            &path,
            date("2026-10-28"),
        ))
    })
    .await
    .unwrap()
    .unwrap();
    db.read(|c| {
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM import_acquisition", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    })
    .await
    .unwrap();
    export.write("sources","date,source,referrer,utm_source,utm_medium,utm_campaign,pageviews\n2026-10-25,Google,google.com,google,cpc,good,1\n2026-10-25,Google,google.com,google,cpc,bad\0tag,1\n");
    let path = export.0.clone();
    assert!(
        db.call(move |c| Ok(migration::import(
            c,
            "example.com",
            &path,
            date("2026-10-28")
        )))
        .await
        .unwrap()
        .is_err()
    );
    db.read(|c| {
        for table in [
            "import_batches",
            "import_daily",
            "import_breakdowns",
            "import_metrics",
            "import_acquisition",
            "import_groups",
            "import_files",
        ] {
            assert_eq!(
                c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))?,
                0,
                "{table}"
            );
        }
        Ok(())
    })
    .await
    .unwrap();
    export.write(
        "sources",
        "date,source,referrer,pageviews\n2026-10-25,Google,google.com,2\n",
    );
    let path = export.0.clone();
    db.call(move |c| {
        Ok(migration::import(
            c,
            "example.com",
            &path,
            date("2026-10-28"),
        ))
    })
    .await
    .unwrap()
    .unwrap();
    db.call(|c| {
        assert_eq!(c.query_row("SELECT COUNT(*) FROM import_acquisition WHERE channel IS NULL AND utm_source IS NULL AND utm_medium IS NULL AND utm_campaign IS NULL AND utm_content IS NULL AND utm_term IS NULL AND visitors IS NULL AND visits IS NULL AND bounces IS NULL AND duration IS NULL",[],|r|r.get::<_,i64>(0))?,1);
        assert_eq!(c.query_row("SELECT COUNT(*) FROM import_breakdowns WHERE dimension='channel'",[],|r|r.get::<_,i64>(0))?,0);
        c.execute("DELETE FROM sites WHERE id=1",[])?;
        assert_eq!(c.query_row("SELECT COUNT(*) FROM import_acquisition",[],|r|r.get::<_,i64>(0))?,0);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn imports_ce_engagement_totals_with_weighted_denominators_and_missing_history() {
    for missing in [false, true] {
        let export = Export::new();
        export.full();
        let denominator = if missing { "" } else { "1" };
        export.write("pages", &format!("date,hostname,page,visitors,visits,pageviews,total_time_on_page,total_time_on_page_visits,total_scroll_depth,total_scroll_depth_visits\n2026-10-25,example.com,/one,5,5,10,100,2,150,2\n2026-10-26,example.com,/one,4,4,7,300,{denominator},40,{denominator}\n"));
        let db = db().await;
        let path = export.0.clone();
        db.call(move |c| {
            Ok(migration::import(
                c,
                "example.com",
                &path,
                date("2026-10-27"),
            ))
        })
        .await
        .unwrap()
        .unwrap();
        let query = ReportQuery {
            from: Some("2026-10-25".into()),
            to: Some("2026-10-26".into()),
            ..Default::default()
        };
        let r = analytics::report(&db, 1, query.range(date("2026-10-27")).unwrap(), query)
            .await
            .unwrap();
        let p = &r.pages[0];
        assert_eq!(p.time_on_page_ms, 400000); // CE exports seconds; native storage uses milliseconds.
        if missing {
            assert!(!p.available(analytics::Metric::TimeOnPage));
            assert!(!p.available(analytics::Metric::ScrollDepth));
        } else {
            assert_eq!(
                (
                    p.time_on_page_visits,
                    p.total_scroll_depth,
                    p.scroll_depth_visits
                ),
                (3, 190, 3)
            );
            assert!((analytics::Metric::TimeOnPage.value(p) - 400. / 3.).abs() < 1e-9);
            assert!((analytics::Metric::ScrollDepth.value(p) - 190. / 3.).abs() < 1e-9);
        }
        db.call(|c|c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200,'2026-10-27',zeroblob(16),zeroblob(16),'/one','','','Chrome','Desktop')",[])).await.unwrap();
        owlytics::engagement::collect(
            &db,
            owlytics::engagement::Event {
                domain: "example.com".into(),
                id: "00".repeat(16),
                time_ms: 20000,
                scroll_depth: 80,
            },
            "Mozilla/5.0 Chrome/130.0.0.0 Safari/537.36".into(),
            || 1793059220,
        )
        .await
        .unwrap();
        let q = ReportQuery {
            from: Some("2026-10-25".into()),
            to: Some("2026-10-27".into()),
            ..Default::default()
        };
        let mixed = analytics::report(&db, 1, q.range(date("2026-10-27")).unwrap(), q)
            .await
            .unwrap();
        let p = &mixed.pages[0];
        assert_eq!(p.time_on_page_ms, 420000);
        assert_eq!(p.available(analytics::Metric::TimeOnPage), !missing);
        assert_eq!(p.available(analytics::Metric::ScrollDepth), !missing);
        if !missing {
            assert_eq!(analytics::Metric::TimeOnPage.value(p), 105.);
            assert_eq!(analytics::Metric::ScrollDepth.value(p), 67.5);
        }
    }
}

#[tokio::test]
async fn invalid_engagement_import_rolls_back_every_table() {
    for row in ["100,0,100,1", "100,1,101,1", "-1,1,50,1", "100,1,50,-1"] {
        let export = Export::new();
        export.full();
        export.write("pages",&format!("date,hostname,page,pageviews,total_time_on_page,total_time_on_page_visits,total_scroll_depth,total_scroll_depth_visits\n2026-10-25,example.com,/one,10,{row}\n"));
        let db = db().await;
        let path = export.0.clone();
        assert!(
            db.call(move |c| Ok(migration::import(
                c,
                "example.com",
                &path,
                date("2026-10-27")
            )))
            .await
            .unwrap()
            .is_err()
        );
        db.read(|c| {
            for table in [
                "import_batches",
                "import_daily",
                "import_breakdowns",
                "import_metrics",
                "import_engagement",
            ] {
                assert_eq!(
                    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))?,
                    0
                );
            }
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn malformed_imports_roll_back_every_table() {
    for invalid in [
        "date,visitors,pageviews\n2026-10-25,5,10\n2026-10-26,-1,7\n",
        "date,visitors,pageviews\n2026-10-25,5,10\n2026-10-25,5,10\n",
        "date,visitors,pageviews\n2026-10-24,5,10\n",
        "date,visitors,pageviews\n2026-10-25,1,99999999999999999999\n",
        "date,visitors,pageviews\n",
        "date,pageviews\n2026-10-25,10\n",
        "date,date,visitors,pageviews\n2026-10-25,2026-10-25,5,10\n",
    ] {
        let export = Export::new();
        export.full();
        export.write("visitors", invalid);
        let db = db().await;
        let path = export.0.clone();
        assert!(
            db.call(move |c| Ok(migration::import(
                c,
                "example.com",
                &path,
                date("2026-10-27")
            )))
            .await
            .unwrap()
            .is_err(),
            "{invalid}"
        );
        let count: i64 = db.read(|c| c.query_row("SELECT (SELECT COUNT(*) FROM import_batches)+(SELECT COUNT(*) FROM import_daily)+(SELECT COUNT(*) FROM import_breakdowns)", [], |r| r.get(0))).await.unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn rejects_mismatched_sites_dates_and_breakdown_totals() {
    for pages in [
        "date,hostname,page,pageviews\n2026-10-25,wrong.example,/one,10\n",
        "date,hostname,page,pageviews\n2026-10-25,example.com,/one,11\n",
        "date,hostname,page,pageviews\n2026-10-25,example.com,//evil,10\n",
        "date,hostname,page,pageviews\n2026-10-25,example.com,,10\n",
        "date,hostname,page,pageviews\n2026-10-25,example.com,javascript:alert(1),10\n",
    ] {
        let export = Export::new();
        export.full();
        export.write("pages", pages);
        let db = db().await;
        let path = export.0.clone();
        assert!(
            db.call(move |c| Ok(migration::import(
                c,
                "example.com",
                &path,
                date("2026-10-27")
            )))
            .await
            .unwrap()
            .is_err()
        );
        let count: i64 = db
            .read(|c| c.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    let export = Export::new();
    export.full();
    std::fs::rename(
        export.0.join("imported_pages_20261025_20261027.csv"),
        export.0.join("imported_pages_20261024_20261027.csv"),
    )
    .unwrap();
    let db = db().await;
    let path = export.0.clone();
    assert!(
        db.call(move |c| Ok(migration::import(
            c,
            "example.com",
            &path,
            date("2026-10-27")
        )))
        .await
        .unwrap()
        .is_err()
    );
}

#[tokio::test]
async fn preview_validates_full_export_without_saving_and_matches_the_import() {
    let export = Export::new();
    export.full();
    let db = db().await;
    let directory = export.0.clone();
    let preview = db
        .call(move |c| {
            Ok(migration::preview(
                c,
                "example.com",
                &directory,
                date("2026-10-27"),
            ))
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (preview.visitors, preview.pageviews, preview.skipped_rows),
        (9, 17, 1)
    );
    assert!(preview.ignored.contains(&"custom_props".to_owned()));
    assert!(!preview.session_date_differences);
    let count=db.read(|c|c.query_row("SELECT (SELECT COUNT(*) FROM import_batches)+(SELECT COUNT(*) FROM import_daily)+(SELECT COUNT(*) FROM import_breakdowns)",[],|r|r.get::<_,i64>(0))).await.unwrap();
    assert_eq!(count, 0);
    export.write(
        "pages",
        "date,hostname,page,pageviews\n2026-10-25,example.com,/one,11\n",
    );
    let directory = export.0.clone();
    assert!(
        db.call(move |c| Ok(migration::preview(
            c,
            "example.com",
            &directory,
            date("2026-10-27")
        )))
        .await
        .unwrap()
        .is_err()
    );
    export.full();
    let directory = export.0.clone();
    let imported = db
        .call(move |c| {
            Ok(migration::import(
                c,
                "example.com",
                &directory,
                date("2026-10-27"),
            ))
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            imported.from,
            imported.to,
            imported.visitors,
            imported.pageviews
        ),
        (
            preview.from,
            preview.to,
            preview.visitors,
            preview.pageviews
        )
    );
    let count = db
        .read(|c| {
            c.query_row("SELECT COUNT(*) FROM import_batches", [], |r| {
                r.get::<_, i64>(0)
            })
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn session_dated_breakdowns_can_exceed_event_dated_totals_without_being_rewritten() {
    // CE exports headline views/pages from events, but these tables from sessions.
    // A session can attribute views to a different date at midnight.
    for (table, csv) in [
        (
            "sources",
            "date,source,referrer,pageviews\n2026-10-25,Search,search.example,11\n2026-10-26,Search,search.example,6\n",
        ),
        (
            "locations",
            "date,country,pageviews\n2026-10-25,PH,11\n2026-10-26,PH,6\n",
        ),
        (
            "browsers",
            "date,browser,pageviews\n2026-10-25,Safari,11\n2026-10-26,Safari,6\n",
        ),
        (
            "devices",
            "date,device,pageviews\n2026-10-25,Desktop,11\n2026-10-26,Desktop,6\n",
        ),
    ] {
        let export = Export::new();
        export.full();
        export.write(table, csv);
        // Extra current-CE columns are accepted without changing page counts.
        export.write("pages","\"pageviews\",\"date\",\"hostname\",\"page\",\"total_scroll_depth\",\"total_time_on_page\"\r\n10,2026-10-25,example.com,/one,500,1000\r\n7,2026-10-26,example.com,/one,300,800\r\n");
        let db = db().await;
        let directory = export.0.clone();
        let summary = db
            .call(move |c| {
                Ok(migration::import(
                    c,
                    "example.com",
                    &directory,
                    date("2026-10-27"),
                ))
            })
            .await
            .unwrap()
            .unwrap();
        assert!(summary.session_date_differences, "{table}");
        assert_eq!((summary.visitors, summary.pageviews), (9, 17));
        let query = ReportQuery {
            from: Some("2026-10-25".into()),
            to: Some("2026-10-25".into()),
            ..Default::default()
        };
        let report = analytics::report(&db, 1, query.range(date("2026-10-27")).unwrap(), query)
            .await
            .unwrap();
        assert_eq!(report.pageviews, 10);
        let rows = match table {
            "sources" => report.sources,
            "locations" => report.countries,
            "browsers" => report.browsers,
            "devices" => report.devices,
            _ => unreachable!(),
        };
        assert_eq!(rows.iter().map(|row| row.pageviews).sum::<i64>(), 11);
    }
}

#[tokio::test]
async fn imports_ce_empty_visit_pages_and_about_blank_without_inventing_paths() {
    let export = Export::new();
    export.write(
        "visitors",
        "date,visitors,pageviews,visits,bounces,visit_duration\n2026-10-25,2,1,2,1,0\n",
    );
    export.write(
        "pages",
        "date,hostname,page,visitors,visits,pageviews\n2026-10-25,,blank,1,1,1\n",
    );
    export.write("entry_pages", "date,entry_page,visitors,entrances,visit_duration,bounces,pageviews\n2026-10-25,,1,1,0,0,0\n2026-10-25,blank,1,1,0,1,1\n");
    export.write("exit_pages", "date,exit_page,visitors,exits,visit_duration,bounces,pageviews\n2026-10-25,,1,1,0,0,0\n2026-10-25,blank,1,1,0,1,1\n");
    let db = db().await;
    let path = export.0.clone();
    let summary = db
        .call(move |c| {
            Ok(migration::import(
                c,
                "example.com",
                &path,
                date("2026-10-28"),
            ))
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!((summary.visitors, summary.pageviews), (2, 1));
    db.read(|c| {
        let values: Vec<(String,String,i64,i64)> = c.prepare("SELECT dimension,value,m.visits,b.pageviews FROM import_metrics m JOIN import_breakdowns b USING(site_id,day,dimension,value) WHERE dimension IN ('page','entry','exit') ORDER BY dimension,value")?
            .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<Result<_,_>>()?;
        assert_eq!(values, vec![
            ("entry".into(), "".into(), 1, 0),
            ("entry".into(), "blank".into(), 1, 1),
            ("exit".into(), "".into(), 1, 0),
            ("exit".into(), "blank".into(), 1, 1),
            ("page".into(), "blank".into(), 1, 1),
        ]);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn extra_export_hosts_require_an_explicit_allowlist_in_preview_and_import() {
    let export = Export::new();
    export.full();
    export.write("pages", "date,hostname,page,pageviews\n2026-10-25,example-com.translate.goog,/one,10\n2026-10-26,example.com,/two,7\n");
    let db = db().await;
    let path = export.0.clone();
    db.call(move |c| {
        assert!(migration::preview(c, "example.com", &path, date("2026-10-27")).is_err());
        assert!(
            migration::import_with_hostnames(
                c,
                "example.com",
                &path,
                date("2026-10-27"),
                &["unrelated.example".into()],
                true
            )
            .is_err()
        );
        let allowed = vec!["example-com.translate.goog".into()];
        let preview = migration::import_with_hostnames(
            c,
            "example.com",
            &path,
            date("2026-10-27"),
            &allowed,
            true,
        )
        .unwrap();
        assert_eq!((preview.visitors, preview.pageviews), (9, 17));
        assert_eq!(
            c.query_row("SELECT COUNT(*) FROM import_batches", [], |r| r
                .get::<_, i64>(0))?,
            0
        );
        let imported = migration::import_with_hostnames(
            c,
            "example.com",
            &path,
            date("2026-10-27"),
            &allowed,
            false,
        )
        .unwrap();
        assert_eq!((imported.visitors, imported.pageviews), (9, 17));
        assert_eq!(
            c.query_row(
                "SELECT SUM(pageviews) FROM import_breakdowns WHERE dimension='page'",
                [],
                |r| r.get::<_, i64>(0)
            )?,
            17
        );
        Ok(())
    })
    .await
    .unwrap();
}
