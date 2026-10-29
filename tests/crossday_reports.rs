use owlytics::{
    analytics::{self, Metric, Report, ReportQuery, ReportScope},
    db::Database,
};

async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0),(2,'other.example','UTC',0)",[])?;
        for (person,stamp,path,source) in [
            (1_u128,1793059140,"/before","Google"),
            (1,1793059260,"/seen","Google"),
            (2,1793059320,"/seen","GitHub"),
            (1,1793149200,"/seen","Google"),
        ] {
            let day=chrono::DateTime::from_timestamp(stamp,0).unwrap().date_naive().to_string();
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,?3,randomblob(16),?4,?5,'US','Chrome','Desktop')",
                rusqlite::params![stamp,day,person.to_be_bytes().as_slice(),path,source])?;
        }
        Ok(())
    }).await.unwrap();
    db
}

fn query(from: &str, to: &str) -> ReportQuery {
    ReportQuery {
        from: Some(from.into()),
        to: Some(to.into()),
        interval: Some("day".into()),
        ..Default::default()
    }
}
async fn report(db: &Database, q: ReportQuery) -> Report {
    let range = q.range("2026-10-28".parse().unwrap()).unwrap();
    analytics::report(db, 1, range, q).await.unwrap()
}

#[tokio::test]
async fn whole_date_reports_do_not_add_the_same_visitor_across_dates() {
    let db = fixture().await;
    for q in [
        query("2026-10-26", "2026-10-28"),
        ReportQuery {
            browser: Some("Chrome".into()),
            scope: ReportScope::Timeline,
            ..query("2026-10-26", "2026-10-28")
        },
    ] {
        let r = report(&db, q).await;
        assert_eq!((r.visitors, r.pageviews, r.totals.visits), (2, 4, 3));
        assert_eq!(Metric::ViewsPerVisit.value(&r.totals), 4. / 3.);
    }
    let r = report(&db, query("2026-10-26", "2026-10-28")).await;
    let google = r.sources.iter().find(|r| r.name == "Google").unwrap();
    assert_eq!(
        (google.visitors, google.pageviews, google.visits),
        (1, 3, 2)
    );
    assert_eq!(google.percentage, Some((1, 2)));
    let seen = r.pages.iter().find(|r| r.name == "/seen").unwrap();
    assert_eq!((seen.visitors, seen.pageviews, seen.visits), (2, 3, 3));
}

#[tokio::test]
async fn whole_date_reports_include_complete_visits_begun_the_day_before() {
    let db = fixture().await;
    for q in [
        query("2026-10-27", "2026-10-27"),
        ReportQuery {
            browser: Some("Chrome".into()),
            scope: ReportScope::Timeline,
            ..query("2026-10-27", "2026-10-27")
        },
    ] {
        let r = report(&db, q).await;
        assert_eq!((r.visitors, r.pageviews, r.totals.visits), (2, 2, 2));
        assert_eq!(Metric::ViewsPerVisit.value(&r.totals), 1.5);
        assert_eq!(Metric::VisitDuration.value(&r.totals), 60.);
    }
}

#[tokio::test]
async fn calendar_buckets_deduplicate_visitors_and_attribute_visits_to_the_last_pageview() {
    let db = fixture().await;
    let r = report(&db, query("2026-10-26", "2026-10-28")).await;
    let days = r.calendar.as_ref().unwrap();
    assert_eq!(
        days.iter()
            .map(|r| (r.name.as_str(), r.visitors, r.visits, r.pageviews))
            .collect::<Vec<_>>(),
        [
            ("2026-10-26", 0, 0, 1),
            ("2026-10-27", 2, 2, 2),
            ("2026-10-28", 1, 1, 1)
        ]
    );
    // Daily downloads use mixed event/session metrics, whereas the graph
    // requests one metric and prefers visits for its visitor/visit counts.
    let exported = report(
        &db,
        ReportQuery {
            export: true,
            ..query("2026-10-26", "2026-10-28")
        },
    )
    .await;
    assert_eq!(
        exported
            .calendar
            .as_ref()
            .unwrap()
            .iter()
            .map(|r| r.visitors)
            .collect::<Vec<_>>(),
        [1, 2, 1]
    );
    for (interval, name) in [("week", "2026-10-26"), ("month", "2026-10-01")] {
        let r = report(
            &db,
            ReportQuery {
                interval: Some(interval.into()),
                ..query("2026-10-26", "2026-10-28")
            },
        )
        .await;
        let rows = r.calendar.as_ref().unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(
            (row.name.as_str(), row.visitors, row.visits, row.pageviews),
            (name, 2, 3, 4)
        );
        assert_eq!(Metric::ViewsPerVisit.value(row), 4. / 3.);
        assert_eq!(Metric::VisitDuration.value(row), 40.);
    }
}

#[tokio::test]
async fn calendar_page_filters_use_events_unless_entry_or_exit_requires_visits() {
    let db = fixture().await;
    for entry in [None, Some("/before".into())] {
        let r = report(
            &db,
            ReportQuery {
                page: Some("/before".into()),
                entry: entry.clone(),
                ..query("2026-10-26", "2026-10-26")
            },
        )
        .await;
        assert_eq!((r.visitors, r.pageviews), (1, 1));
        let first = &r.calendar.as_ref().unwrap()[0];
        // The only visit ends after this date. Event-based chart counts still
        // show its pageview; visit-based counts belong to the following date.
        assert_eq!(
            (first.visitors, first.visits),
            if entry.is_some() { (0, 0) } else { (1, 1) }
        );
    }
}

#[tokio::test]
async fn calendar_engagement_folds_a_page_visit_before_averaging() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        for (stamp,day,ms,depth) in [(1793059140,"2026-10-26",1000,20),(1793059260,"2026-10-27",2000,80)] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,zeroblob(16),randomblob(16),'/same','Google','US','Chrome','Desktop')",rusqlite::params![stamp,day])?;
            c.execute("INSERT INTO page_engagement VALUES(last_insert_rowid(),?1,?2)",rusqlite::params![ms,depth])?;
        }
        Ok(())
    }).await.unwrap();
    for (interval, scope) in [
        ("week", ReportScope::All),
        ("month", ReportScope::All),
        ("week", ReportScope::Timeline),
    ] {
        let r = report(
            &db,
            ReportQuery {
                page: Some("/same".into()),
                metric: Metric::TimeOnPage,
                interval: Some(interval.into()),
                scope,
                ..query("2026-10-26", "2026-10-27")
            },
        )
        .await;
        let row = &r.calendar.as_ref().unwrap()[0];
        assert_eq!(
            (
                row.time_on_page_ms,
                row.time_on_page_visits,
                row.total_scroll_depth,
                row.scroll_depth_visits
            ),
            (3000, 1, 80, 1)
        );
        assert_eq!(Metric::TimeOnPage.value(row), 3.);
        assert_eq!(Metric::ScrollDepth.value(row), 80.);
        assert_eq!(Metric::TimeOnPage.value(&r.totals), 3.);
        assert_eq!(Metric::ScrollDepth.value(&r.totals), 80.);
        assert_eq!(
            r.daily.iter().map(|r| r.time_on_page_visits).sum::<i64>(),
            2
        );
    }
}

#[tokio::test]
async fn calendar_history_adds_known_counters_after_native_deduplication() {
    let db = fixture().await;
    db.call(|c| {
        c.execute("INSERT INTO import_batches VALUES(1,1,'2026-10-26','2026-10-27',0)",[])?;
        for day in ["2026-10-26","2026-10-27"] {
            c.execute("INSERT INTO import_daily VALUES(1,?1,1,5,10)",[day])?;
            c.execute("INSERT INTO import_metrics(site_id,day,dimension,value,batch_id,visitors,visits,bounces,duration) VALUES(1,?1,'total','',1,5,6,2,60)",[day])?;
        }
        Ok(())
    }).await.unwrap();
    let r = report(
        &db,
        ReportQuery {
            interval: Some("week".into()),
            ..query("2026-10-26", "2026-10-28")
        },
    )
    .await;
    let row = &r.calendar.as_ref().unwrap()[0];
    assert_eq!((row.visitors, row.visits, row.pageviews), (12, 15, 24));
    assert_eq!(Metric::VisitDuration.value(row), 16.);
    assert_eq!(row.unavailable, 0);
    db.call(|c| {
        c.execute(
            "UPDATE import_metrics SET visits=NULL WHERE day='2026-10-27'",
            [],
        )
    })
    .await
    .unwrap();
    let r = report(
        &db,
        ReportQuery {
            interval: Some("week".into()),
            ..query("2026-10-26", "2026-10-28")
        },
    )
    .await;
    assert!(!r.calendar.as_ref().unwrap()[0].available(Metric::Visits));
}

#[tokio::test]
async fn hourly_page_counts_and_csv_counts_use_events_while_minute_counts_use_active_visits() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        for (person,stamp) in [(1_u128,1793059260),(2,1793059320),(2,1793061720)] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,'2026-10-27',?2,randomblob(16),'/target','Google','US','Chrome','Desktop')",rusqlite::params![stamp,person.to_be_bytes().as_slice()])?;
        }
        // Activity can span an hour without a pageview. Query fixtures model
        // that eligibility separately from the visit's measured endpoints.
        c.execute("UPDATE sessions SET active_at=1793066400 WHERE visitor=?1",[1_u128.to_be_bytes().as_slice()])?;
        c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793066460,'2026-10-27',?1,randomblob(16),'/target','Google','US','Chrome','Desktop')",[1_u128.to_be_bytes().as_slice()])?;
        Ok(())
    }).await.unwrap();
    for (page, entry, export, gap) in [
        (None, None, false, 1),
        (Some("/target"), None, false, 0),
        (None, None, true, 0),
        (Some("/target"), Some("/target"), false, 1),
    ] {
        let r = report(
            &db,
            ReportQuery {
                page: page.map(str::to_owned),
                entry: entry.map(str::to_owned),
                export,
                interval: Some("hour".into()),
                ..query("2026-10-27", "2026-10-27")
            },
        )
        .await;
        assert_eq!(
            r.timeline[..3]
                .iter()
                .map(|b| (b.visitors, b.visits))
                .collect::<Vec<_>>(),
            [(2, 3), (gap, gap), (1, 1)]
        );
    }
    let r = report(
        &db,
        ReportQuery {
            page: Some("/target".into()),
            interval: Some("minute".into()),
            ..query("2026-10-27", "2026-10-27")
        },
    )
    .await;
    assert_eq!((r.timeline[60].visitors, r.timeline[60].visits), (1, 1));
}
