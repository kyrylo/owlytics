//! Import Plausible's full site export, one record at a time. Aggregates remain
//! aggregates: no synthetic events or invented cross-dimension intersections.
use crate::{analytics, storage::Error};
use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Summary {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub visitors: i64,
    pub pageviews: i64,
    pub skipped_rows: u64,
    pub ignored: Vec<String>,
    pub missing: Vec<String>,
    pub session_date_differences: bool,
}

struct Export {
    path: PathBuf,
    from: NaiveDate,
    to: NaiveDate,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn exports(directory: &Path) -> Result<(BTreeMap<String, Export>, Vec<String>), Error> {
    let mut files = BTreeMap::new();
    let mut ignored = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let filename = entry.file_name();
        let Some(filename) = filename.to_str() else {
            return Err(invalid("Export filenames must be UTF-8"));
        };
        // macOS archive/copy tools can add AppleDouble metadata beside CSVs.
        if filename.starts_with("._") {
            continue;
        }
        let Some(stem) = filename.strip_suffix(".csv") else {
            continue;
        };
        let mut parts = stem.rsplitn(3, '_');
        let parse = |value: Option<&str>| {
            value
                .filter(|v| v.len() == 8)
                .and_then(|v| NaiveDate::parse_from_str(v, "%Y%m%d").ok())
                .ok_or_else(|| {
                    invalid(format!(
                        "Expected a full site export filename, got {filename}"
                    ))
                })
        };
        let to = parse(parts.next())?;
        let from = parse(parts.next())?;
        let table = parts.next().unwrap_or("").trim_start_matches("imported_");
        if from > to {
            return Err(invalid(format!("Reversed date range in {filename}")));
        }
        if ["custom_events", "custom_props"].contains(&table) {
            ignored.push(table.to_owned());
            continue;
        }
        if ![
            "visitors",
            "pages",
            "sources",
            "locations",
            "devices",
            "browsers",
            "operating_systems",
            "entry_pages",
            "exit_pages",
        ]
        .contains(&table)
        {
            return Err(invalid(format!("Unrecognized export table: {table}")));
        }
        if files
            .insert(
                table.to_owned(),
                Export {
                    path: entry.path(),
                    from,
                    to,
                },
            )
            .is_some()
        {
            return Err(invalid(format!(
                "More than one {table} file; import one export at a time"
            )));
        }
    }
    ignored.sort();
    Ok((files, ignored))
}

struct Columns {
    names: csv::StringRecord,
}
impl Columns {
    fn new(names: csv::StringRecord, required: &[&str]) -> Result<Self, Error> {
        let mut seen = BTreeSet::new();
        if names.iter().any(|name| !seen.insert(name)) {
            return Err(invalid("Duplicate CSV column"));
        }
        for required in required {
            if !names.iter().any(|name| name == *required) {
                return Err(invalid(format!("Missing CSV column: {required}")));
            }
        }
        Ok(Self { names })
    }
    fn get<'a>(&self, row: &'a csv::StringRecord, name: &str) -> &'a str {
        self.names
            .iter()
            .position(|column| column == name)
            .and_then(|i| row.get(i))
            .unwrap_or("")
    }
    fn count(&self, row: &csv::StringRecord, name: &str) -> Result<i64, Error> {
        let value = self.get(row, name);
        if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid(format!("Invalid {name} count")));
        }
        value
            .parse::<i64>()
            .ok()
            .filter(|n| *n <= 1_000_000_000_000)
            .ok_or_else(|| invalid(format!("{name} count is too large")))
    }
}

fn dimension(
    table: &str,
    columns: &Columns,
    row: &csv::StringRecord,
    domain: &str,
    allowed_hostnames: &[String],
) -> Result<(&'static str, String), Error> {
    let get = |column| columns.get(row, column);
    let (key, value) = match table {
        "pages" | "entry_pages" | "exit_pages" => {
            let column = match table {
                "entry_pages" => "entry_page",
                "exit_pages" => "exit_page",
                _ => "page",
            };
            let path = get(column).split(['?', '#']).next().unwrap_or("");
            // CE exports event-only visits with no entry/exit page. Browser
            // about:blank pageviews also appear under the literal path "blank".
            // Keep these historical labels and counts instead of inventing URLs.
            let legacy_label = path == "blank"
                || (path.is_empty() && table != "pages" && columns.count(row, "pageviews")? == 0);
            if (!path.starts_with('/') && !legacy_label) || path.starts_with("//") {
                return Err(invalid("Invalid page path"));
            }
            let hostname = get("hostname");
            if !hostname.is_empty()
                && !analytics::matches_domain(hostname, domain)
                && !allowed_hostnames
                    .iter()
                    .any(|allowed| hostname.eq_ignore_ascii_case(allowed))
            {
                return Err(invalid(
                    "The pages export contains a different hostname; import the matching site's export",
                ));
            }
            (
                match table {
                    "entry_pages" => "entry",
                    "exit_pages" => "exit",
                    _ => "page",
                },
                path.to_owned(),
            )
        }
        "sources" => {
            // The export already contains the attributed source. A campaign
            // tag can intentionally override a different referring website.
            let source = get("source").to_owned();
            (
                "source",
                if ["Direct / None", "(direct)", "Direct", "None"].contains(&source.as_str()) {
                    String::new()
                } else {
                    source
                },
            )
        }
        "locations" => {
            let value = get("country");
            if !value.is_empty()
                && (value.len() != 2 || !value.bytes().all(|c| c.is_ascii_uppercase()))
            {
                return Err(invalid("Invalid country code"));
            }
            ("country", value.to_owned())
        }
        "operating_systems" => ("os", get("operating_system").to_owned()),
        "devices" => (
            "device",
            match get("device") {
                "Laptop" => "Desktop",
                other => other,
            }
            .to_owned(),
        ),
        "browsers" => (
            "browser",
            match get("browser") {
                "Microsoft Edge" => "Edge",
                other => other,
            }
            .to_owned(),
        ),
        _ => unreachable!(),
    };
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(invalid("Invalid dimension value"));
    }
    Ok((key, value))
}

pub fn import(
    connection: &mut Connection,
    domain: &str,
    directory: &Path,
    today: NaiveDate,
) -> Result<Summary, Error> {
    import_with_hostnames(connection, domain, directory, today, &[], false)
}

/// Run the same validation and totals calculation, then roll back every write.
pub fn preview(
    connection: &mut Connection,
    domain: &str,
    directory: &Path,
    today: NaiveDate,
) -> Result<Summary, Error> {
    import_with_hostnames(connection, domain, directory, today, &[], true)
}

/// Explicitly accept additional hostname labels in a verified site's export.
/// Native collection and the default importer retain their normal host checks.
pub fn import_with_hostnames(
    connection: &mut Connection,
    domain: &str,
    directory: &Path,
    today: NaiveDate,
    allowed_hostnames: &[String],
    dry_run: bool,
) -> Result<Summary, Error> {
    if allowed_hostnames
        .iter()
        .any(|host| host.is_empty() || host.len() > 253 || host.chars().any(char::is_control))
    {
        return Err(invalid("Invalid allowed hostname label"));
    }
    let (files, ignored) = exports(directory)?;
    let visitor_file = files
        .get("visitors")
        .ok_or_else(|| invalid("The full visitors CSV is required"))?;
    if files
        .values()
        .any(|file| file.from != visitor_file.from || file.to != visitor_file.to)
    {
        return Err(invalid(
            "All files must belong to the same export date range",
        ));
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let site: Option<(i64, String)> = tx
        .query_row(
            "SELECT id,domain FROM sites WHERE domain=?1",
            [domain],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (site_id, domain) =
        site.ok_or_else(|| invalid("Add the website in Owlytics before importing"))?;
    let first_native: Option<String> = tx.query_row(
        "SELECT MIN(day) FROM events WHERE site_id=?1",
        [site_id],
        |r| r.get(0),
    )?;
    let cutoff = first_native
        .and_then(|day| NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok())
        .map(|day| day.min(today))
        .unwrap_or(today);
    let from = visitor_file.from;
    let to = visitor_file.to.min(
        cutoff
            .pred_opt()
            .ok_or_else(|| invalid("Invalid cutover date"))?,
    );
    if from > to {
        return Err(invalid(
            "There are no historical dates before the collection cutover",
        ));
    }
    let overlap: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM import_batches WHERE site_id=?1 AND from_day<=?3 AND to_day>=?2)", params![site_id, from.to_string(), to.to_string()], |r| r.get(0))?;
    if overlap {
        return Err(invalid(
            "This date range has already been imported or overlaps an earlier import",
        ));
    }
    tx.execute(
        "INSERT INTO import_batches(site_id,from_day,to_day,imported_at) VALUES(?1,?2,?3,?4)",
        params![
            site_id,
            from.to_string(),
            to.to_string(),
            crate::auth::now()
        ],
    )?;
    let batch = tx.last_insert_rowid();
    let mut summary = Summary {
        from,
        to,
        visitors: 0,
        pageviews: 0,
        skipped_rows: 0,
        session_date_differences: false,
        ignored,
        missing: ["pages", "sources", "locations", "devices", "browsers"]
            .into_iter()
            .filter(|table| !files.contains_key(*table))
            .map(str::to_owned)
            .collect(),
    };
    let mut ordered: Vec<_> = files.iter().collect();
    ordered.sort_by_key(|(table, _)| if *table == "visitors" { 0 } else { 1 });
    for (table, file) in ordered {
        let mut csv = csv::Reader::from_reader(File::open(&file.path)?);
        let mut required = vec!["date", "pageviews"];
        required.extend(match table.as_str() {
            "visitors" => vec!["visitors"],
            "pages" => vec!["page", "hostname"],
            "sources" => vec!["source", "referrer"],
            "locations" => vec!["country"],
            "devices" => vec!["device"],
            "browsers" => vec!["browser"],
            "operating_systems" => vec!["operating_system"],
            "entry_pages" => vec!["entry_page"],
            "exit_pages" => vec!["exit_page"],
            _ => unreachable!(),
        });
        let columns = Columns::new(csv.headers()?.clone(), &required)?;
        let headers = serde_json::to_string(&columns.names.iter().collect::<Vec<_>>())
            .expect("string list is serializable");
        tx.execute(
            "INSERT INTO import_files VALUES(?1,?2,?3)",
            params![batch, table, headers],
        )?;
        for (index, record) in csv.records().enumerate() {
            let record = record?;
            if record.iter().any(|value| value.len() > 4096) {
                return Err(invalid(format!(
                    "Oversized field in {table}, row {}",
                    index + 2
                )));
            }
            let date = NaiveDate::parse_from_str(columns.get(&record, "date"), "%Y-%m-%d")
                .map_err(|_| invalid(format!("Invalid date in {table}, row {}", index + 2)))?;
            if date < file.from || date > file.to {
                return Err(invalid(format!(
                    "Date outside the filename's range in {table}, row {}",
                    index + 2
                )));
            }
            let views = columns.count(&record, "pageviews")?;
            let visitors = if table == "visitors" {
                columns.count(&record, "visitors")?
            } else {
                0
            };
            if date > to {
                summary.skipped_rows += 1;
                continue;
            }
            let day = date.to_string();
            let optional = |name| -> Result<Option<i64>, Error> {
                if columns.get(&record, name).is_empty() {
                    Ok(None)
                } else {
                    columns.count(&record, name).map(Some)
                }
            };
            let imported_visitors = optional("visitors")?;
            let visits = optional(match table.as_str() {
                "entry_pages" => "entrances",
                "exit_pages" => "exits",
                _ => "visits",
            })?;
            let bounces = optional("bounces")?;
            let duration = optional("visit_duration")?;
            let exits = optional("exits")?;
            if table == "pages" {
                let time = optional("total_time_on_page")?;
                let time_visits = optional("total_time_on_page_visits")?;
                let scroll = optional("total_scroll_depth")?;
                let scroll_visits = optional("total_scroll_depth_visits")?;
                if matches!((time,time_visits),(Some(t),Some(0)) if t>0)
                    || matches!((scroll,scroll_visits),(Some(s),Some(v)) if s>100*v)
                {
                    return Err(invalid("Invalid page engagement totals"));
                }
                let (_, path) = dimension(table, &columns, &record, &domain, allowed_hostnames)?;
                tx.execute("INSERT INTO import_engagement VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(site_id,day,path) DO UPDATE SET time_ms=time_ms+excluded.time_ms,time_visits=time_visits+excluded.time_visits,scroll_depth=scroll_depth+excluded.scroll_depth,scroll_visits=scroll_visits+excluded.scroll_visits", params![site_id,day,path,batch,time.map(|seconds|seconds*1000),time_visits,scroll,scroll_visits])?;
            }
            if let (Some(b), Some(v)) = (bounces, visits) {
                if b > v {
                    return Err(invalid("Bounces exceed visits"));
                }
            }
            let mut dimensions = if table == "visitors" {
                vec![("total", String::new())]
            } else {
                vec![dimension(
                    table,
                    &columns,
                    &record,
                    &domain,
                    allowed_hostnames,
                )?]
            };
            match table.as_str() {
                "sources" => {
                    let raw = columns.get(&record, "referrer");
                    let url = if raw.contains("://") {
                        raw.to_owned()
                    } else {
                        format!("https://{raw}")
                    };
                    dimensions.push(("referrer", analytics::referrer(&url, &domain)));
                    let tags = [
                        "utm_source",
                        "utm_medium",
                        "utm_campaign",
                        "utm_content",
                        "utm_term",
                    ]
                    .map(|name| {
                        columns
                            .names
                            .iter()
                            .any(|column| column == name)
                            .then(|| columns.get(&record, name))
                    });
                    if tags
                        .iter()
                        .flatten()
                        .any(|value| value.len() > 2048 || value.chars().any(char::is_control))
                    {
                        return Err(invalid(
                            "Campaign tags must be at most 2,048 bytes without control characters",
                        ));
                    }
                    let channel = match (tags[0], tags[1], tags[2]) {
                        (Some(source), Some(medium), Some(campaign)) => Some(
                            crate::acquisition::channel(&dimensions[0].1, source, medium, campaign),
                        ),
                        _ => None,
                    };
                    tx.execute("INSERT INTO import_acquisition(site_id,day,batch_id,source,referrer,channel,utm_source,utm_medium,utm_campaign,utm_content,utm_term,pageviews,visitors,visits,bounces,duration) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)", params![site_id,day,batch,dimensions[0].1,dimensions[1].1,channel,tags[0],tags[1],tags[2],tags[3],tags[4],views,imported_visitors,visits,bounces,duration])?;
                    for (key, value) in [
                        "utm_source",
                        "utm_medium",
                        "utm_campaign",
                        "utm_content",
                        "utm_term",
                    ]
                    .into_iter()
                    .zip(tags)
                    {
                        if let Some(value) = value {
                            dimensions.push((key, value.to_owned()));
                        }
                    }
                    if let Some(channel) = channel {
                        dimensions.push(("channel", channel.to_owned()));
                    }
                }
                "locations" => {
                    let country = columns.get(&record, "country");
                    for key in ["region", "city"] {
                        if !columns.names.iter().any(|column| column == key) {
                            continue;
                        }
                        let value = columns.get(&record, key);
                        dimensions.push((
                            key,
                            if value.is_empty() || value == "0" {
                                String::new()
                            } else {
                                crate::geography::location_key(key, &format!("{country}: {value}"))
                            },
                        ));
                    }
                }
                "browsers"
                    if columns
                        .names
                        .iter()
                        .any(|column| column == "browser_version") =>
                {
                    dimensions.push((
                        "browser_version",
                        columns.get(&record, "browser_version").to_owned(),
                    ))
                }
                "operating_systems"
                    if columns
                        .names
                        .iter()
                        .any(|column| column == "operating_system_version") =>
                {
                    dimensions.push((
                        "os_version",
                        columns.get(&record, "operating_system_version").to_owned(),
                    ))
                }
                _ => {}
            }
            for (key, value) in &dimensions {
                if value.len() > 2048 || value.chars().any(char::is_control) {
                    return Err(invalid("Invalid dimension value"));
                }
                tx.execute("INSERT INTO import_metrics VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET visitors=visitors+excluded.visitors,visits=visits+excluded.visits,bounces=bounces+excluded.bounces,duration=duration+excluded.duration,exits=exits+excluded.exits",params![site_id,day,key,value,batch,imported_visitors,visits,bounces,duration,exits])?;
            }
            let group = match table.as_str() {
                "locations" => Some("location"),
                "browsers" => Some("browser"),
                "operating_systems" => Some("os"),
                _ => None,
            };
            if let Some(group) = group {
                let fields: BTreeMap<_, _> = dimensions
                    .iter()
                    .filter(|(key, _)| {
                        let column = match *key {
                            "os" => "operating_system",
                            "os_version" => "operating_system_version",
                            other => other,
                        };
                        columns.names.iter().any(|name| name == column)
                    })
                    .map(|(key, value)| (*key, value.as_str()))
                    .collect();
                let fields = serde_json::to_string(&fields).expect("string map is serializable");
                tx.execute("INSERT INTO import_groups(site_id,day,batch_id,kind,dimensions,pageviews,visitors,visits,bounces,duration) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![site_id,day,batch,group,fields,views,imported_visitors,visits,bounces,duration])?;
            }
            if table == "visitors" {
                tx.execute(
                    "INSERT INTO import_daily VALUES(?1,?2,?3,?4,?5)",
                    params![site_id, day, batch, visitors, views],
                )?;
                summary.visitors = summary
                    .visitors
                    .checked_add(visitors)
                    .ok_or_else(|| invalid("Visitor total overflow"))?;
                summary.pageviews = summary
                    .pageviews
                    .checked_add(views)
                    .ok_or_else(|| invalid("Pageview total overflow"))?;
            } else {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM import_daily WHERE site_id=?1 AND day=?2)",
                    params![site_id, day],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Err(invalid(format!(
                        "{table} contains a date missing from the visitors CSV"
                    )));
                }
                for (dimension, value) in dimensions {
                    tx.execute("INSERT INTO import_breakdowns VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(site_id,day,dimension,value) DO UPDATE SET pageviews=pageviews+excluded.pageviews", params![site_id,day,dimension,value,batch,views])?;
                }
            }
        }
    }
    let days: i64 = tx.query_row(
        "SELECT COUNT(*) FROM import_daily WHERE batch_id=?1",
        [batch],
        |r| r.get(0),
    )?;
    if days == 0 {
        return Err(invalid("The visitors CSV contains no historical rows"));
    }
    // Pages and headline pageviews use event dates. Other CE breakdowns use
    // session dates, so sessions crossing midnight can legitimately exceed a
    // day's headline total. Preserve those aggregates without shifting dates.
    let inconsistent: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM import_breakdowns b JOIN import_daily d ON d.site_id=b.site_id AND d.day=b.day WHERE b.batch_id=?1 AND b.dimension='page' GROUP BY b.day HAVING SUM(b.pageviews)>MAX(d.pageviews))", [batch], |r| r.get(0))?;
    if inconsistent {
        return Err(invalid(
            "Pages exceed their daily pageview total; check the export files",
        ));
    }
    summary.session_date_differences = tx.query_row("SELECT EXISTS(SELECT 1 FROM import_breakdowns b JOIN import_daily d ON d.site_id=b.site_id AND d.day=b.day WHERE b.batch_id=?1 AND b.dimension!='page' GROUP BY b.day,b.dimension HAVING SUM(b.pageviews)>MAX(d.pageviews))", [batch], |r| r.get(0))?;
    if dry_run {
        tx.rollback()?;
    } else {
        tx.commit()?;
    }
    Ok(summary)
}
