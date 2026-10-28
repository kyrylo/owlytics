use owlytics::{
    analytics::{self, ReportQuery, ReportScope},
    db::Database,
};
use std::collections::BTreeMap;

fn query() -> ReportQuery {
    ReportQuery {
        from: Some("2025-01-01".into()),
        to: Some("2025-01-01".into()),
        interval: Some("day".into()),
        scope: ReportScope::Timeline,
        comparison_names: Some(BTreeMap::from([
            ("page".into(), vec!["/rare".into(), "/new".into()]),
            ("source".into(), vec!["GitHub".into(), "New source".into()]),
        ])),
        ..Default::default()
    }
}

async fn report(db: &Database, q: ReportQuery) -> analytics::Report {
    let range = q.range("2025-01-02".parse().unwrap()).unwrap();
    analytics::report(db, 1, range, q).await.unwrap()
}

#[tokio::test]
async fn comparison_lookup_uses_exact_names_beyond_previous_rank_and_page() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        let tx=c.unchecked_transaction()?;
        for i in 0..150 {
            // Two visitors each put /rare below the default top-100 limit.
            for _ in 0..2 {
                tx.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1735689600,'2025-01-01',randomblob(16),randomblob(16),?1,'Google','US','Chrome','Desktop')",[format!("/common/{i}")])?;
            }
        }
        for (seconds,path) in [(0,"/rare"),(30,"/finish")] {
            tx.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,referrer,country,browser,device) VALUES(1,1735689600+?1,'2025-01-01',zeroblob(16),randomblob(16),?2,'GitHub','github.com/article','PH','Safari','Mobile')",rusqlite::params![seconds,path])?;
        }
        tx.commit()
    }).await.unwrap();
    let mut q = query();
    q.detail = Some("page".into());
    q.search = Some("not present".into());
    q.offset = Some(100);
    q.sort = Some("name".into());
    q.order = Some("desc".into());
    let result = report(&db, q).await;
    assert_eq!(
        result
            .pages
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["/rare"]
    );
    assert_eq!(
        (
            result.pages[0].visitors,
            result.pages[0].pageviews,
            result.pages[0].visits
        ),
        (1, 1, 1)
    );
    assert_eq!(
        (
            result.sources[0].visitors,
            result.sources[0].pageviews,
            result.sources[0].duration
        ),
        (1, 2, 30)
    );
    assert_eq!(result.sources[0].percentage, Some((1, 301)));
    assert_eq!(result.totals.visitors, 301);
    assert!(result.countries.is_empty() && result.browsers.is_empty());
    assert!(result.comparison_missing.is_empty());
}

#[tokio::test]
async fn absent_imported_rows_are_zero_only_when_the_dimension_is_covered() {
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute_batch(
            "INSERT INTO sites VALUES(1,'example.com','UTC',0);
        INSERT INTO import_batches VALUES(1,1,'2025-01-01','2025-01-01',0);
        INSERT INTO import_daily VALUES(1,'2025-01-01',1,10,20);",
        )
    })
    .await
    .unwrap();
    let missing = report(&db, query()).await;
    assert!(missing.comparison_missing.contains("source"));
    assert!(missing.comparison_missing.contains("page"));
    db.call(|c|c.execute("INSERT INTO import_files VALUES(1,'sources','[\"source\",\"visitors\",\"pageviews\"]')",[])).await.unwrap();
    let covered = report(&db, query()).await;
    assert!(covered.sources.is_empty());
    assert!(!covered.comparison_missing.contains("source"));
    assert!(covered.comparison_missing.contains("page"));
    let disabled = report(
        &db,
        ReportQuery {
            with_imported: Some(false),
            ..query()
        },
    )
    .await;
    assert!(disabled.comparison_missing.is_empty());
}

#[tokio::test]
async fn comparison_lookup_preserves_the_audience_and_cannot_be_set_by_url() {
    let parsed: ReportQuery =
        serde_json::from_str(r#"{"comparison_names":{"page":["/rare"]}}"#).unwrap();
    assert!(parsed.comparison_names.is_none());
    let db = Database::open(":memory:").unwrap();
    db.call(|c| {
        c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)",[])?;
        for country in ["US","US","PH"] {
            c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,1735689600,'2025-01-01',randomblob(16),randomblob(16),'/rare','GitHub',?1,'Chrome','Desktop')",[country])?;
        }
        Ok(())
    }).await.unwrap();
    let result = report(
        &db,
        ReportQuery {
            country: Some("PH".into()),
            ..query()
        },
    )
    .await;
    assert_eq!(result.pages[0].visitors, 1);
    assert_eq!(result.sources[0].visitors, 1);
    assert_eq!(result.sources[0].percentage, Some((1, 1)));
}
