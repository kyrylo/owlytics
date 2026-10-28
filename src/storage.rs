use crate::analytics;
use rusqlite::{Connection, OpenFlags, params};
use std::{fs::File, io::Read, net::IpAddr, path::Path};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Csv(#[from] csv::Error),
    #[error("{0}")]
    Invalid(String),
}

/// Replace country or city data atomically. Both DB-IP Lite CSV formats are headerless.
pub fn import_geo(connection: &mut Connection, input: impl Read) -> Result<u64, Error> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM geo_ranges", [])?;
    let mut csv = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(input);
    let mut insert =
        tx.prepare("INSERT INTO geo_ranges(family,start,end,country) VALUES(?1,?2,?3,?4)")?;
    let mut locations =
        tx.prepare("INSERT INTO geo_locations(family,start,region,city) VALUES(?1,?2,?3,?4)")?;
    let mut count = 0u64;
    for record in csv.records() {
        let record = record?;
        let invalid = || Error::Invalid(format!("Invalid country range on row {}", count + 1));
        if record.len() != 3 && record.len() != 8 {
            return Err(invalid());
        }
        let country = &record[if record.len() == 8 { 3 } else { 2 }];
        let start: IpAddr = record[0].parse().map_err(|_| invalid())?;
        let end: IpAddr = record[1].parse().map_err(|_| invalid())?;
        let (family, start) = analytics::ip_key(start);
        let (end_family, end) = analytics::ip_key(end);
        if family != end_family
            || start > end
            || country.len() != 2
            || !country.bytes().all(|c| c.is_ascii_uppercase())
        {
            return Err(invalid());
        }
        insert.execute(params![family, start, end, country])?;
        if record.len() == 8 {
            if [&record[4], &record[5]]
                .iter()
                .any(|v| v.len() > 256 || v.chars().any(char::is_control))
            {
                return Err(invalid());
            }
            // Coordinates are deliberately not imported or attached to visits.
            let region = if record[4].is_empty() {
                String::new()
            } else {
                format!("{}: {}", country, &record[4])
            };
            let city = if record[5].is_empty() {
                String::new()
            } else {
                format!("{}: {}, {}", country, &record[5], &record[4])
            };
            locations.execute(params![
                family,
                start,
                crate::geography::location_key("region", &region),
                crate::geography::location_key("city", &city)
            ])?;
        }
        count += 1;
    }
    drop(insert);
    drop(locations);
    if count == 0 {
        return Err(Error::Invalid("The country CSV is empty".into()));
    }
    let overlaps: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM (SELECT start,LAG(end) OVER(PARTITION BY family ORDER BY start) AS previous_end FROM geo_ranges) WHERE start<=previous_end)", [], |r| r.get(0))?;
    if overlaps {
        return Err(Error::Invalid("Country ranges overlap".into()));
    }
    tx.execute("INSERT INTO metadata VALUES('geo_source','DB-IP Lite') ON CONFLICT(key) DO UPDATE SET value=excluded.value", [])?;
    tx.commit()?;
    Ok(count)
}

fn new_private_file(path: &Path) -> Result<File, Error> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

/// SQLite's backup API includes committed WAL data. Ephemeral credentials and
/// visitor salts are removed from the copy before it is handed to the operator.
pub fn backup(connection: &Connection, destination: &Path) -> Result<(), Error> {
    let file = new_private_file(destination)?;
    let result = (|| {
        connection.backup(rusqlite::MAIN_DB, destination, None)?;
        let copy = Connection::open(destination)?;
        copy.execute_batch("PRAGMA secure_delete=ON; DELETE FROM owner_sessions; DELETE FROM login_limit; DELETE FROM site_salts; VACUUM;")?;
        let integrity: String = copy.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(Error::Invalid("Backup integrity check failed".into()));
        }
        drop(copy);
        file.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

/// Restore into a new file only, so neither live databases nor existing backups
/// can be overwritten by a mistaken path.
pub fn restore(source: &Path, destination: &Path) -> Result<(), Error> {
    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !(4..=crate::db::SCHEMA_VERSION).contains(&version) {
        return Err(Error::Invalid(
            "Restore requires a supported Owlytics backup".into(),
        ));
    }
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(Error::Invalid("Backup integrity check failed".into()));
    }
    backup(&connection, destination)
}
