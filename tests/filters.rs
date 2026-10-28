use owlytics::{
    analytics::{self, Metric, Report, ReportQuery, ReportScope},
    db::Database,
    filters::{Filter, Operator},
};

fn filter(key: &str, op: Operator, values: &[&str]) -> Filter {
    Filter {
        key: key.into(),
        op,
        values: values.iter().map(|value| (*value).into()).collect(),
    }
}
fn query(filters: Vec<Filter>) -> ReportQuery {
    ReportQuery {
        from: Some("2026-10-27".into()),
        to: Some("2026-10-27".into()),
        filters,
        ..Default::default()
    }
}
async fn report(db: &Database, query: ReportQuery) -> Report {
    let range = query.range("2026-10-27".parse().unwrap()).unwrap();
    analytics::report(db, 1, range, query).await.unwrap()
}
async fn fixture() -> Database {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
        for (id, visitor, path, seconds, browser, country, time, depth) in [
            (1i128, 1i128, "/a", 0, "Chrome", "US", 1000, 20),
            (2, 1, "/b", 30, "Chrome", "US", 3000, 80),
            (3, 1, "/a", 60, "Chrome", "US", 2000, 60),
            (4, 2, "/b", 70, "Safari", "US", 5000, 100),
            (5, 3, "/c", 80, "Firefox", "PH", 8000, 40),
            (6, 4, "/%_quote'🦉", 90, "Chrome", "PH", 1000, 30),
        ] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1793059200+?1,'2026-10-27',?2,?3,?4,'',?5,?6,'Desktop')",
                rusqlite::params![seconds, visitor.to_be_bytes().as_slice(), id.to_be_bytes().as_slice(), path, country, browser])?;
            c.execute("INSERT INTO page_engagement VALUES(?1,?2,?3)", rusqlite::params![id as i64,time,depth])?;
        }
        Ok(())
    }).await.unwrap();
    db
}

#[tokio::test]
async fn grouped_pages_keep_distinct_visitors_and_weighted_per_page_engagement() {
    let db = fixture().await;
    for interval in ["hour", "day"] {
        let mut q = query(vec![filter("page", Operator::Is, &["/a", "/b"])]);
        q.interval = Some(interval.into());
        let result = report(&db, q).await;
        assert_eq!(
            (result.visitors, result.pageviews, result.totals.visits),
            (2, 4, 2)
        );
        assert_eq!(
            (result.totals.bounces, result.totals.entrances),
            (1, Some(2))
        );
        assert_eq!(Metric::BounceRate.value(&result.totals), 50.);
        assert_eq!(
            (
                result.totals.time_on_page_ms,
                result.totals.time_on_page_visits
            ),
            (11000, 3)
        );
        assert_eq!(
            (
                result.totals.total_scroll_depth,
                result.totals.scroll_depth_visits
            ),
            (240, 3)
        );
        assert_eq!(Metric::ScrollDepth.value(&result.totals), 80.);
        assert_eq!(result.pages.len(), 2);
        if interval == "hour" {
            let bucket = &result.timeline[0];
            assert_eq!(bucket.value(Metric::ScrollDepth), Some(80.));
            assert_eq!(bucket.value(Metric::BounceRate), Some(50.));
            assert_eq!(bucket.value(Metric::TimeOnPage), Some(11. / 3.));
        }
    }
}

#[tokio::test]
async fn grouped_filters_union_values_intersect_dimensions_and_exclude_every_match() {
    let db = fixture().await;
    for (filters, expected) in [
        (
            vec![filter("browser", Operator::Is, &["Chrome", "Safari"])],
            (3, 5),
        ),
        (
            vec![filter("browser", Operator::IsNot, &["Chrome", "Safari"])],
            (1, 1),
        ),
        (vec![filter("page", Operator::IsNot, &["/a", "/b"])], (2, 2)),
        (
            vec![filter("page", Operator::Contains, &["%", "nonexistent"])],
            (1, 1),
        ),
        (
            vec![filter("page", Operator::Contains, &["_quote'🦉"])],
            (1, 1),
        ),
        (
            vec![filter("page", Operator::ContainsNot, &["/a", "/b"])],
            (2, 2),
        ),
        (
            vec![
                filter("page", Operator::Is, &["/a", "/b"]),
                filter("browser", Operator::IsNot, &["Safari"]),
            ],
            (1, 3),
        ),
        (
            vec![
                filter("browser", Operator::IsNot, &["Safari"]),
                filter("page", Operator::Is, &["/a", "/b"]),
            ],
            (1, 3),
        ),
        (vec![filter("source", Operator::Is, &[""])], (4, 6)),
        (vec![filter("entry", Operator::Is, &["/a", "/b"])], (2, 4)),
        (vec![filter("exit", Operator::IsNot, &["/a", "/b"])], (2, 2)),
    ] {
        for scope in [
            ReportScope::All,
            ReportScope::Dashboard,
            ReportScope::Timeline,
        ] {
            for window in [None, Some((1793059200, 1793059300))] {
                let mut q = query(filters.clone());
                q.scope = scope;
                q.window = window;
                let result = report(&db, q).await;
                assert_eq!(
                    (result.visitors, result.pageviews),
                    expected,
                    "{filters:?} {scope:?} {window:?}"
                );
            }
        }
    }
    let mut legacy = query(vec![]);
    legacy.browser = Some("Chrome".into());
    for scope in [ReportScope::All, ReportScope::Timeline] {
        legacy.scope = scope;
        legacy.interval = Some("day".into());
        let mut grouped = query(vec![filter("browser", Operator::Is, &["Chrome"])]);
        grouped.scope = scope;
        grouped.interval = Some("day".into());
        assert_eq!(
            report(&db, legacy.clone()).await.totals,
            report(&db, grouped).await.totals
        );
    }
}

#[tokio::test]
async fn malformed_filters_are_rejected_before_querying_even_for_library_callers() {
    let db = fixture().await;
    let range = query(vec![]).range("2026-10-27".parse().unwrap()).unwrap();
    for filters in [
        vec![filter("page", Operator::Is, &[])],
        vec![filter("unknown", Operator::Is, &["x"])],
        vec![filter("country", Operator::Contains, &["US"])],
        vec![filter("page", Operator::Contains, &[""])],
        vec![filter("page", Operator::Is, &["a\nb"])],
        vec![
            filter("page", Operator::Is, &["/a"]),
            filter("page", Operator::IsNot, &["/b"]),
        ],
        vec![filter("page", Operator::Is, &[&"x".repeat(2049)])],
        vec![filter("page", Operator::Is, &vec!["/a"; 51])],
        vec![filter(
            "page",
            Operator::Is,
            &vec!["\"".repeat(1000).as_str(); 5],
        )],
    ] {
        let q = query(filters);
        assert!(q.range(range.from).is_err());
        assert!(analytics::report(&db, 1, range.clone(), q).await.is_err());
    }
    let mut mixed = query(vec![filter("browser", Operator::Is, &["Safari"])]);
    mixed.browser = Some("Chrome".into());
    assert!(mixed.range(range.from).is_err());
}
