//! Outbound clicks are separate from traffic and inherit their page's audience.
use crate::{
    analytics,
    db::{Database, Error},
};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub domain: String,
    pub id: String,
    pub page_id: String,
    pub url: String,
}

impl Event {
    pub fn validate(&mut self) -> bool {
        let Some(domain) = analytics::normalize_domain(&self.domain) else {
            return false;
        };
        self.domain = domain;
        if [&self.id, &self.page_id]
            .iter()
            .any(|id| id.len() != 32 || hex::decode(id).is_err())
            || self.url.len() > 2048
            || self.url.chars().any(char::is_control)
        {
            return false;
        }
        let Ok(url) = url::Url::parse(&self.url) else {
            return false;
        };
        if !matches!(url.scheme(), "http" | "https")
            || url
                .host_str()
                .is_none_or(|host| analytics::matches_domain(host, &self.domain))
        {
            return false;
        }
        // Sanitize again at the boundary, including clients other than our script.
        self.url = format!("{}{}", url.origin().ascii_serialization(), url.path());
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Collected {
    Stored,
    Duplicate,
    PendingPageview,
    Invalid,
}

pub(crate) async fn collect_request(
    db: &Database,
    event: Event,
    ua: String,
) -> Result<Collected, Error> {
    // An immediate exit can reach us before its pageview. Keep the keepalive
    // request open briefly: the departed document cannot run a retry timer.
    // Waiting is outside the SQLite worker, so the parent can be stored.
    for attempt in 0..=10 {
        let result = collect(db, event.clone(), ua.clone(), crate::auth::now).await?;
        if result != Collected::PendingPageview || attempt == 10 {
            return Ok(result);
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    unreachable!()
}

pub async fn collect(
    db: &Database,
    mut event: Event,
    ua: String,
    clock: impl FnOnce() -> i64 + Send + 'static,
) -> Result<Collected, Error> {
    if !event.validate() {
        return Ok(Collected::Invalid);
    }
    if analytics::user_agent(&ua).is_none() {
        return Ok(Collected::Duplicate);
    }
    db.call(move |c| {
        let now = clock();
        let parent = c.query_row(
            "SELECT e.id,e.site_id,e.received_at FROM events e JOIN sites s ON s.id=e.site_id WHERE s.domain=?1 AND e.request_id=?2",
            params![event.domain, hex::decode(event.page_id).expect("validated ID")],
            |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?)),
        ).optional()?;
        let Some((page, site, received)) = parent else { return Ok(Collected::PendingPageview); };
        // Match engagement's short-lived page capability; stale pages cannot
        // be used to keep an anonymous identity alive indefinitely.
        if now < received || now - received > 86_400 { return Ok(Collected::Invalid); }
        let inserted = c.execute(
            "INSERT INTO outbound_clicks(site_id,event_id,request_id,received_at,url) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(site_id,request_id) DO NOTHING",
            params![site,page,hex::decode(event.id).expect("validated ID"),now,event.url],
        )?;
        Ok(if inserted == 0 { Collected::Duplicate } else { Collected::Stored })
    }).await
}

pub const GOAL: &str = "Outbound Link: Click";

#[derive(Debug)]
pub struct Row {
    pub url: String,
    pub visitors: i64,
    pub clicks: i64,
}

#[derive(Debug, Default)]
pub struct Report {
    pub rows: Vec<Row>,
    pub total: usize,
    pub visitors: i64,
    pub clicks: i64,
    pub eligible: i64,
}

// Clicks inherit their parent page's dimensions but use their own timestamp.
// The audience also includes visitors whose pageview preceded this window.
pub(crate) fn source(
    site: i64,
    window: (i64, i64),
    query: &analytics::ReportQuery,
    clicks: bool,
    url: bool,
) -> (String, Vec<Value>) {
    let mut values: Vec<Value> = vec![site.into(), window.0.into(), window.1.into()];
    let alias = if clicks { "o" } else { "e" };
    let mut predicate =
        format!("{alias}.site_id=?1 AND {alias}.received_at>=?2 AND {alias}.received_at<?3");
    for filter in query.active_filters() {
        let (_, column, _) = crate::filters::DIMENSIONS
            .iter()
            .find(|(key, _, _)| *key == filter.key)
            .expect("validated filter");
        let table = if crate::acquisition::is_dimension(&filter.key) {
            "a"
        } else if matches!(filter.key.as_str(), "entry" | "exit") {
            "s"
        } else {
            "e"
        };
        predicate.push_str(&format!(
            " AND {}",
            filter.sql(&format!("{table}.{column}"), &mut values)
        ));
    }
    if clicks
        && url
        && let Some(url) = &query.url
    {
        values.push(url.clone().into());
        predicate.push_str(&format!(" AND o.url=?{}", values.len()));
    }
    (
        format!(
            "FROM {} JOIN sessions s ON s.id=e.session_id LEFT JOIN session_acquisition a ON a.session_id=s.id WHERE {predicate}",
            if clicks {
                "outbound_clicks o JOIN events e ON e.id=o.event_id"
            } else {
                "events e"
            }
        ),
        values,
    )
}

pub(crate) fn audience(
    site: i64,
    window: (i64, i64),
    query: &analytics::ReportQuery,
    column: &str,
) -> (String, Vec<Value>) {
    let (pages, values) = source(site, window, query, false, false);
    let (clicks, _) = source(site, window, query, true, false);
    (
        format!(
            "SELECT {column} AS name,e.visitor {pages} UNION SELECT {column} AS name,e.visitor {clicks}"
        ),
        values,
    )
}

pub(crate) fn report(
    c: &Connection,
    site: i64,
    window: (i64, i64),
    query: &analytics::ReportQuery,
) -> rusqlite::Result<Report> {
    let detail = !query.export && query.detail.as_deref() == Some("outbound");
    let (mut from, mut values) = source(site, window, query, true, true);
    let (visitors, clicks) = c.query_row(
        &format!("SELECT COUNT(DISTINCT e.visitor),COUNT(*) {from}"),
        rusqlite::params_from_iter(&values),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (audience, args) = audience(site, window, query, "''");
    let eligible = c.query_row(
        &format!("SELECT COUNT(DISTINCT visitor) FROM ({audience})"),
        rusqlite::params_from_iter(&args),
        |r| r.get(0),
    )?;
    if let Some(search) = query.search.as_deref().filter(|_| detail) {
        values.push(search.to_owned().into());
        from.push_str(&format!(
            " AND instr(lower(o.url),lower(?{}))>0",
            values.len()
        ));
    }
    from.push_str(" GROUP BY o.url");
    let total = c.query_row(
        &format!("SELECT COUNT(*) FROM (SELECT 1 {from})"),
        rusqlite::params_from_iter(&values),
        |r| r.get::<_, i64>(0),
    )?;
    let sort = match query.sort.as_deref().filter(|_| detail) {
        Some("name") => "o.url",
        Some("clicks" | "pageviews") => "clicks",
        _ => "visitors",
    };
    let direction = if detail && query.order.as_deref() == Some("asc") {
        "ASC"
    } else {
        "DESC"
    };
    let limit = if query.export {
        300
    } else if detail {
        crate::reports::PAGE_SIZE
    } else {
        9
    };
    let offset = if detail { query.offset.unwrap_or(0) } else { 0 };
    let rows=c.prepare(&format!("SELECT o.url,COUNT(DISTINCT e.visitor) AS visitors,COUNT(*) AS clicks {from} ORDER BY {sort} {direction},o.url LIMIT {limit} OFFSET {offset}"))?.query_map(rusqlite::params_from_iter(&values),|r|Ok(Row{url:r.get(0)?,visitors:r.get(1)?,clicks:r.get(2)?}))?.collect::<rusqlite::Result<_>>()?;
    Ok(Report {
        rows,
        total: total as usize,
        visitors,
        clicks,
        eligible,
    })
}
