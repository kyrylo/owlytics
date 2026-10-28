//! Disposable performance fixture: cargo run --release --example benchmark -- NEW.sqlite [rows]
use owlytics::{
    analytics::{self, ReportQuery},
    db::Database,
};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let path = PathBuf::from(arguments.next().ok_or("Pass a NEW database path")?);
    let option = arguments.next();
    let report_only = option.as_deref() == Some("--report-only");
    if path.exists() && !report_only {
        return Err("Refusing to overwrite an existing database".into());
    }
    let rows: i64 = option
        .filter(|_| !report_only)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(1_100_000);
    if !(1..=10_000_000).contains(&rows) {
        return Err("Choose 1 through 10,000,000 rows".into());
    }
    let opened = Instant::now();
    let db = Database::open(&path)?;
    println!(
        "open_migration_milliseconds={:.1}",
        opened.elapsed().as_secs_f64() * 1000.
    );
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).max_blocking_threads(4).enable_all().build()?.block_on(async {
        if !report_only {
        db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])).await?;
        let started = Instant::now();
        for start in (0..rows).step_by(10_000) {
            db.call(move |c| {
                let tx = c.transaction()?;
                let mut insert = tx.prepare("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
                for i in start..(start+10_000).min(rows) {
                    let day = i % 30 + 1;
                    let visitor = ((day * 100_000 + (i/30)%10_000) as u128).to_be_bytes();
                    let id = (i as u128).to_be_bytes();
                    insert.execute(rusqlite::params![1_759_276_800+i, format!("2025-10-{day:02}"), visitor.as_slice(), id.as_slice(), format!("/page/{}", (i/7)%1000), ["","google.com","github.com","news.ycombinator.com"][(i%4) as usize], ["US","GB","PH","DE","JP"][(i%5) as usize], ["Chrome","Safari","Firefox"][(i%3) as usize], ["Desktop","Mobile"][(i%2) as usize]])?;
                }
                drop(insert); tx.commit()
            }).await?;
            if (start+10_000) % 250_000 == 0 { eprintln!("Seeded {} rows", start+10_000); }
        }
        println!("seed_rows={rows} seed_seconds={:.3}", started.elapsed().as_secs_f64());
        }
        db.call(|c| c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;")).await?;
        let query = ReportQuery { from: Some("2025-10-01".into()), to: Some("2025-10-30".into()), ..Default::default() };
        for trial in 1..=3 {
            let started = Instant::now();
            let report = analytics::report(&db, 1, query.range("2025-10-30".parse()?).unwrap(), query.clone()).await?;
            assert_eq!(report.pageviews, rows);
            assert_eq!(report.visitors, rows.min(300_000));
            println!("report_trial={trial} milliseconds={:.1} visitors={} pageviews={}", started.elapsed().as_secs_f64()*1000., report.visitors, report.pageviews);
        }
        let filtered = ReportQuery { page: Some("/page/1".into()), ..query.clone() };
        let started = Instant::now();
        let report = analytics::report(&db, 1, filtered.range("2025-10-30".parse()?).unwrap(), filtered).await?;
        println!("filtered_milliseconds={:.1} filtered_pageviews={} time_on_page_seconds={:.3} scroll_depth_percent={:.3}", started.elapsed().as_secs_f64()*1000., report.pageviews,analytics::Metric::TimeOnPage.value(&report.totals),analytics::Metric::ScrollDepth.value(&report.totals));
        let broad = ReportQuery { browser: Some("Chrome".into()), ..query.clone() };
        let started = Instant::now();
        let report = analytics::report(&db, 1, broad.range("2025-10-30".parse()?).unwrap(), broad).await?;
        assert_eq!(report.pageviews, (rows + 2) / 3);
        println!("broad_filter_milliseconds={:.1} broad_filter_pageviews={}", started.elapsed().as_secs_f64()*1000., report.pageviews);
        for (label, browser, scope) in [
            ("dashboard", None, analytics::ReportScope::Dashboard),
            ("filtered_dashboard", Some("Chrome"), analytics::ReportScope::Dashboard),
            ("filtered_comparison", Some("Chrome"), analytics::ReportScope::Timeline),
        ] {
            let selected = ReportQuery { browser:browser.map(str::to_owned), scope, ..query.clone() };
            let started = Instant::now();
            let report = analytics::report(&db, 1, selected.range("2025-10-30".parse()?).unwrap(), selected).await?;
            assert_eq!(report.pageviews, if browser.is_some() {(rows+2)/3} else {rows});
            println!("{label}_milliseconds={:.1} pageviews={}",started.elapsed().as_secs_f64()*1000.,report.pageviews);
        }
        let integrity = db.read(|c| c.query_row("PRAGMA quick_check", [], |r| r.get::<_,String>(0))).await?;
        assert_eq!(integrity, "ok");
        println!("database_bytes={}", std::fs::metadata(&path)?.len());
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
