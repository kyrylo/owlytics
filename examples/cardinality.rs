//! Separate seeding from measurement so fixture creation does not inflate RSS.
//! cargo run --release --example cardinality -- NEW.sqlite seed 100000
//! /usr/bin/time -l target/release/examples/cardinality NEW.sqlite dashboard
use owlytics::{
    analytics::{self, ReportQuery, ReportScope},
    db::Database,
};
use std::{path::PathBuf, time::Instant};

fn session_plan(c: &rusqlite::Connection, phase: &str) -> rusqlite::Result<()> {
    let plan = c.prepare("EXPLAIN QUERY PLAN SELECT 1 FROM sessions s WHERE s.site_id=1 AND s.day='2025-10-15' AND s.visitor=zeroblob(16) AND s.entry_page='/page' AND s.id!=1 AND s.pageviews>0")?
        .query_map([], |r| r.get::<_, String>(3))?
        .collect::<Result<Vec<_>, _>>()?;
    println!("session_lookup_phase={phase} plan={}", plan.join("; "));
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or("Pass a database path")?);
    let mode = args
        .next()
        .ok_or("Choose seed, dashboard, filtered, exits, filtered-exits, detail, or search")?;
    if !matches!(
        mode.as_str(),
        "seed" | "dashboard" | "filtered" | "exits" | "filtered-exits" | "detail" | "search"
    ) {
        return Err(
            "Choose seed, dashboard, filtered, exits, filtered-exits, detail, or search".into(),
        );
    }
    if (mode == "seed") == path.exists() {
        return Err("Seed requires a NEW path; reports require an existing fixture".into());
    }
    let count: i64 = args
        .next()
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(100_000);
    if !(1..=500_000).contains(&count) || args.next().is_some() {
        return Err("Choose 1 through 500,000 rows".into());
    }
    let db = Database::open(&path)?;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .enable_all()
        .build()?
        .block_on(async {
            if mode == "seed" {
                let started = Instant::now();
                db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
                    .await?;
                db.read(|c| session_plan(c, "fresh")).await?;
                for start in (0..count).step_by(1000) {
                    db.call(move |c| {
                        let tx = c.transaction()?;
                        let mut insert = tx.prepare("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,unixepoch('2025-10-15 12:00:00'),'2025-10-15',?1,?1,?2,'Google','US','Chrome','Desktop')")?;
                        for id in start..(start + 1000).min(count) {
                            insert.execute(rusqlite::params![
                                (id as u128).to_be_bytes().as_slice(),
                                format!("/articles/{id:08}/a-long-but-valid-page-address-with-a-distinct-slug")
                            ])?;
                        }
                        drop(insert);
                        tx.commit()?;
                        // Keep this fixture representative of a fresh install:
                        // collection must scale before planner statistics exist.
                        if start == 0 {
                            session_plan(c, "populated_without_statistics")?;
                        }
                        Ok(())
                    }).await?;
                }
                db.call(|c| c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;"))
                    .await?;
                println!("seed_rows={count} seed_seconds={:.3}", started.elapsed().as_secs_f64());
            } else {
                let detail = matches!(mode.as_str(),"detail"|"search");
                let offset = if mode=="detail" {90_000} else if mode=="search" {50} else {0};
                let query = ReportQuery {
                    from: Some("2025-10-15".into()),
                    to: Some("2025-10-15".into()),
                    interval: Some("day".into()),
                    scope: ReportScope::Dashboard,
                    browser: matches!(mode.as_str(),"filtered"|"filtered-exits").then(|| "Chrome".into()),
                    pages_tab: matches!(mode.as_str(),"exits"|"filtered-exits").then(|| "exit".into()),
                    detail: detail.then(|| "page".into()),
                    sort: detail.then(|| "name".into()),
                    order: detail.then(|| if mode=="detail" {"desc"} else {"asc"}.into()),
                    search: (mode=="search").then(|| "099".into()),
                    offset: detail.then_some(offset),
                    ..Default::default()
                };
                let expected = db.read(|c| c.query_row("SELECT COUNT(*),COUNT(DISTINCT path),COUNT(DISTINCT visitor),COUNT(DISTINCT session_id) FROM events WHERE site_id=1", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?)))).await?;
                assert_eq!(expected, (expected.0, expected.0, expected.0, expected.0));
                let started = Instant::now();
                let report = analytics::report(&db, 1, query.range("2025-10-15".parse()?).unwrap(), query).await?;
                let elapsed = started.elapsed().as_secs_f64() * 1000.;
                assert_eq!((report.pageviews, report.visitors, report.totals.visits), (expected.0, expected.0, expected.0));
                if detail {
                    let search=mode=="search";
                    let (total,names)=db.read(move |c| {
                        let predicate=if search {"path LIKE '%099%'"} else {"1"};
                        let order=if search {"ASC"} else {"DESC"};
                        let total=c.query_row(&format!("SELECT COUNT(*) FROM events WHERE site_id=1 AND {predicate}"),[],|r|r.get::<_,i64>(0))?;
                        let names=c.prepare(&format!("SELECT path FROM events WHERE site_id=1 AND {predicate} ORDER BY path {order} LIMIT 50 OFFSET ?1"))?.query_map([offset as i64],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
                        Ok((total,names))
                    }).await?;
                    assert_eq!(report.detail_total,total as usize);
                    assert_eq!(report.pages.iter().map(|r|r.name.clone()).collect::<Vec<_>>(),names);
                } else {
                    assert_eq!(report.pages.len(), (expected.0 as usize).min(100));
                }
                assert!(report.pages.iter().all(|r| (r.visitors, r.pageviews, r.visits, r.entrances) == (1,1,1,Some(1))));
                if matches!(mode.as_str(),"exits"|"filtered-exits") {
                    assert_eq!(report.exit_pages.len(), (expected.0 as usize).min(100));
                    assert!(report.exit_pages.iter().all(|r| (r.pageviews, r.exits) == (1,1)));
                }
                println!("mode={mode} distinct_paths={} milliseconds={elapsed:.1} returned_pages={} counts=ok", expected.1, report.pages.len());
            }
            let integrity = db.read(|c| c.query_row("PRAGMA quick_check", [], |r| r.get::<_,String>(0))).await?;
            assert_eq!(integrity, "ok");
            println!("database_bytes={} integrity=ok", std::fs::metadata(path)?.len());
            Ok::<_, Box<dyn std::error::Error>>(())
        })
}
