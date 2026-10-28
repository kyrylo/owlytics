//! SQLite collection cost as unrelated daily visitors grow, without ANALYZE.
//! Uses real event/acquisition triggers in batches; this is NOT HTTP throughput.
//! cargo run --release --example collection_growth -- NEW.sqlite 100000
//! Add `legacy-index` to reproduce the schema-10 index on a disposable NEW file.
use owlytics::db::Database;
use rusqlite::{StatementStatus, params};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or("Pass a NEW database path")?);
    let count: i64 = args.next().ok_or("Pass a row count")?.parse()?;
    let legacy = match args.next().as_deref() {
        Some("legacy-index") => true,
        None => false,
        _ => return Err("Only optional argument: legacy-index".into()),
    };
    if path.exists()
        || !(1000..=100_000).contains(&count)
        || count % 1000 != 0
        || args.next().is_some()
    {
        return Err("Use a NEW file and 1,000–100,000 rows in multiples of 1,000".into());
    }
    let db = Database::open(&path)?;
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        db.call(move |c| {
            c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
            if legacy {
                c.execute_batch("DROP INDEX sessions_site_day; CREATE INDEX sessions_site_day ON sessions(site_id,day); DROP INDEX events_site_path_day; CREATE INDEX events_site_path_day ON events(site_id,path,day);")?;
            }
            Ok(())
        }).await?;
        println!("index={} statistics=none collection=sqlite-trigger-batches", if legacy {"legacy"} else {"site-day-visitor"});
        let total_started = Instant::now();
        for start in (0..count).step_by(1000) {
            let started = Instant::now();
            let steps = db.call(move |c| {
                let tx = c.transaction()?;
                let mut insert = tx.prepare("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,unixepoch('2025-10-15 12:00:00'),'2025-10-15',?1,?1,?2,'Google','US','Chrome','Desktop')")?;
                let mut acquisition = tx.prepare("INSERT INTO session_acquisition SELECT session_id,'Organic Search','google','organic','launch','','',NULL FROM events WHERE site_id=1 AND request_id=?1")?;
                for id in start..start+1000 {
                    let id_bytes = (id as u128).to_be_bytes();
                    insert.execute(params![id_bytes.as_slice(),format!("/article/{}",id%100)])?;
                    acquisition.execute([id_bytes.as_slice()])?;
                }
                let steps = insert.get_status(StatementStatus::VmStep) + acquisition.get_status(StatementStatus::VmStep);
                drop(insert);
                drop(acquisition);
                tx.commit()?;
                Ok(steps)
            }).await?;
            if start < 5000 || start + 1000 == count || (start + 1000) % 10000 == 0 {
                println!("daily_visitors={} batch_views=1000 batch_ms={:.1} vm_steps_per_view={:.1}", start+1000, started.elapsed().as_secs_f64()*1000., f64::from(steps)/1000.);
            }
        }
        println!("total_views={count} seconds={:.3}",total_started.elapsed().as_secs_f64());
        db.call(move |c| {
            assert_eq!(c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='sqlite_stat1'", [], |r| r.get::<_,i64>(0))?,0);
            for sql in ["SELECT COUNT(*) FROM events", "SELECT COUNT(*) FROM sessions", "SELECT COUNT(*) FROM session_acquisition", "SELECT pageviews FROM daily_reports WHERE dimension='total'", "SELECT visitors FROM daily_reports WHERE dimension='utm_campaign'", "SELECT visits FROM daily_session_reports WHERE dimension='total'"] {
                assert_eq!(c.query_row(sql, [], |r| r.get::<_,i64>(0))?, count, "{sql}");
            }
            assert_eq!(c.query_row("PRAGMA quick_check", [], |r| r.get::<_,String>(0))?,"ok");
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
            Ok(())
        }).await?;
        println!("counts=ok integrity=ok database_bytes={}",std::fs::metadata(path)?.len());
        Ok::<_,Box<dyn std::error::Error>>(())
    })
}
