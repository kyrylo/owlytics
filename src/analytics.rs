use crate::db::{Database, Error};
use chrono::{Datelike, NaiveDate, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use std::net::IpAddr;

#[derive(Clone, Debug)]
pub struct Site {
    pub id: i64,
    pub domain: String,
    pub timezone: String,
}

impl Site {
    pub fn today(&self) -> NaiveDate {
        self.today_at(Utc::now())
    }

    fn today_at(&self, now: chrono::DateTime<Utc>) -> NaiveDate {
        now.with_timezone(
            &self
                .timezone
                .parse::<chrono_tz::Tz>()
                .expect("validated site timezone"),
        )
        .date_naive()
    }
}

pub fn normalize_domain(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 253 || input.contains(['/', ':', '@', '?', '#', '\\']) {
        return None;
    }
    let url = url::Url::parse(&format!("https://{input}")).ok()?;
    let host = url.host_str()?.trim_end_matches('.').to_owned();
    if host.is_empty()
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
    {
        return None;
    }
    Some(host)
}

pub fn valid_timezone(value: &str) -> bool {
    value.parse::<chrono_tz::Tz>().is_ok()
}

fn site_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Site> {
    Ok(Site {
        id: row.get(0)?,
        domain: row.get(1)?,
        timezone: row.get(2)?,
    })
}

pub async fn sites(db: &Database) -> Result<Vec<Site>, Error> {
    db.read(|c| {
        c.prepare("SELECT id,domain,timezone FROM sites ORDER BY domain")?
            .query_map([], site_row)?
            .collect()
    })
    .await
}

pub async fn site(db: &Database, id: i64) -> Result<Option<Site>, Error> {
    db.read(move |c| {
        c.query_row(
            "SELECT id,domain,timezone FROM sites WHERE id=?1",
            [id],
            site_row,
        )
        .optional()
    })
    .await
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub domain: String,
    pub path: String,
    #[serde(default)]
    pub referrer: String,
    pub id: String,
    #[serde(default)]
    pub campaign: Option<crate::acquisition::Campaign>,
}

impl Event {
    pub fn validate(&mut self) -> bool {
        let Some(domain) = normalize_domain(&self.domain) else {
            return false;
        };
        self.domain = domain;
        // Queries and fragments are discarded, including from nonstandard clients.
        self.path = self.path.split(['?', '#']).next().unwrap_or("/").to_owned();
        self.path.starts_with('/')
            && !self.path.starts_with("//")
            && self.path.len() <= 2048
            && !self.path.chars().any(|c| c.is_control() || c == '\\')
            && self.id.len() == 32
            && hex::decode(&self.id).is_ok()
            && self.referrer.len() <= 2048
            && self.campaign.as_ref().is_none_or(|tags| tags.valid())
    }
}

pub fn matches_domain(host: &str, domain: &str) -> bool {
    fn bare(host: &str) -> &str {
        let host = host.trim_end_matches('.');
        if host
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("www."))
        {
            &host[4..]
        } else {
            host
        }
    }
    bare(host).eq_ignore_ascii_case(bare(domain))
}

pub fn source(referrer: &str, domain: &str) -> String {
    let Ok(url) = url::Url::parse(referrer) else {
        return String::new();
    };
    if !matches!(url.scheme(), "http" | "https" | "android-app") {
        return String::new();
    }
    let host = url.host_str().unwrap_or("").trim_end_matches('.');
    if host.is_empty() || matches_domain(host, domain) || host == "localhost" {
        String::new()
    } else {
        let host = host.strip_prefix("www.").unwrap_or(host);
        if url.scheme() == "android-app" {
            format!("android-app://{host}")
        } else {
            host.to_owned()
        }
    }
}

/// Keep the referring page for drilldowns, but never its credentials, query or fragment.
pub fn referrer(value: &str, domain: &str) -> String {
    let host = source(value, domain);
    if host.is_empty() {
        return String::new();
    }
    let Ok(url) = url::Url::parse(value) else {
        return String::new();
    };
    format!("{}{}", host, url.path().trim_end_matches('/'))
}

pub fn client_ip(
    peer: IpAddr,
    forwarded: Option<&str>,
    trusted: &[ipnet::IpNet],
) -> Option<IpAddr> {
    let normalize = |ip: IpAddr| match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        other => other,
    };
    let mut address = normalize(peer);
    if !trusted.iter().any(|network| network.contains(&address)) {
        return Some(address);
    }
    let value = forwarded?;
    if value.len() > 2048 {
        return None;
    }
    for part in value.rsplit(',').take(32) {
        if !trusted.iter().any(|network| network.contains(&address)) {
            break;
        }
        address = normalize(part.trim().parse().ok()?);
    }
    // A chain containing only proxies does not identify a visitor.
    (!trusted.iter().any(|network| network.contains(&address))).then_some(address)
}

pub fn user_agent(value: &str) -> Option<(&'static str, &'static str)> {
    if value.is_empty() || value.len() > 1024 {
        return None;
    }
    let ua = value.to_ascii_lowercase();
    if [
        "bot",
        "spider",
        "crawl",
        "headless",
        "lighthouse",
        "pagespeed",
        "curl/",
        "wget/",
        "python-",
        "preview",
        "monitor",
        "facebookexternalhit",
    ]
    .iter()
    .any(|part| ua.contains(part))
    {
        return None;
    }
    let browser = if ua.contains("edg/") || ua.contains("edga/") || ua.contains("edgios/") {
        "Edge"
    } else if ua.contains("opr/") || ua.contains("opera") {
        "Opera"
    } else if ua.contains("samsungbrowser/") {
        "Samsung Internet"
    } else if ua.contains("firefox/") || ua.contains("fxios/") {
        "Firefox"
    } else if ua.contains("chrome/") || ua.contains("crios/") {
        "Chrome"
    } else if ua.contains("safari/") {
        "Safari"
    } else {
        "Other"
    };
    let device = if ua.contains("ipad")
        || ua.contains("tablet")
        || (ua.contains("android") && !ua.contains("mobile"))
    {
        "Tablet"
    } else if ua.contains("mobile") || ua.contains("iphone") {
        "Mobile"
    } else {
        "Desktop"
    };
    Some((browser, device))
}

/// Only coarse, normalized software labels are persisted; the raw UA is discarded.
pub fn software(value: &str) -> (String, String, String) {
    let lower = value.to_ascii_lowercase();
    let version = |tokens: &[&str]| -> String {
        tokens
            .iter()
            .find_map(|token| {
                lower.split_once(token).map(|(_, rest)| {
                    rest.chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_')
                        .collect::<String>()
                        .replace('_', ".")
                })
            })
            .unwrap_or_default()
    };
    let browser_version = version(&[
        "edgios/",
        "edga/",
        "edg/",
        "opr/",
        "samsungbrowser/",
        "fxios/",
        "firefox/",
        "crios/",
        "chrome/",
        "version/",
    ]);
    let (os, os_version) = if lower.contains("iphone") || lower.contains("ipad") {
        ("iOS", version(&["cpu iphone os ", "cpu os ", "iphone os "]))
    } else if lower.contains("android") {
        ("Android", version(&["android "]))
    } else if lower.contains("windows nt") {
        let release = version(&["windows nt "]);
        (
            "Windows",
            match release.as_str() {
                "10.0" => "10",
                "6.3" => "8.1",
                "6.2" => "8",
                "6.1" => "7",
                _ => &release,
            }
            .to_owned(),
        )
    } else if lower.contains("cros ") {
        (
            "Chrome OS",
            lower
                .split_once("cros ")
                .and_then(|(_, rest)| rest.split_whitespace().nth(1))
                .unwrap_or("")
                .trim_end_matches(')')
                .to_owned(),
        )
    } else if lower.contains("mac os x") {
        ("macOS", version(&["mac os x "]))
    } else if lower.contains("ubuntu") {
        ("Ubuntu", String::new())
    } else if lower.contains("linux") {
        ("Linux", String::new())
    } else {
        ("", String::new())
    };
    (browser_version, os.to_owned(), os_version)
}

/// Discard expired identity secrets, including the previous rotation's key.
pub fn prune_salts(c: &Connection, now: i64) -> rusqlite::Result<()> {
    crate::visitor::prune(c, now)
}

pub fn source_name(host: &str) -> String {
    crate::acquisition::source_name(host)
}

pub fn ip_key(ip: IpAddr) -> (i64, Vec<u8>) {
    match ip {
        IpAddr::V4(ip) => (4, ip.octets().to_vec()),
        IpAddr::V6(ip) => (6, ip.octets().to_vec()),
    }
}

pub fn lookup_country(c: &Connection, ip: IpAddr) -> rusqlite::Result<String> {
    let (family, key) = ip_key(ip);
    // Nonoverlapping ranges: find the nearest start, then test its end.
    let candidate = c.query_row("SELECT end,country FROM geo_ranges WHERE family=?1 AND start<=?2 ORDER BY start DESC LIMIT 1", params![family, key], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))).optional()?;
    Ok(candidate
        .filter(|(end, _)| &key <= end)
        .map(|(_, country)| country)
        .unwrap_or_default())
}

#[derive(Debug, PartialEq)]
pub enum Collected {
    Stored,
    Duplicate,
    UnknownSite,
    Invalid,
}

pub async fn collect(
    db: &Database,
    mut event: Event,
    ip: IpAddr,
    ua: String,
    clock: impl FnOnce() -> i64 + Send + 'static,
) -> Result<Collected, Error> {
    if !event.validate() {
        return Ok(Collected::Invalid);
    }
    let Some((browser, device)) = user_agent(&ua) else {
        return Ok(Collected::Duplicate);
    };
    let (browser_version, os, os_version) = software(&ua);
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| Error::Worker)?;
    db.call(move |c| {
        // Sample inside the serialized writer. Requests queued across midnight
        // must not rotate a site's salt backwards after a newer day was stored.
        let now = clock();
        let tx = c.transaction()?;
        let Some(site) = tx.query_row("SELECT id,domain,timezone FROM sites WHERE domain=?1", [&event.domain], site_row).optional()? else { return Ok(Collected::UnknownSite); };
        let zone = site.timezone.parse::<chrono_tz::Tz>().expect("validated site timezone");
        let instant = chrono::DateTime::from_timestamp(now, 0).expect("current timestamp");
        let date = instant.with_timezone(&zone).date_naive();
        let day = date.to_string();
        let visitor = crate::visitor::identify(&tx, site.id, now, &random, ip, &ua)?;
        let country = lookup_country(&tx, ip)?;
        let (family, key) = ip_key(ip);
        let (region,city) = tx.query_row("SELECT l.region,l.city FROM geo_ranges r JOIN geo_locations l ON l.family=r.family AND l.start=r.start WHERE r.family=?1 AND r.start=(SELECT start FROM geo_ranges WHERE family=?1 AND start<=?2 ORDER BY start DESC LIMIT 1) AND r.end>=?2", params![family,key], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?.unwrap_or_default();
        let referring_source = source(&event.referrer, &site.domain);
        let attribution = event.campaign.as_ref().map(|tags| crate::acquisition::Attribution::resolve(&referring_source, tags));
        let attributed_source = attribution.as_ref().map(|a| a.source.clone()).unwrap_or_else(|| source_name(&referring_source));
        let changed = tx.prepare_cached("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device,browser_version,os,os_version,region,city,referrer) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16) ON CONFLICT(site_id,request_id) DO NOTHING")?.execute(params![site.id, now, day, &visitor[..16], hex::decode(&event.id).expect("validated event id"), event.path, attributed_source, country, browser, device, browser_version,os,os_version,region,city,referrer(&event.referrer,&site.domain)])?;
        if changed == 1 {
            if let Some(a) = attribution {
                // Existing visits retain their first touch, including visits
                // started with an older tracker whose campaign data is unknown.
                tx.execute("INSERT INTO session_acquisition(session_id,channel,utm_source,utm_medium,utm_campaign,utm_content,utm_term,click_id) SELECT e.session_id,?3,?4,?5,?6,?7,?8,?9 FROM events e WHERE e.site_id=?1 AND e.request_id=?2 AND e.session_id=e.id", params![site.id,hex::decode(&event.id).expect("validated event id"),a.channel,a.utm_source,a.utm_medium,a.utm_campaign,a.utm_content,a.utm_term,a.click_id])?;
            }
        }
        tx.commit()?;
        Ok(if changed == 1 { Collected::Stored } else { Collected::Duplicate })
    }).await
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    #[default]
    Visitors,
    Pageviews,
    Visits,
    #[serde(rename = "views_per_visit")]
    ViewsPerVisit,
    #[serde(rename = "bounce_rate")]
    BounceRate,
    #[serde(rename = "visit_duration")]
    VisitDuration,
    #[serde(rename = "time_on_page")]
    TimeOnPage,
    #[serde(rename = "scroll_depth")]
    ScrollDepth,
}

impl Metric {
    pub fn name(self) -> &'static str {
        match self {
            Self::Visitors => "Visitors",
            Self::Pageviews => "Pageviews",
            Self::Visits => "Total visits",
            Self::ViewsPerVisit => "Views per visit",
            Self::BounceRate => "Bounce rate",
            Self::VisitDuration => "Visit duration",
            Self::TimeOnPage => "Time on page",
            Self::ScrollDepth => "Scroll depth",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Visitors => "visitors",
            Self::Pageviews => "pageviews",
            Self::Visits => "visits",
            Self::ViewsPerVisit => "views_per_visit",
            Self::BounceRate => "bounce_rate",
            Self::VisitDuration => "visit_duration",
            Self::TimeOnPage => "time_on_page",
            Self::ScrollDepth => "scroll_depth",
        }
    }
    pub fn value(self, row: &Row) -> f64 {
        match self {
            Self::Visitors => row.visitors as f64,
            Self::Pageviews => row.pageviews as f64,
            Self::Visits => row.visits as f64,
            Self::TimeOnPage => {
                if row.time_on_page_visits > 0 {
                    row.time_on_page_ms as f64 / row.time_on_page_visits as f64 / 1000.
                } else {
                    0.
                }
            }
            Self::ScrollDepth => {
                if row.scroll_depth_visits > 0 {
                    row.total_scroll_depth as f64 / row.scroll_depth_visits as f64
                } else {
                    0.
                }
            }
            Self::ViewsPerVisit => {
                if row.mean_visits() > 0 {
                    row.mean_pageviews() as f64 / row.mean_visits() as f64
                } else {
                    0.
                }
            }
            Self::BounceRate => {
                let entrances = row.entrances.unwrap_or(row.mean_visits());
                if entrances > 0 {
                    row.bounces as f64 * 100. / entrances as f64
                } else {
                    0.
                }
            }
            Self::VisitDuration => {
                if row.mean_visits() > 0 {
                    row.duration as f64 / row.mean_visits() as f64
                } else {
                    0.
                }
            }
        }
    }

    pub fn count(self, row: &Row) -> i64 {
        match self {
            Self::Visitors => row.visitors,
            Self::Pageviews => row.pageviews,
            _ => self.value(row).round() as i64,
        }
    }
}

/// Internal report work selection; URL parameters cannot change it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReportScope {
    #[default]
    All,
    Dashboard,
    Timeline,
    Suggestions,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct ReportQuery {
    #[serde(default, deserialize_with = "crate::filters::from_query")]
    pub filters: Vec<crate::filters::Filter>,
    pub filter: Option<String>,
    pub filter_mode: Option<String>,
    pub filter_op: Option<crate::filters::Operator>,
    pub filter_search: Option<String>,
    pub filter_offset: Option<usize>,
    #[serde(default)]
    pub metric: Metric,
    pub date: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub page: Option<String>,
    pub source: Option<String>,
    pub referrer: Option<String>,
    pub country: Option<String>,
    pub browser: Option<String>,
    pub device: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub browser_version: Option<String>,
    pub entry: Option<String>,
    pub exit: Option<String>,
    pub period: Option<String>,
    pub compare: Option<String>,
    pub compare_from: Option<String>,
    pub compare_to: Option<String>,
    pub compare_match: Option<String>,
    pub interval: Option<String>,
    pub pages_tab: Option<String>,
    pub locations_tab: Option<String>,
    pub devices_tab: Option<String>,
    pub sources_tab: Option<String>,
    pub with_imported: Option<bool>,
    pub detail: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub search: Option<String>,
    pub offset: Option<usize>,
    #[serde(skip)]
    pub export: bool,
    #[serde(skip)]
    pub window: Option<(i64, i64)>,
    #[serde(skip)]
    pub scope: ReportScope,
    /// Internal comparison lookup: names from the primary report, before any
    /// ranking/search in the comparison period. Never accepted from a URL.
    #[serde(skip)]
    pub comparison_names: Option<std::collections::BTreeMap<String, Vec<String>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl ReportQuery {
    pub(crate) fn imports_enabled(&self) -> bool {
        self.with_imported.unwrap_or(true)
    }

    pub(crate) fn source_tab(&self) -> &'static str {
        match self.sources_tab.as_deref() {
            Some("source") => "source",
            Some("referrer") => "referrer",
            Some(key) if crate::acquisition::is_dimension(key) => {
                crate::acquisition::DIMENSIONS
                    .iter()
                    .find(|(name, _)| *name == key)
                    .unwrap()
                    .0
            }
            _ if self.exact_filter("source").is_some() => "referrer",
            _ => "source",
        }
    }
    pub(crate) fn legacy_filter(&self, key: &str) -> Option<&str> {
        match key {
            "page" => &self.page,
            "source" => &self.source,
            "referrer" => &self.referrer,
            "country" => &self.country,
            "region" => &self.region,
            "city" => &self.city,
            "browser" => &self.browser,
            "browser_version" => &self.browser_version,
            "os" => &self.os,
            "os_version" => &self.os_version,
            "device" => &self.device,
            "entry" => &self.entry,
            "exit" => &self.exit,
            _ => return None,
        }
        .as_deref()
    }
    pub(crate) fn has_filter(&self, key: &str) -> bool {
        self.legacy_filter(key).is_some() || self.filters.iter().any(|filter| filter.key == key)
    }
    pub(crate) fn exact_filter(&self, key: &str) -> Option<&str> {
        self.filters
            .iter()
            .find(|filter| filter.key == key)
            .map_or_else(|| self.legacy_filter(key), |filter| filter.single())
    }
    pub(crate) fn matches_filter(&self, key: &str, value: &str) -> bool {
        self.filters
            .iter()
            .find(|filter| filter.key == key)
            .map_or_else(
                || {
                    self.legacy_filter(key)
                        .is_none_or(|candidate| candidate == value)
                },
                |filter| filter.matches(value),
            )
    }
    pub(crate) fn active_filters(&self) -> Vec<crate::filters::Filter> {
        crate::filters::DIMENSIONS
            .iter()
            .filter_map(|(key, _, _)| {
                self.legacy_filter(key).map(|value| crate::filters::Filter {
                    key: (*key).into(),
                    op: crate::filters::Operator::Is,
                    values: vec![value.into()],
                })
            })
            .chain(self.filters.iter().cloned())
            .collect()
    }
    pub(crate) fn normalize_locations(&mut self) {
        for (key, field) in [("region", &mut self.region), ("city", &mut self.city)] {
            if let Some(value) = field {
                *value = crate::geography::location_key(key, value);
            }
        }
        for filter in &mut self.filters {
            if matches!(filter.key.as_str(), "region" | "city") {
                for value in &mut filter.values {
                    *value = crate::geography::location_key(&filter.key, value);
                }
                filter.values.sort();
                filter.values.dedup();
            }
        }
    }
    pub(crate) fn remove_filter(&mut self, key: &str) {
        self.filters.retain(|filter| filter.key != key);
        let field = match key {
            "page" => &mut self.page,
            "source" => &mut self.source,
            "referrer" => &mut self.referrer,
            "country" => &mut self.country,
            "region" => &mut self.region,
            "city" => &mut self.city,
            "browser" => &mut self.browser,
            "browser_version" => &mut self.browser_version,
            "os" => &mut self.os,
            "os_version" => &mut self.os_version,
            "device" => &mut self.device,
            "entry" => &mut self.entry,
            "exit" => &mut self.exit,
            _ => return,
        };
        *field = None;
    }
    pub(crate) fn without_filters(&self) -> Self {
        let mut query = Self {
            filters: Vec::new(),
            filter: None,
            filter_mode: None,
            filter_op: None,
            filter_search: None,
            filter_offset: None,
            page: None,
            source: None,
            referrer: None,
            country: None,
            region: None,
            city: None,
            browser: None,
            browser_version: None,
            os: None,
            os_version: None,
            device: None,
            entry: None,
            exit: None,
            offset: None,
            ..self.clone()
        };
        query.metric = match query.metric {
            Metric::ScrollDepth => Metric::ViewsPerVisit,
            Metric::TimeOnPage => Metric::VisitDuration,
            metric => metric,
        };
        for field in [&mut query.detail, &mut query.devices_tab] {
            match field.as_deref() {
                Some("browser_version") => *field = Some("browser".into()),
                Some("os_version") => *field = Some("os".into()),
                _ => {}
            }
        }
        if self.has_filter("country") || self.has_filter("region") || self.has_filter("city") {
            query.locations_tab = Some("map".into());
        }
        if self.has_filter("source") && query.sources_tab.as_deref() == Some("referrer") {
            query.sources_tab = Some("source".into());
        }
        query
    }

    pub(crate) fn custom_comparison(&self) -> Result<Option<Range>, &'static str> {
        if self.compare.as_deref() != Some("custom") {
            return Ok(None);
        }
        let parse = |value: &Option<String>| {
            value
                .as_ref()
                .filter(|value| value.len() == 10)
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                .filter(|date| (1..=9999).contains(&date.year()))
                .ok_or("Choose both comparison dates in YYYY-MM-DD format.")
        };
        let from = parse(&self.compare_from)?;
        let to = parse(&self.compare_to)?;
        if from > to || (to - from).num_days() > 3660 {
            return Err("Choose a comparison range of up to ten years.");
        }
        Ok(Some(Range { from, to }))
    }

    pub(crate) fn location_tab(&self) -> &str {
        match self.locations_tab.as_deref() {
            Some(tab @ ("map" | "country" | "region" | "city")) => tab,
            _ if self.exact_filter("city").is_some() || self.exact_filter("region").is_some() => {
                "city"
            }
            _ if self.exact_filter("country").is_some() => "region",
            _ => "map",
        }
    }

    pub(crate) fn validate_filters(&self) -> Result<(), &'static str> {
        let mut keys = std::collections::BTreeSet::new();
        let mut bytes = 0;
        for filter in self.active_filters() {
            filter.validate()?;
            if !keys.insert(filter.key) {
                return Err("Choose one filter per dimension.");
            }
            bytes += filter.values.iter().map(String::len).sum::<usize>();
        }
        if bytes > 8192
            || serde_json::to_string(&self.filters)
                .expect("serializable filters")
                .len()
                > 8192
        {
            return Err("Filters are too long.");
        }
        Ok(())
    }

    pub fn range(&self, today: NaiveDate) -> Result<Range, &'static str> {
        self.validate_filters()?;
        if let (Some(key), Some(operator)) = (&self.filter, self.filter_op)
            && !crate::filters::operators(key).contains(&operator)
        {
            return Err("Choose a valid operator for this filter.");
        }
        if self.filter.as_deref().is_some_and(|key| {
            !crate::filters::DIMENSIONS
                .iter()
                .any(|(name, _, _)| *name == key)
        }) || self
            .filter_mode
            .as_deref()
            .is_some_and(|mode| !matches!(mode, "values" | "operator"))
            || self.filter_offset.is_some_and(|offset| offset > 1_000_000)
            || self
                .filter_search
                .as_deref()
                .is_some_and(|value| value.len() > 2048 || value.chars().any(char::is_control))
        {
            return Err("Invalid filter options.");
        }
        let parse = |value: &Option<String>, default| match value {
            Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .filter(|date| value.len() == 10 && (1..=9999).contains(&date.year()))
                .ok_or("Use dates in YYYY-MM-DD format."),
            None => Ok(default),
        };
        let default_to = if self.from.is_none() && self.to.is_none() {
            today.pred_opt().ok_or("Invalid date")?
        } else {
            today
        };
        let mut to = parse(&self.to, default_to)?;
        let default_from = to
            .checked_sub_signed(chrono::Duration::days(27))
            .unwrap_or(to);
        let mut from = parse(&self.from, default_from)?;
        let anchor = parse(&self.date, today)?;
        if let Some(period) = self.period.as_deref().or(self.date.as_ref().map(|_| "day")) {
            to = today;
            from = match period {
                "day" => {
                    to = anchor;
                    anchor
                }
                "yesterday" => {
                    to = today.pred_opt().ok_or("Invalid date")?;
                    to
                }
                "realtime" | "24h" => today.pred_opt().ok_or("Invalid date")?,
                "7d" | "28d" | "30d" | "91d" => {
                    to = today.pred_opt().ok_or("Invalid date")?;
                    let days = match period {
                        "7d" => 7,
                        "28d" => 28,
                        "30d" => 30,
                        _ => 91,
                    };
                    today - chrono::Duration::days(days)
                }
                "month" => {
                    let start = anchor.with_day(1).unwrap();
                    if start != today.with_day(1).unwrap() {
                        to = start
                            .checked_add_months(chrono::Months::new(1))
                            .and_then(|date| date.pred_opt())
                            .ok_or("Invalid date")?;
                    }
                    start
                }
                "lastmonth" => {
                    to = today
                        .with_day(1)
                        .unwrap()
                        .pred_opt()
                        .ok_or("Invalid date")?;
                    to.with_day(1).unwrap()
                }
                "year" => {
                    let start = anchor.with_ordinal(1).unwrap();
                    if anchor.year() != today.year() {
                        to = start
                            .checked_add_months(chrono::Months::new(12))
                            .and_then(|date| date.pred_opt())
                            .ok_or("Invalid date")?;
                    }
                    start
                }
                "6mo" | "12mo" => {
                    let month = today.with_day(1).unwrap();
                    to = month.pred_opt().ok_or("Invalid date")?;
                    month
                        .checked_sub_months(chrono::Months::new(if period == "6mo" {
                            6
                        } else {
                            12
                        }))
                        .ok_or("Invalid date")?
                }
                "all" => today - chrono::Duration::days(3660),
                _ => return Err("Invalid period."),
            };
        }
        if self
            .interval
            .as_deref()
            .is_some_and(|v| !matches!(v, "minute" | "hour" | "day" | "week" | "month"))
        {
            return Err("Invalid chart interval.");
        }
        if self
            .compare
            .as_deref()
            .is_some_and(|v| !matches!(v, "off" | "previous" | "year" | "custom"))
            || self
                .compare_match
                .as_deref()
                .is_some_and(|v| !matches!(v, "date" | "weekday"))
        {
            return Err("Invalid comparison.");
        }
        self.custom_comparison()?;
        if from.year() < 1 {
            return Err("Use dates in YYYY-MM-DD format.");
        }
        if self.sort.as_deref().is_some_and(|v| {
            !matches!(
                v,
                "name"
                    | "percentage"
                    | "visitors"
                    | "pageviews"
                    | "visits"
                    | "bounce_rate"
                    | "visit_duration"
                    | "time_on_page"
                    | "scroll_depth"
                    | "exit_rate"
            )
        }) || self
            .order
            .as_deref()
            .is_some_and(|v| !matches!(v, "asc" | "desc"))
            || self.offset.is_some_and(|v| v > 1_000_000)
            || self
                .search
                .as_ref()
                .is_some_and(|v| v.len() > 2048 || v.chars().any(char::is_control))
            || self.detail.as_deref().is_some_and(|v| {
                !crate::acquisition::is_dimension(v)
                    && !matches!(
                        v,
                        "page"
                            | "entry"
                            | "exit"
                            | "source"
                            | "referrer"
                            | "country"
                            | "region"
                            | "city"
                            | "browser"
                            | "browser_version"
                            | "os"
                            | "os_version"
                            | "device"
                    )
            })
        {
            return Err("Invalid report options.");
        }
        if from > to || (to - from).num_days() > 3660 {
            return Err("Choose a date range of up to ten years.");
        }
        if [
            &self.page,
            &self.source,
            &self.referrer,
            &self.country,
            &self.browser,
            &self.device,
            &self.region,
            &self.city,
            &self.os,
            &self.os_version,
            &self.browser_version,
            &self.entry,
            &self.exit,
        ]
        .iter()
        .any(|v| {
            v.as_ref()
                .is_some_and(|v| v.len() > 2048 || v.chars().any(char::is_control))
        }) {
            return Err("Invalid filter.");
        }
        Ok(Range { from, to })
    }

    pub fn filtered(&self) -> bool {
        !self.filters.is_empty()
            || [
                &self.page,
                &self.source,
                &self.referrer,
                &self.country,
                &self.browser,
                &self.device,
                &self.region,
                &self.city,
                &self.os,
                &self.os_version,
                &self.browser_version,
                &self.entry,
                &self.exit,
            ]
            .iter()
            .any(|v| v.is_some())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub visitors: i64,
    pub pageviews: i64,
    pub visits: i64,
    pub bounces: i64,
    pub duration: i64,
    /// Means use the complete overlapping visits, while traffic counts use
    /// pageviews inside the requested window. None means both populations match
    /// (including historical aggregates that do not distinguish them).
    pub session_visits: Option<i64>,
    pub session_pageviews: Option<i64>,
    pub exits: i64,
    pub time_on_page_ms: i64,
    pub time_on_page_visits: i64,
    pub total_scroll_depth: i64,
    pub scroll_depth_visits: i64,
    /// Page bounce rates use entrances, while page visits count every session
    /// that viewed the page. Page filters apply this denominator to every
    /// breakdown and time bucket; unfiltered metrics use visits.
    pub entrances: Option<i64>,
    /// Numerator and denominator for the displayed traffic share. These use
    /// pageviews for legacy imports and visitors otherwise; never persisted.
    pub percentage: Option<(i64, i64)>,
    /// Missing historical aggregates: visitors, visits, bounces, duration, exits,
    /// time on page, scroll depth (bits 1 through 64).
    /// Native rows have all aggregates; never present missing history as zero.
    pub unavailable: u8,
}

impl Row {
    pub fn mean_visits(&self) -> i64 {
        self.session_visits.unwrap_or(self.visits)
    }
    pub fn mean_pageviews(&self) -> i64 {
        self.session_pageviews.unwrap_or(self.pageviews)
    }
    pub(crate) fn add(&mut self, other: &Row) {
        // Resolve fallback counts before adding traffic. Preserve explicit zero
        // denominators; they are not missing measurements.
        if self.session_visits.is_some() || other.session_visits.is_some() {
            self.session_visits = Some(self.mean_visits() + other.mean_visits());
        }
        if self.session_pageviews.is_some() || other.session_pageviews.is_some() {
            self.session_pageviews = Some(self.mean_pageviews() + other.mean_pageviews());
        }
        self.visitors += other.visitors;
        self.pageviews += other.pageviews;
        self.visits += other.visits;
        self.bounces += other.bounces;
        self.duration += other.duration;
        self.exits += other.exits;
        self.unavailable |= other.unavailable;
        if let Some(entrances) = other.entrances {
            *self.entrances.get_or_insert(0) += entrances;
        }
        self.add_engagement(other);
    }
    pub(crate) fn add_engagement(&mut self, other: &Row) {
        self.time_on_page_ms += other.time_on_page_ms;
        self.time_on_page_visits += other.time_on_page_visits;
        self.total_scroll_depth += other.total_scroll_depth;
        self.scroll_depth_visits += other.scroll_depth_visits;
        self.unavailable |= other.unavailable & (32 | 64);
    }
    pub fn available(&self, metric: Metric) -> bool {
        if metric == Metric::TimeOnPage && self.time_on_page_visits == 0 {
            return false;
        }
        if metric == Metric::ScrollDepth && self.scroll_depth_visits == 0 {
            return false;
        }
        let fields = match metric {
            Metric::Visitors => 1,
            Metric::Pageviews => 0,
            Metric::Visits | Metric::ViewsPerVisit => 2,
            Metric::BounceRate => 2 | 4,
            Metric::VisitDuration => 2 | 8,
            Metric::TimeOnPage => 32,
            Metric::ScrollDepth => 64,
        };
        self.unavailable & fields == 0
    }
}

pub struct SiteOverview {
    pub site: Site,
    pub hourly: [i64; 24],
    pub visitors: i64,
    pub previous_visitors: i64,
}

/// Rolling 24-hour summaries use the timestamp/visitor covering index. Distinct
/// visitors are counted over the whole window, never by adding hourly uniques.
/// Historical daily imports cannot supply hourly traffic and stay in reports.
pub async fn site_overviews(
    db: &Database,
    now: chrono::DateTime<Utc>,
) -> Result<Vec<SiteOverview>, Error> {
    let end = now.timestamp() + 1;
    let start = end - 86400;
    db.read(move |c| {
        let tx = c.transaction()?;
        let sites = tx
            .prepare("SELECT id,domain,timezone FROM sites ORDER BY domain")?
            .query_map([], site_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut result = Vec::with_capacity(sites.len());
        let mut statement = tx.prepare(
            "WITH recent AS MATERIALIZED (
                SELECT visitor,received_at>=?2 AS current,(received_at-?2)/3600 AS hour
                FROM events WHERE site_id=?1 AND received_at>=?2-86400 AND received_at<?3
             )
             SELECT current,-1,COUNT(DISTINCT visitor) FROM recent GROUP BY current
             UNION ALL
             SELECT 1,hour,COUNT(DISTINCT visitor) FROM recent WHERE current=1 GROUP BY hour",
        )?;
        for site in sites {
            let mut overview = SiteOverview {
                site,
                hourly: [0; 24],
                visitors: 0,
                previous_visitors: 0,
            };
            let mut rows = statement.query(params![overview.site.id, start, end])?;
            while let Some(row) = rows.next()? {
                let current: bool = row.get(0)?;
                let hour: i64 = row.get(1)?;
                let visitors = row.get(2)?;
                if hour == -1 {
                    if current {
                        overview.visitors = visitors;
                    } else {
                        overview.previous_visitors = visitors;
                    }
                } else if let Some(bucket) = overview.hourly.get_mut(hour as usize) {
                    *bucket = visitors;
                }
            }
            result.push(overview);
        }
        drop(statement);
        tx.commit()?;
        Ok(result)
    })
    .await
}

#[derive(Debug, Default)]
pub struct Report {
    /// Chart observation time; rolling requests retain their window's clock.
    pub generated_at: i64,
    pub realtime: bool,
    pub visitors: i64,
    pub pageviews: i64,
    pub daily: Vec<Row>,
    /// Exact rows for the requested calendar interval. Native identities are
    /// deduplicated inside the interval before historical counters are added.
    pub calendar: Option<Vec<Row>>,
    pub pages: Vec<Row>,
    pub sources: Vec<Row>,
    pub referrers: Vec<Row>,
    pub acquisition: std::collections::BTreeMap<String, Vec<Row>>,
    pub acquisition_incomplete: std::collections::BTreeSet<String>,
    pub countries: Vec<Row>,
    pub browsers: Vec<Row>,
    pub devices: Vec<Row>,
    pub regions: Vec<Row>,
    pub cities: Vec<Row>,
    pub operating_systems: Vec<Row>,
    pub browser_versions: Vec<Row>,
    pub os_versions: Vec<Row>,
    pub entry_pages: Vec<Row>,
    pub exit_pages: Vec<Row>,
    pub totals: Row,
    pub current_visitors: i64,
    pub timeline: Vec<crate::timeline::Bucket>,
    pub timeline_start: i64,
    pub timeline_end: i64,
    pub timeline_step: i64,
    pub session_metrics_available: bool,
    pub geo_enabled: bool,
    pub breakdown_pageviews: bool,
    pub imports_excluded: bool,
    pub imports_excluded_dimensions: std::collections::BTreeSet<String>,
    pub has_imports: bool,
    pub imports_available: bool,
    pub imports_disabled: bool,
    pub has_data: bool,
    pub detail_total: usize,
    /// A missing comparison row is unknown when historical dimension coverage
    /// is incomplete. A covered, absent row is a genuine zero.
    pub comparison_missing: std::collections::BTreeSet<String>,
}

impl Report {
    pub fn rows(&self, key: &str) -> &[Row] {
        match key {
            "page" => &self.pages,
            "source" => &self.sources,
            "referrer" => &self.referrers,
            "country" => &self.countries,
            "region" => &self.regions,
            "city" => &self.cities,
            "browser" => &self.browsers,
            "browser_version" => &self.browser_versions,
            "os" => &self.operating_systems,
            "os_version" => &self.os_versions,
            "device" => &self.devices,
            "entry" => &self.entry_pages,
            "exit" => &self.exit_pages,
            other => self.acquisition_rows(other),
        }
    }

    pub(crate) fn comparison_names(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        crate::reports::DIMENSIONS
            .iter()
            .map(|(key, _)| *key)
            .chain(["entry", "exit"])
            .filter_map(|key| {
                let rows = self.rows(key);
                (!rows.is_empty()).then(|| {
                    (
                        key.to_owned(),
                        rows.iter().map(|row| row.name.clone()).collect(),
                    )
                })
            })
            .collect()
    }

    pub fn acquisition_rows(&self, key: &str) -> &[Row] {
        self.acquisition
            .get(key)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

pub async fn report(
    db: &Database,
    site_id: i64,
    range: Range,
    mut query: ReportQuery,
) -> Result<Report, Error> {
    query.normalize_locations();
    crate::reports::report(db, site_id, range, query).await
}
