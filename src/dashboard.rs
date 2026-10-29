use crate::{
    analytics::{self, Metric, Range, ReportQuery, Row, Site},
    auth,
    icons::report as icon,
    web::{App, Session, escape, session, shell, unavailable},
};
use axum::{
    Form,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::fmt::Write;

async fn owner(app: &App, headers: &HeaderMap) -> Result<Session, Response> {
    match session(app, headers).await {
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(Redirect::to("/login").into_response()),
        Err(_) => Err(unavailable()),
    }
}

fn nav(session: &Session) -> String {
    format!(
        "<header class=\"nav\"><div class=\"nav-inner\"><a class=\"brand\" href=\"/\"><img src=\"/assets/logo.svg\" width=\"38\" height=\"38\" alt=\"\">Owlytics</a><div class=\"nav-actions\"><a class=\"quiet\" href=\"/settings\">Settings</a><form method=\"post\" action=\"/logout\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><button class=\"quiet\">Sign out</button></form></div></div></header>",
        escape(&session.csrf)
    )
}

pub(crate) fn page(title: &str, session: &Session, body: &str) -> Response {
    Html(shell(
        title,
        &format!("{}<main class=\"container\">{body}</main>", nav(session)),
    ))
    .into_response()
}

pub(crate) async fn home(State(app): State<App>, headers: HeaderMap) -> Response {
    let session = match owner(&app, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let sites = match analytics::site_overviews(&app.db, chrono::Utc::now()).await {
        Ok(sites) => sites,
        Err(_) => return unavailable(),
    };
    let mut body = String::from(
        "<div class=\"heading home-heading\"><h1>Websites</h1><a class=\"button button-primary\" href=\"/sites/new\"><span aria-hidden=\"true\">+</span> Add website</a></div>",
    );
    if sites.is_empty() {
        body.push_str("<section class=\"empty\"><img src=\"/assets/owl.svg\" width=\"70\" height=\"100\" alt=\"\"><h2>No websites yet</h2></section>");
    } else {
        body.push_str("<svg width=\"0\" height=\"0\" aria-hidden=\"true\" class=\"sr-only\"><defs><linearGradient id=\"spark-gradient\" x2=\"0\" y2=\"1\"><stop stop-color=\"white\" stop-opacity=\".16\"/><stop offset=\"1\" stop-color=\"white\" stop-opacity=\"0\"/></linearGradient><mask id=\"spark-fade\" maskContentUnits=\"objectBoundingBox\"><rect width=\"1\" height=\"1\" fill=\"url(#spark-gradient)\"/></mask></defs></svg><div class=\"site-grid\">");
        for overview in sites {
            let site = &overview.site;
            let trend = if overview.previous_visitors > 0 {
                let change =
                    (overview.visitors as f64 / overview.previous_visitors as f64 - 1.) * 100.;
                let (class, arrow) = if change >= 0.5 {
                    ("up", "↗")
                } else if change <= -0.5 {
                    ("down", "↘")
                } else {
                    ("steady", "→")
                };
                format!(
                    "<span class=\"trend {class}\" title=\"Compared with the previous 24 hours\"><span class=\"trend-arrow\" aria-hidden=\"true\">{arrow}</span> {:.0}%<span class=\"sr-only\"> {} compared with the previous 24 hours</span></span>",
                    change.abs(),
                    if change >= 0.5 {
                        "increase"
                    } else if change <= -0.5 {
                        "decrease"
                    } else {
                        "change"
                    }
                )
            } else {
                String::from(
                    "<span class=\"trend steady\" title=\"No previous visitors to compare\">—</span>",
                )
            };
            let peak = *overview.hourly.iter().max().unwrap_or(&0).max(&1) as f64;
            let points = overview
                .hourly
                .iter()
                .enumerate()
                .map(|(i, value)| {
                    format!(
                        "{:.2},{:.2}",
                        i as f64 / 23. * 1000.,
                        58. - *value as f64 / peak * 54.
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            let _ = write!(
                body,
                "<article class=\"site-card\"><a class=\"site-card-main\" href=\"/sites/{}?period=24h\"><div class=\"site-card-heading\">{}<h2>{}</h2></div><svg class=\"site-sparkline\" viewBox=\"0 0 1000 60\" preserveAspectRatio=\"none\" role=\"img\" aria-label=\"Hourly visitors in the last 24 hours\"><polygon class=\"spark-area\" points=\"0,60 {points} 1000,60\"/><polyline points=\"{points}\"/></svg><div class=\"site-card-footer\"><div class=\"site-visitors\"><strong title=\"{} visitors\">{}</strong><span>visitors in last 24h</span></div>{trend}</div></a><a class=\"site-settings\" href=\"/sites/{}/settings\" aria-label=\"Settings for {}\">⋮</a></article>",
                site.id,
                crate::icons::site(site.id, &site.domain),
                escape(&site.domain),
                number(overview.visitors),
                compact_number(overview.visitors),
                site.id,
                escape(&site.domain)
            );
        }
        body.push_str("</div>");
    }
    page("Websites", &session, &body)
}

fn compact_number(value: i64) -> String {
    for (threshold, suffix) in [(1_000_000_000, "b"), (1_000_000, "m"), (1_000, "k")] {
        if value >= threshold - threshold / 20_000 {
            return format!("{:.1}", value as f64 / threshold as f64)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
                + suffix;
        }
    }
    value.to_string()
}

fn site_form(session: &Session, domain: &str, timezone: &str, error: Option<&str>) -> Response {
    let error = error
        .map(|e| format!("<p class=\"error\" role=\"alert\">{}</p>", escape(e)))
        .unwrap_or_default();
    page(
        "Add website",
        session,
        &format!(
            "<a class=\"back\" href=\"/\">← Websites</a><section class=\"form-card\"><h1>Add website</h1>{error}<form method=\"post\" action=\"/sites\"><input type=\"hidden\" name=\"csrf\" value=\"{}\"><label for=\"domain\">Domain</label><input id=\"domain\" name=\"domain\" placeholder=\"example.com\" value=\"{}\" required maxlength=\"253\" autocapitalize=\"none\" spellcheck=\"false\"><label for=\"timezone\">Timezone</label><input id=\"timezone\" name=\"timezone\" value=\"{}\" list=\"timezones\" required><datalist id=\"timezones\"><option value=\"UTC\"><option value=\"Asia/Manila\"><option value=\"Europe/London\"><option value=\"America/New_York\"><option value=\"America/Los_Angeles\"></datalist><button class=\"primary\">Add website <span aria-hidden=\"true\">→</span></button></form></section>",
            escape(&session.csrf),
            escape(domain),
            escape(timezone)
        ),
    )
}

pub(crate) async fn new_site(State(app): State<App>, headers: HeaderMap) -> Response {
    match owner(&app, &headers).await {
        Ok(session) => site_form(&session, "", "UTC", None),
        Err(response) => response,
    }
}

#[derive(Deserialize)]
pub(crate) struct SiteForm {
    csrf: String,
    domain: String,
    timezone: String,
}

pub(crate) async fn create(
    State(app): State<App>,
    headers: HeaderMap,
    Form(form): Form<SiteForm>,
) -> Response {
    let session = match owner(&app, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    if !auth::same_origin(&headers, &app.origin())
        || !auth::matches_token(&session.csrf, &form.csrf)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let domain = analytics::normalize_domain(&form.domain);
    if domain.is_none() || !analytics::valid_timezone(&form.timezone) {
        let mut response = site_form(
            &session,
            &form.domain,
            &form.timezone,
            Some("Enter a domain and an IANA timezone, such as Asia/Manila."),
        );
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return response;
    }
    let timezone = form.timezone.clone();
    let result = app
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO sites(domain,timezone,created_at) VALUES(?1,?2,?3)",
                rusqlite::params![domain.unwrap(), timezone, auth::now()],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await;
    match result {
        Ok(id) => Redirect::to(&format!("/sites/{id}/settings")).into_response(),
        Err(crate::db::Error::Sqlite(rusqlite::Error::SqliteFailure(error, _)))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            let mut response = site_form(
                &session,
                &form.domain,
                &form.timezone,
                Some("That website is already here."),
            );
            *response.status_mut() = StatusCode::CONFLICT;
            response
        }
        Err(_) => unavailable(),
    }
}

pub(crate) async fn settings(
    State(app): State<App>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Response {
    let session = match owner(&app, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let site = match analytics::site(&app.db, id).await {
        Ok(Some(site)) => site,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return unavailable(),
    };
    let snippet = format!(
        "<script defer data-domain=\"{}\" src=\"{}/script.js\"></script>",
        site.domain,
        app.origin()
    );
    page(
        &site.domain,
        &session,
        &format!(
            "<a class=\"back\" href=\"/sites/{id}\">← {}</a><section class=\"form-card wide\"><h1>Install</h1><p class=\"muted\">Add this inside your website’s &lt;head&gt;.</p><pre><code>{}</code></pre><p class=\"muted small\">Timezone: {}</p><a class=\"button\" href=\"/sites/{id}\">Open dashboard →</a></section>",
            escape(&site.domain),
            escape(&snippet),
            escape(&site.timezone)
        ),
    )
}

pub(crate) fn link(
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    change: Option<(&str, Option<&str>)>,
) -> String {
    let mut query = query.clone();
    if let Some((key, value)) = change {
        if key == "period" {
            query.date = None;
            query.interval = None;
        }
        if !matches!(
            key,
            "filter" | "filter_mode" | "filter_search" | "filter_offset"
        ) {
            query.filter = None;
            query.filter_mode = None;
            query.filter_op = None;
            query.filter_search = None;
            query.filter_offset = None;
        } else if key == "filter" {
            query.filter_mode = None;
            query.filter_op = None;
            query.filter_search = None;
            query.filter_offset = None;
        }
        if key != "offset" && !matches!(key, "metric" | "interval" | "compare" | "compare_match") {
            query.offset = None;
        }
        if key == "detail" || key.ends_with("_tab") {
            query.search = None;
            query.sort = None;
            query.order = None;
            if key.ends_with("_tab")
                && query.detail.as_deref().is_some_and(|dimension| match key {
                    "pages_tab" => matches!(dimension, "page" | "entry" | "exit"),
                    "sources_tab" => {
                        matches!(dimension, "source" | "referrer")
                            || crate::acquisition::is_dimension(dimension)
                    }
                    "locations_tab" => matches!(dimension, "country" | "region" | "city"),
                    "devices_tab" => matches!(
                        dimension,
                        "browser" | "browser_version" | "os" | "os_version" | "device"
                    ),
                    _ => false,
                })
            {
                query.detail = value
                    .filter(|v| key != "locations_tab" || *v != "map")
                    .map(str::to_owned);
            }
        }
        if crate::reports::DIMENSIONS
            .iter()
            .any(|(dimension, _)| *dimension == key)
            || matches!(key, "entry" | "exit")
        {
            query.detail = None;
            query.search = None;
            query.sort = None;
            query.order = None;
        }
        if crate::filters::DIMENSIONS
            .iter()
            .any(|(name, _, _)| *name == key)
        {
            query.remove_filter(key);
            if crate::acquisition::is_dimension(key)
                && let Some(value) = value
            {
                query.filters.push(crate::filters::Filter {
                    key: key.to_owned(),
                    op: crate::filters::Operator::Is,
                    values: vec![value.to_owned()],
                });
            }
        }
        // Location links descend through the hierarchy; removing or changing
        // a parent also removes its narrower filters. Values stay untouched.
        let parent = value
            .and_then(|v| v.split_once(": "))
            .map(|(code, _)| code)
            .filter(|code| code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase()));
        match key {
            "channel" if value.is_some() => query.sources_tab = Some("source".into()),
            "source" => {
                query.sources_tab = Some(
                    if value.is_some() {
                        "referrer"
                    } else {
                        "source"
                    }
                    .into(),
                )
            }
            "browser" => query.remove_filter("browser_version"),
            "os" => query.remove_filter("os_version"),
            "country" => {
                query.remove_filter("region");
                query.remove_filter("city");
                query.locations_tab = Some(if value.is_some() { "region" } else { "map" }.into());
            }
            "region" => {
                if let Some(country) = parent {
                    query.remove_filter("country");
                    query.country = Some(country.into());
                }
                query.remove_filter("city");
                query.locations_tab = Some(
                    if value.is_some() {
                        "city"
                    } else if query.exact_filter("country").is_some() {
                        "region"
                    } else {
                        "map"
                    }
                    .into(),
                );
            }
            "city" => {
                if let Some(country) = parent {
                    if query.exact_filter("country") != Some(country) {
                        query.remove_filter("region");
                    }
                    query.remove_filter("country");
                    query.country = Some(country.into());
                }
                query.locations_tab = Some("city".into());
            }
            _ => {}
        }
    }
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params
        .append_pair("from", &range.from.to_string())
        .append_pair("to", &range.to.to_string());
    params.append_pair("metric", query.metric.key());
    let with_imported = match change {
        Some(("with_imported", value)) => value.map(|value| value == "true"),
        _ => query.with_imported,
    };
    if let Some(enabled) = with_imported {
        params.append_pair("with_imported", if enabled { "true" } else { "false" });
    }
    if !query.filters.is_empty() {
        params.append_pair(
            "filters",
            &serde_json::to_string(&query.filters).expect("serializable filters"),
        );
    }
    if let Some(operator) = query.filter_op {
        params.append_pair("filter_op", operator.key());
    }
    if let Some(offset) = query.filter_offset.filter(|offset| *offset > 0) {
        params.append_pair("filter_offset", &offset.to_string());
    }
    for (key, value) in [
        ("filter", &query.filter),
        ("filter_mode", &query.filter_mode),
        ("filter_search", &query.filter_search),
        ("page", &query.page),
        ("source", &query.source),
        ("referrer", &query.referrer),
        ("country", &query.country),
        ("region", &query.region),
        ("city", &query.city),
        ("browser", &query.browser),
        ("browser_version", &query.browser_version),
        ("os", &query.os),
        ("os_version", &query.os_version),
        ("device", &query.device),
        ("entry", &query.entry),
        ("exit", &query.exit),
        ("pages_tab", &query.pages_tab),
        ("sources_tab", &query.sources_tab),
        ("locations_tab", &query.locations_tab),
        ("devices_tab", &query.devices_tab),
        ("period", &query.period),
        ("date", &query.date),
        ("compare", &query.compare),
        ("compare_from", &query.compare_from),
        ("compare_to", &query.compare_to),
        ("compare_match", &query.compare_match),
        ("interval", &query.interval),
        ("detail", &query.detail),
        ("sort", &query.sort),
        ("order", &query.order),
        ("search", &query.search),
    ] {
        let value = if change.is_some_and(|(name, _)| name == key) {
            change.unwrap().1
        } else {
            value.as_deref()
        };
        if let Some(value) = value {
            params.append_pair(key, value);
        }
    }
    if let Some(offset) = query.offset.filter(|offset| *offset > 0) {
        params.append_pair("offset", &offset.to_string());
    }
    format!("/sites/{}?{}", site.id, params.finish())
}

fn headline_value(metric: Metric, row: &Row) -> String {
    compact_metric(metric, row.available(metric).then(|| metric.value(row)))
}

fn compact_metric(metric: Metric, value: Option<f64>) -> String {
    if let Some(value) = value.filter(|_| {
        matches!(
            metric,
            Metric::Visitors | Metric::Pageviews | Metric::Visits
        )
    }) {
        compact_number(value as i64)
    } else {
        metric_display(metric, value)
    }
}

fn headline_dates(range: &Range, today: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    if range.from == range.to {
        range.from.format("%a, %d %b").to_string()
    } else {
        format!(
            "{} - {}",
            range.from.format(if range.from.year() != range.to.year() {
                "%-d %b %y"
            } else {
                "%-d %b"
            }),
            range.to.format(
                if range.to.year() != today.year() || range.from.year() != range.to.year() {
                    "%-d %b %y"
                } else {
                    "%-d %b"
                }
            )
        )
    }
}

pub(crate) fn hidden_state(
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    omit: &[&str],
) -> String {
    let target = link(site, range, query, None);
    let mut fields = String::new();
    for (key, value) in url::form_urlencoded::parse(target.split_once('?').unwrap().1.as_bytes()) {
        if !omit.contains(&key.as_ref()) {
            let _ = write!(
                fields,
                "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
                escape(&key),
                escape(&value)
            );
        }
    }
    fields
}

fn metric_display(metric: Metric, value: Option<f64>) -> String {
    let Some(value) = value else {
        return "—".into();
    };
    match metric {
        Metric::ViewsPerVisit => format!("{value:.2}"),
        Metric::BounceRate | Metric::ScrollDepth => format!("{value:.0}%"),
        Metric::VisitDuration | Metric::TimeOnPage => {
            let seconds = value.round() as i64;
            if seconds >= 60 {
                format!("{}m {:02}s", seconds / 60, seconds % 60)
            } else {
                format!("{seconds}s")
            }
        }
        _ => number(value as i64),
    }
}

struct MetricChange {
    amount: f64,
    rate: bool,
}

impl MetricChange {
    fn between(
        rate: bool,
        now: Option<f64>,
        before: Option<f64>,
        rate_baseline: bool,
    ) -> Option<Self> {
        let (now, before) = (now?, before?);
        if rate && !rate_baseline {
            return None;
        }
        Some(Self {
            amount: if rate {
                now - before
            } else if before == 0. {
                if now > 0. { 100. } else { 0. }
            } else {
                (now / before - 1.) * 100.
            },
            rate,
        })
    }

    fn tone(&self) -> &'static str {
        if self.amount.abs() < 0.5 {
            "steady"
        } else if (self.rate && self.amount < 0.) || (!self.rate && self.amount > 0.) {
            "positive"
        } else {
            "negative"
        }
    }

    fn arrow(&self) -> &'static str {
        if self.amount.abs() < 0.5 {
            "→"
        } else if self.amount >= 0. {
            "↗"
        } else {
            "↘"
        }
    }

    fn tooltip(&self) -> String {
        format!(
            "{} {:.0}{}",
            self.arrow(),
            self.amount.abs(),
            if self.rate { " pp" } else { "%" }
        )
    }

    fn description(&self) -> String {
        format!(
            "{} {:.0} {}",
            if self.amount.abs() < 0.5 {
                "Changed"
            } else if self.amount > 0. {
                "Up"
            } else {
                "Down"
            },
            self.amount.abs(),
            if self.rate {
                "percentage points"
            } else {
                "percent"
            }
        )
    }
}

fn number(value: i64) -> String {
    let text = value.to_string();
    let mut result = String::new();
    for (index, c) in text.chars().enumerate() {
        if index != 0 && (text.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

fn country_map(
    rows: &[Row],
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    report: &analytics::Report,
) -> String {
    let values = rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                if report.breakdown_pageviews {
                    row.pageviews
                } else {
                    row.visitors
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let shapes = crate::geography::country_shapes().collect::<Vec<_>>();
    let maximum = shapes
        .iter()
        .filter_map(|(code, _)| values.get(code))
        .copied()
        .max()
        .unwrap_or(0);
    let metric = if report.breakdown_pageviews {
        "pageviews"
    } else if report.realtime {
        "current visitors"
    } else {
        "visitors"
    };
    let mut out = format!(
        "<figure class=\"country-map\"><div class=\"map-canvas\"><svg viewBox=\"0 0 475 335\" role=\"group\" aria-label=\"{} by country\">",
        if report.breakdown_pageviews {
            "Pageviews"
        } else {
            "Visitors"
        }
    );
    let sprite = crate::geography::country_sprite_url();
    for (code, _) in &shapes {
        let count = values.get(code).copied().unwrap_or(0);
        if count > 0 {
            let intensity = 100. * count as f64 / maximum as f64;
            let unit = if count == 1 {
                &metric[..metric.len() - 1]
            } else {
                metric
            };
            let label = format!(
                "{} · {} {}",
                crate::geography::country_name(code),
                number(count),
                unit
            );
            let _ = write!(
                out,
                "<a href=\"{}\" aria-label=\"{}\" data-country=\"{code}\" data-name=\"{}\" data-value=\"{}\" data-unit=\"{unit}\"><title>{}</title><use class=\"map-country\" fill=\"color-mix(in srgb,var(--map-high) {intensity:.4}%,var(--map-low))\" href=\"{sprite}#{code}\"/></a>",
                escape(&link(site, range, query, Some(("country", Some(code))))),
                escape(&label),
                escape(crate::geography::country_name(code)),
                compact_number(count),
                escape(&label),
            );
        } else {
            let _ = write!(
                out,
                "<use class=\"map-country\" href=\"{sprite}#{code}\" aria-hidden=\"true\"/>"
            );
        }
    }
    out.push_str("<use class=\"map-outline\" hidden/></svg><div class=\"map-tooltip\" role=\"tooltip\" hidden><strong></strong><div><b></b> <span></span></div></div></div><figcaption>");
    if maximum == 0 {
        out.push_str("<p class=\"map-note\">No location data in this period</p>");
    }
    let unmapped: i64 = values
        .iter()
        .filter(|(code, _)| !shapes.iter().any(|(mapped, _)| mapped == *code))
        .map(|(_, count)| count)
        .sum();
    if unmapped > 0 {
        let _ = write!(
            out,
            "<a class=\"map-note\" href=\"{}\">Not on map: {} {metric}</a>",
            escape(&link(
                site,
                range,
                query,
                Some(("locations_tab", Some("country")))
            )),
            number(unmapped)
        );
    }
    out.push_str("</figcaption></figure>");
    out
}

fn source_tabs(site: &Site, range: &Range, query: &ReportQuery, selected: &str) -> String {
    let mut out = String::new();
    for (label, key) in [("Channels", "channel"), ("Sources", "source")] {
        let current = selected == key || (key == "source" && selected == "referrer");
        let target = if key == "source" && query.exact_filter("source").is_some() {
            "referrer"
        } else {
            key
        };
        let _ = write!(
            out,
            "<a data-preference=\"sources_tab\"{} href=\"{}\">{label}</a>",
            if current {
                " aria-current=\"page\""
            } else {
                ""
            },
            escape(&link(
                site,
                range,
                query,
                Some(("sources_tab", Some(target)))
            ))
        );
    }
    let campaign = crate::acquisition::is_campaign(selected);
    let title = crate::acquisition::DIMENSIONS
        .iter()
        .find(|(key, _)| *key == selected && campaign)
        .map(|(_, label)| format!("{label}s"))
        .unwrap_or_else(|| "Campaigns".into());
    let _ = write!(
        out,
        "<details class=\"dropdown campaign-menu\" name=\"dashboard-menu\"><summary{}>{title}{}</summary><div class=\"dropdown-content\">",
        if campaign {
            " aria-current=\"page\""
        } else {
            ""
        },
        crate::icons::control("chevron")
    );
    for (key, label) in crate::acquisition::DIMENSIONS
        .iter()
        .filter(|(key, _)| crate::acquisition::is_campaign(key))
    {
        let _ = write!(
            out,
            "<a data-preference=\"sources_tab\"{} href=\"{}\">{label}s</a>",
            if *key == selected {
                " aria-current=\"page\""
            } else {
                ""
            },
            escape(&link(site, range, query, Some(("sources_tab", Some(key)))))
        );
    }
    out.push_str("</div></details>");
    out
}

struct TableComparison<'a> {
    report: &'a analytics::Report,
    current: &'a Range,
    previous: &'a Range,
}

impl TableComparison<'_> {
    fn value(
        &self,
        dimension: &str,
        row: &Row,
        column: &str,
        heading: &str,
        compact: bool,
    ) -> String {
        let absent = Row {
            percentage: Some((0, 0)),
            ..Default::default()
        };
        let previous = self
            .report
            .rows(dimension)
            .iter()
            .find(|old| old.name == row.name)
            .or_else(|| (!self.report.comparison_missing.contains(dimension)).then_some(&absent));
        let now = crate::report_table::value(row, column);
        let before = previous.and_then(|old| crate::report_table::value(old, column));
        let change = MetricChange::between(
            column == "bounce_rate",
            now,
            before,
            previous.is_some_and(|old| old.entrances.unwrap_or(old.visits) > 0),
        );
        let current_value = crate::report_table::display(row, column);
        let previous_value = previous
            .map(|old| crate::report_table::display(old, column))
            .unwrap_or_else(|| "—".into());
        let name = crate::report_table::label(dimension, &row.name);
        let dates = |range: &Range| {
            if range.from == range.to {
                range.from.format("%-d %b %Y").to_string()
            } else {
                format!(
                    "{} – {}",
                    range.from.format("%-d %b %Y"),
                    range.to.format("%-d %b %Y")
                )
            }
        };
        let current_date = dates(self.current);
        let previous_date = dates(self.previous);
        let description = format!(
            "{name} · {heading}\n{current_date}: {current_value}\n{previous_date}: {previous_value}{}",
            change
                .as_ref()
                .map(|change| format!("\n{}", change.description()))
                .unwrap_or_default()
        );
        let arrow = change
            .as_ref()
            .map(|change| {
                format!(
                    "<i class=\"row-change {}\" aria-hidden=\"true\">{}</i>",
                    change.tone(),
                    change.arrow()
                )
            })
            .unwrap_or_default();
        let value = if compact {
            format!(
                "<strong>{}</strong>",
                compact_number(if column == "pageviews" {
                    row.pageviews
                } else {
                    row.visitors
                })
            )
        } else {
            current_value.clone()
        };
        format!(
            "<span class=\"row-value\" data-row-comparison data-name=\"{}\" data-heading=\"{}\" data-current=\"{}\" data-previous=\"{}\" data-current-date=\"{}\" data-previous-date=\"{}\" data-change=\"{}\" data-tone=\"{}\" title=\"{}\"{}>{value}{arrow}</span>",
            escape(&name),
            escape(heading),
            escape(&current_value),
            escape(&previous_value),
            escape(&current_date),
            escape(&previous_date),
            escape(
                &change
                    .as_ref()
                    .map(MetricChange::tooltip)
                    .unwrap_or_default()
            ),
            change.as_ref().map(MetricChange::tone).unwrap_or("steady"),
            escape(&description),
            if compact {
                String::new()
            } else {
                format!(" tabindex=\"0\" aria-label=\"{}\"", escape(&description))
            }
        )
    }
}

fn panel(
    group: &str,
    tabs: &[(&str, &str, &[Row])],
    selected: &str,
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    report: &analytics::Report,
    comparison: Option<&TableComparison<'_>>,
) -> String {
    let background_selected = selected;
    let selected = query
        .detail
        .as_deref()
        .filter(|d| tabs.iter().any(|(_, key, _)| key == d))
        .unwrap_or(selected);
    let (title, dimension, rows) = tabs
        .iter()
        .find(|(_, key, _)| *key == selected)
        .unwrap_or(&tabs[0]);
    let expanded = *dimension != "map" && query.detail.as_deref() == Some(*dimension);
    let mut out = String::new();
    if expanded {
        let mut background = query.clone();
        background.detail = None;
        background.search = None;
        background.sort = None;
        background.order = None;
        background.offset = None;
        out.push_str(&panel(
            group,
            tabs,
            background_selected,
            site,
            range,
            &background,
            report,
            comparison,
        ));
        let close = escape(&link(site, range, query, Some(("detail", None))));
        let _ = write!(
            out,
            "<div class=\"report-overlay\"><a class=\"report-backdrop\" href=\"{close}\" tabindex=\"-1\"></a><section class=\"panel report-panel panel-{dimension} expanded-report\"><header class=\"report-heading\"><h2>{title}</h2>"
        );
        out.push_str(&report_search(site, range, query));
        let _ = write!(
            out,
            "<a class=\"close-report\" href=\"{close}\" aria-label=\"Close expanded report\">{}</a></header>",
            crate::icons::control("close")
        );
    } else {
        let _ = write!(
            out,
            "<section class=\"panel report-panel panel-{dimension}\"><div class=\"report-tabs\">"
        );
        if group == "sources_tab" {
            out.push_str(&source_tabs(site, range, query, dimension));
        } else {
            for (label, key, _) in tabs {
                let current = if key == dimension {
                    " aria-current=\"page\""
                } else {
                    ""
                };
                let _ = write!(
                    out,
                    "<a data-preference=\"{group}\"{current} href=\"{}\">{label}</a>",
                    escape(&link(site, range, query, Some((group, Some(key)))))
                );
            }
        }
        let _ = write!(
            out,
            "<a class=\"expand-report\" href=\"{}\" aria-label=\"{} report\">{}</a></div>",
            escape(&link(
                site,
                range,
                query,
                Some((
                    "detail",
                    Some(if *dimension == "map" {
                        "country"
                    } else {
                        dimension
                    })
                ))
            )),
            "Expand",
            crate::icons::control("expand")
        );
    }
    let import_dimension = if *dimension == "map" {
        "country"
    } else {
        dimension
    };
    if report
        .imports_excluded_dimensions
        .contains(import_dimension)
        && !report.imports_excluded
    {
        out.push_str("<p class=\"report-note import-report-note\">Some historical data is unavailable with these filters.</p>");
    }
    if *dimension == "map" {
        out.push_str(&country_map(rows, site, range, query, report));
        out.push_str("</section>");
        return out;
    }
    let pageviews = report.breakdown_pageviews;
    let count = |r: &Row| if pageviews { r.pageviews } else { r.visitors };
    if report.acquisition_incomplete.contains(*dimension)
        && !query
            .active_filters()
            .iter()
            .any(|filter| report.acquisition_incomplete.contains(&filter.key))
    {
        out.push_str("<p class=\"report-note\">Some visits have no campaign data.</p>");
    }
    let label = match *dimension {
        "source" => "Source",
        "referrer" => "Referrer",
        key if crate::acquisition::is_dimension(key) => {
            crate::acquisition::DIMENSIONS
                .iter()
                .find(|(name, _)| *name == key)
                .unwrap()
                .1
        }
        "page" | "entry" | "exit" => "Page",
        "country" => "Country",
        "region" => "Region",
        "city" => "City",
        "browser" | "browser_version" => "Browser",
        "os" | "os_version" => "Operating system",
        _ => "Screen size",
    };
    if expanded {
        out.push_str(&expanded_table(
            dimension, label, rows, site, range, query, report, comparison,
        ));
        out.push_str("</section></div>");
        return out;
    }
    let _ = write!(
        out,
        "<div class=\"report-columns\"><span>{label}</span><span>{}</span></div>",
        if pageviews {
            "Pageviews"
        } else if report.realtime {
            "Current visitors"
        } else if *dimension == "entry" {
            "Unique entrances"
        } else if *dimension == "exit" {
            "Unique exits"
        } else {
            "Visitors"
        }
    );
    if rows.is_empty() {
        out.push_str(if crate::acquisition::is_campaign(dimension) {
            "<p class=\"no-data\">No tagged traffic</p>"
        } else {
            "<p class=\"no-data\">No data yet</p>"
        });
    }
    let max = rows.iter().map(count).max().unwrap_or(1).max(1) as f64;
    for row in rows.iter().take(9) {
        let name = crate::report_table::label(dimension, &row.name);
        let percentage = crate::report_table::display(row, "percentage");
        let image = if crate::acquisition::is_campaign(dimension)
            || matches!(
                *dimension,
                "page" | "entry" | "exit" | "browser_version" | "os_version"
            ) {
            String::new()
        } else {
            icon(dimension, &row.name)
        };
        let value = comparison
            .map(|comparison| {
                comparison.value(
                    dimension,
                    row,
                    if pageviews { "pageviews" } else { "visitors" },
                    if pageviews {
                        "Pageviews"
                    } else if *dimension == "entry" {
                        "Unique entrances"
                    } else if *dimension == "exit" {
                        "Unique exits"
                    } else {
                        "Visitors"
                    },
                    true,
                )
            })
            .unwrap_or_else(|| format!("<strong>{}</strong>", compact_number(count(row))));
        let _ = write!(
            out,
            "<a class=\"data-row\" href=\"{}\" title=\"{} · {} pageviews\"><svg class=\"row-bar\" aria-hidden=\"true\" preserveAspectRatio=\"none\"><rect width=\"{:.2}%\" height=\"100%\" rx=\"3\"/></svg><span class=\"row-label\">{image}{}</span>{value}<span class=\"row-percentage\">{percentage}</span></a>",
            escape(&link(
                site,
                range,
                query,
                Some((dimension, Some(&row.name)))
            )),
            escape(&name),
            number(row.pageviews),
            count(row) as f64 / max * 76.,
            escape(&name)
        );
    }
    if rows.len() > 9 {
        let _ = write!(
            out,
            "<a class=\"more-rows-link\" href=\"{}\">View all</a>",
            escape(&link(site, range, query, Some(("detail", Some(dimension)))))
        );
    }
    out.push_str("</section>");
    out
}

fn report_search(site: &Site, range: &Range, query: &ReportQuery) -> String {
    let mut out = String::from("<form class=\"report-search\" method=\"get\">");
    out.push_str(&hidden_state(site, range, query, &["search", "offset"]));
    let _ = write!(
        out,
        "<input id=\"report-search\" type=\"search\" name=\"search\" aria-label=\"Search report\" placeholder=\"Press / to search\" value=\"{}\"><button class=\"quiet\">Search</button></form>",
        escape(query.search.as_deref().unwrap_or(""))
    );
    out
}

fn expanded_table(
    dimension: &str,
    label: &str,
    rows: &[Row],
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    report: &analytics::Report,
    comparison: Option<&TableComparison<'_>>,
) -> String {
    let mut out = format!(
        "<div class=\"report-table-scroll\" tabindex=\"0\" role=\"region\" aria-label=\"{} report\"><table class=\"report-table\"><thead><tr>",
        escape(label)
    );
    let columns = crate::report_table::columns(dimension, report.breakdown_pageviews);
    let sort = query
        .sort
        .as_deref()
        .unwrap_or(if report.breakdown_pageviews {
            "pageviews"
        } else {
            "visitors"
        });
    for (key, heading) in [("name", label)].into_iter().chain(columns.iter().copied()) {
        let active = sort == key;
        let ascending = query.order.as_deref() == Some("asc");
        let mut next = query.clone();
        next.sort = Some(key.into());
        next.order = Some(
            if active && !ascending {
                "asc"
            } else if !active && key == "name" {
                "asc"
            } else {
                "desc"
            }
            .into(),
        );
        next.offset = None;
        let _ = write!(
            out,
            "<th scope=\"col\" aria-sort=\"{}\"><a href=\"{}\">{}<span aria-hidden=\"true\">{}</span></a></th>",
            if active {
                if ascending { "ascending" } else { "descending" }
            } else {
                "none"
            },
            escape(&link(site, range, &next, None)),
            escape(if report.realtime && key == "visitors" {
                "Current visitors"
            } else {
                heading
            }),
            if active {
                if ascending { " ↑" } else { " ↓" }
            } else {
                ""
            }
        );
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        let name = crate::report_table::label(dimension, &row.name);
        let image = if crate::acquisition::is_campaign(dimension)
            || matches!(
                dimension,
                "page" | "entry" | "exit" | "browser_version" | "os_version"
            ) {
            String::new()
        } else {
            icon(dimension, &row.name)
        };
        let _ = write!(
            out,
            "<tr><th scope=\"row\"><a href=\"{}\" title=\"{}\">{image}<span>{}</span></a></th>",
            escape(&link(
                site,
                range,
                query,
                Some((dimension, Some(&row.name)))
            )),
            escape(&name),
            escape(&name)
        );
        for (key, heading) in &columns {
            let missing = crate::report_table::value(row, key).is_none();
            let _ = write!(
                out,
                "<td{}>{}</td>",
                if missing
                    && ((*key == "time_on_page" && row.unavailable & 32 == 0)
                        || (*key == "scroll_depth" && row.unavailable & 64 == 0))
                {
                    " title=\"No engagement measurements yet\""
                } else if missing && *key == "visit_duration" && row.unavailable & (2 | 8) == 0 {
                    " title=\"No visits in this period\""
                } else if missing {
                    " title=\"Unavailable in this historical export\""
                } else {
                    ""
                },
                comparison
                    .map(|comparison| comparison.value(dimension, row, key, heading, false))
                    .unwrap_or_else(|| crate::report_table::display(row, key))
            );
        }
        out.push_str("</tr>");
    }
    if rows.is_empty() {
        let _ = write!(
            out,
            "<tr><td colspan=\"{}\" class=\"no-data\">No matching results</td></tr>",
            columns.len() + 1
        );
    }
    out.push_str(
        "</tbody></table></div><nav class=\"report-pagination\" aria-label=\"Report pages\">",
    );
    let offset = query.offset.unwrap_or(0);
    if offset > 0 {
        let mut previous = query.clone();
        previous.offset = Some(offset.saturating_sub(crate::reports::PAGE_SIZE));
        let _ = write!(
            out,
            "<a href=\"{}\">← Previous</a>",
            escape(&link(site, range, &previous, None))
        );
    }
    let _ = write!(
        out,
        "<span>{}–{} of {}</span>",
        if rows.is_empty() { 0 } else { offset + 1 },
        if rows.is_empty() {
            0
        } else {
            offset + rows.len()
        },
        number(report.detail_total as i64)
    );
    if offset + rows.len() < report.detail_total {
        let mut next = query.clone();
        next.offset = Some(offset + crate::reports::PAGE_SIZE);
        let _ = write!(
            out,
            "<a href=\"{}\">Next →</a>",
            escape(&link(site, range, &next, None))
        );
    }
    out.push_str("</nav>");
    out
}

fn chart(
    report: &analytics::Report,
    range: &Range,
    query: &ReportQuery,
    site: &Site,
    previous: Option<(&analytics::Report, &Range)>,
) -> String {
    use chrono::Datelike;
    struct Point {
        label: String,
        axis: String,
        axis_short: Option<String>,
        clock: Option<String>,
        ambiguous_clock: bool,
        wall_minute: Option<i64>,
        day_axis: String,
        ongoing: bool,
        value: Option<f64>,
        rate_baseline: bool,
        zoom: Option<(&'static str, chrono::NaiveDate)>,
    }
    fn series(
        report: &analytics::Report,
        range: &Range,
        query: &ReportQuery,
        zone: chrono_tz::Tz,
        now: i64,
    ) -> Vec<Point> {
        use chrono::TimeZone;
        let today = chrono::DateTime::from_timestamp(now, 0)
            .unwrap()
            .with_timezone(&zone)
            .date_naive();
        let short_date = |date: chrono::NaiveDate| {
            date.format(
                if date.year() != today.year() || range.from.year() != range.to.year() {
                    "%-d %b %y"
                } else {
                    "%-d %b"
                },
            )
            .to_string()
        };
        if !report.timeline.is_empty() {
            return report
                .timeline
                .iter()
                .enumerate()
                .filter(|(_, bucket)| bucket.start <= now)
                .map(|(index, bucket)| {
                    let time = chrono::DateTime::from_timestamp(bucket.start, 0)
                        .unwrap()
                        .with_timezone(&zone);
                    Point {
                        zoom: None,
                        axis_short: None,
                        clock: (!report.realtime).then(|| time.format("%H:%M").to_string()),
                        ambiguous_clock: matches!(
                            zone.from_local_datetime(&time.naive_local()),
                            chrono::LocalResult::Ambiguous(..)
                        ),
                        wall_minute: Some(time.naive_local().and_utc().timestamp().div_euclid(60)),
                        day_axis: short_date(time.date_naive()),
                        ongoing: !report.realtime && report.timeline_end > now && now < bucket.end,
                        label: if report.realtime {
                            let minutes = report.timeline.len() - index;
                            format!(
                                "{minutes} minute{} ago",
                                if minutes == 1 { "" } else { "s" }
                            )
                        } else {
                            time.format("%a, %d %b %Y, %H:%M %Z").to_string()
                        },
                        axis: if report.realtime {
                            format!("-{}m", report.timeline.len() - index)
                        } else {
                            time.format("%H:%M").to_string()
                        },
                        value: bucket.value(query.metric),
                        rate_baseline: bucket
                            .completed
                            .entrances
                            .unwrap_or(bucket.completed.visits)
                            > 0,
                    }
                })
                .collect();
        }
        let mut result = std::collections::BTreeMap::<chrono::NaiveDate, Row>::new();
        for offset in 0..=(range.to.min(today) - range.from).num_days() {
            let date = range.from + chrono::Duration::days(offset);
            let key = match query.interval.as_deref() {
                Some("week") => {
                    date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64)
                }
                Some("month") => date.with_day(1).unwrap(),
                _ => date,
            };
            result.entry(key).or_default();
        }
        let first = range.from.to_string();
        let last = range.to.min(today).to_string();
        for row in report.calendar.as_ref().unwrap_or(&report.daily) {
            if report.calendar.is_none() && (row.name < first || row.name > last) {
                continue;
            }
            let key = crate::calendar::key(query, &row.name)
                .parse::<chrono::NaiveDate>()
                .expect("stored calendar date");
            if let Some(target) = result.get_mut(&key) {
                target.add(row);
            }
        }
        result
            .into_iter()
            .map(|(date, row)| {
                let from = date.max(range.from);
                let end = match query.interval.as_deref() {
                    Some("week") => date + chrono::Duration::days(6),
                    Some("month") => date
                        .checked_add_months(chrono::Months::new(1))
                        .unwrap()
                        .pred_opt()
                        .unwrap(),
                    _ => from,
                };
                let to = end.min(range.to).min(today);
                let ongoing = (from..=to).contains(&today);
                let partial = from != date || to != end || ongoing;
                let label = match query.interval.as_deref() {
                    Some("week") => format!(
                        "{} {}",
                        if partial {
                            "Partial week of"
                        } else {
                            "Week of"
                        },
                        short_date(from)
                    ),
                    Some("month") => format!(
                        "{}{}",
                        if partial { "Partial of " } else { "" },
                        date.format("%B %Y")
                    ),
                    _ => from.format("%a, %d %b %Y").to_string(),
                };
                Point {
                    zoom: match query.interval.as_deref() {
                        Some("week") => None,
                        Some("month") => Some(("month", date)),
                        _ => Some(("day", date)),
                    },
                    label,
                    axis: if query.interval.as_deref() == Some("month") {
                        date.format("%B %Y").to_string()
                    } else {
                        short_date(from)
                    },
                    axis_short: (query.interval.as_deref() == Some("month"))
                        .then(|| date.format("%b %y").to_string()),
                    clock: None,
                    ambiguous_clock: false,
                    wall_minute: None,
                    day_axis: short_date(from),
                    ongoing,
                    value: (row.available(query.metric)
                        && (query.metric != Metric::VisitDuration || row.mean_visits() > 0))
                        .then(|| {
                            if query.metric == Metric::Visits && !query.has_filter("page") {
                                row.mean_visits() as f64
                            } else {
                                query.metric.value(&row)
                            }
                        }),
                    rate_baseline: row.entrances.unwrap_or(row.mean_visits()) > 0,
                }
            })
            .collect()
    }
    let zone = site.timezone.parse::<chrono_tz::Tz>().unwrap();
    let data = series(report, range, query, zone, report.generated_at);
    if data.is_empty() {
        return "<p class=\"no-data\">No completed chart intervals yet.</p>".into();
    }
    let old = previous
        .map(|(old_report, range)| series(old_report, range, query, zone, report.generated_at))
        .unwrap_or_default();
    let metric = query.metric;
    let peak = data
        .iter()
        .chain(old.iter())
        .filter_map(|point| point.value)
        .fold(1., f64::max);
    let minimum_step = if metric == Metric::ViewsPerVisit {
        0.1
    } else {
        1.
    };
    let rough_step = (peak / 4.).max(minimum_step);
    let magnitude = 10_f64.powf(rough_step.log10().floor());
    let step = [1., 2., 2.5, 5., 10.]
        .into_iter()
        .map(|factor| factor * magnitude)
        .find(|step| *step >= rough_step)
        .unwrap();
    let step = (step / minimum_step).ceil() * minimum_step;
    let ticks = (peak / step).ceil() as usize;
    let peak = step * ticks as f64;
    // Both series use one ordinal axis, including 23/25-hour comparison days.
    let count = data.len().max(old.len());
    let position = |i: usize| {
        if count == 1 {
            500.
        } else {
            i as f64 / (count - 1) as f64 * 1000.
        }
    };
    let height = |value: f64| 280. - value / peak * 260.;
    let segments = |rows: &[Point]| {
        let mut result = vec![Vec::new()];
        for (i, point) in rows.iter().enumerate() {
            if let Some(value) = point.value {
                result
                    .last_mut()
                    .unwrap()
                    .push((i, position(i), height(value)));
            } else if !result.last().unwrap().is_empty() {
                result.push(Vec::new());
            }
        }
        result.retain(|segment| !segment.is_empty());
        result
    };
    let points = |segment: &[(usize, f64, f64)]| {
        segment
            .iter()
            .map(|(_, x, y)| format!("{x:.2},{y:.2}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut content = String::from(
        "<div class=\"traffic-chart\"><div class=\"chart-scale\" aria-hidden=\"true\">",
    );
    for i in (0..=ticks).rev() {
        let value = step * i as f64;
        let label = match metric {
            Metric::BounceRate | Metric::ScrollDepth => format!("{value:.0}%"),
            Metric::ViewsPerVisit => format!("{value:.1}"),
            Metric::VisitDuration | Metric::TimeOnPage => format!("{value:.0}s"),
            _ => compact_number(value.round() as i64),
        };
        let _ = write!(content, "<span>{label}</span>");
    }
    let interval = if !report.timeline.is_empty() {
        if report.timeline_step == 60 {
            "minute"
        } else {
            "hour"
        }
    } else {
        query.interval.as_deref().unwrap_or("day")
    };
    let _ = write!(
        content,
        "</div><div class=\"chart-plot\"><svg class=\"chart\" data-interval=\"{}\" viewBox=\"0 0 1000 300\" preserveAspectRatio=\"none\" role=\"group\" aria-label=\"{} from {} to {}\"><path class=\"grid-line\" d=\"",
        escape(interval),
        metric.name(),
        range.from,
        range.to
    );
    for i in 0..=ticks {
        let _ = write!(
            content,
            "M0 {:.2}H1000",
            280. - i as f64 / ticks as f64 * 260.
        );
    }
    content.push_str("\" aria-hidden=\"true\"/>");
    content.push_str("<defs><linearGradient id=\"chart-fade\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"20\" x2=\"0\" y2=\"280\"><stop offset=\"0\" stop-opacity=\".15\"/><stop offset=\"1\" stop-opacity=\"0\"/></linearGradient><linearGradient id=\"chart-previous-fade\" gradientUnits=\"userSpaceOnUse\" x1=\"0\" y1=\"20\" x2=\"0\" y2=\"280\"><stop offset=\"0\" stop-opacity=\".05\"/><stop offset=\"1\" stop-opacity=\"0\"/></linearGradient></defs>");
    for (rows, class) in [(&old, "chart-area-previous"), (&data, "chart-area")] {
        for segment in segments(rows) {
            let _ = write!(
                content,
                "<polygon class=\"{class}\" points=\"{:.2},280 {} {:.2},280\"/>",
                segment.first().unwrap().1,
                points(&segment),
                segment.last().unwrap().1
            );
        }
    }
    for segment in segments(&old) {
        let _ = write!(
            content,
            "<polyline class=\"chart-previous\" points=\"{}\"/>",
            points(&segment)
        );
        if segment.len() == 1 {
            let _ = write!(
                content,
                "<path class=\"chart-point chart-point-previous\" d=\"M{:.2} {:.2}h0\"/>",
                segment[0].1, segment[0].2
            );
        }
    }
    for segment in segments(&data) {
        let first = segment.first().unwrap();
        let last = segment.last().unwrap();
        if data[last.0].ongoing && segment.len() > 1 {
            let _ = write!(
                content,
                "<polyline class=\"chart-line\" points=\"{}\"/><polyline class=\"chart-line chart-incomplete\" points=\"{}\"/>",
                points(&segment[..segment.len() - 1]),
                points(&segment[segment.len() - 2..])
            );
        } else {
            let _ = write!(
                content,
                "<polyline class=\"chart-line\" points=\"{}\"/>",
                points(&segment)
            );
        }
        if segment.len() == 1 {
            let _ = write!(
                content,
                "<path class=\"chart-point\" d=\"M{:.2} {:.2}h0\"/>",
                first.1, first.2
            );
        }
    }
    for i in 0..count {
        let step = 1000. / (count - 1).max(1) as f64;
        let point_x = position(i);
        let x = (point_x - step / 2.).max(0.);
        let width = (point_x + step / 2.).min(1000.) - x;
        let point = data.get(i);
        let before = old.get(i);
        let value = point.and_then(|point| point.value);
        let old_value = before.and_then(|point| point.value);
        let label = point.map(|point| point.label.as_str()).unwrap_or("");
        let old_label = before.map(|point| point.label.as_str()).unwrap_or("");
        let change = MetricChange::between(
            metric == Metric::BounceRate,
            value,
            old_value,
            before.is_some_and(|point| point.rate_baseline),
        );
        let comparison = old
            .get(i)
            .map(|point| {
                format!(
                    " · Previous: {} ({})",
                    metric_display(metric, point.value),
                    point.label
                )
            })
            .unwrap_or_default();
        let mut description = format!(
            "{}: {} {}{comparison}{}",
            if label.is_empty() {
                "No current interval"
            } else {
                label
            },
            metric_display(metric, value),
            metric.name().to_lowercase(),
            change
                .as_ref()
                .map(|change| format!(" · {}", change.description()))
                .unwrap_or_default()
        );
        if point.is_some_and(|point| point.ongoing) {
            description.push_str(" · In progress");
        }
        let zoom_period = point
            .or(before)
            .and_then(|point| point.zoom)
            .map(|(period, _)| period)
            .unwrap_or("");
        let zoom = point
            .or(before)
            .and_then(|point| point.zoom)
            .and_then(|(period, date)| date_link(site, query, period, date, true));
        let (tag, attributes, shape) = if let Some(target) = &zoom {
            (
                "a",
                format!("href=\"{}\" role=\"link\"", escape(target)),
                format!("<rect x=\"{x:.2}\" y=\"0\" width=\"{width:.2}\" height=\"300\"/>"),
            )
        } else {
            (
                "rect",
                format!("x=\"{x:.2}\" y=\"0\" width=\"{width:.2}\" height=\"300\" role=\"img\""),
                String::new(),
            )
        };
        let mut display = String::new();
        for (prefix, point) in [("", point), ("previous-", before)] {
            if let Some(point) = point {
                if point.ambiguous_clock {
                    let _ = write!(display, " data-{prefix}ambiguous-clock=\"true\"");
                }
                let compact = compact_metric(metric, point.value);
                if compact != metric_display(metric, point.value) {
                    let _ = write!(display, " data-{prefix}display-value=\"{compact}\"");
                }
            }
        }
        let _ = write!(
            content,
            "<{tag} class=\"chart-hit\" {attributes} tabindex=\"0\" aria-label=\"{}\" data-date=\"{}\" data-value=\"{}\" data-previous-date=\"{}\" data-previous-value=\"{}\" data-change=\"{}\" data-tone=\"{}\" data-x=\"{point_x:.2}\" data-y=\"{}\" data-previous-y=\"{}\" data-zoom=\"{zoom_period}\" data-clock=\"{}\" data-previous-clock=\"{}\" data-incomplete=\"{}\"{display}>{shape}<title>{}</title></{tag}>",
            escape(&description),
            escape(label),
            metric_display(metric, value),
            escape(old_label),
            metric_display(metric, old_value),
            change
                .as_ref()
                .map(MetricChange::tooltip)
                .unwrap_or_default(),
            change.as_ref().map(MetricChange::tone).unwrap_or("steady"),
            value
                .map(|value| format!("{:.2}", height(value)))
                .unwrap_or_default(),
            old_value
                .map(|value| format!("{:.2}", height(value)))
                .unwrap_or_default(),
            point.and_then(|point| point.clock.as_deref()).unwrap_or(""),
            before
                .and_then(|point| point.clock.as_deref())
                .unwrap_or(""),
            point.is_some_and(|point| point.ongoing),
            escape(&description)
        );
    }
    content.push_str("<g class=\"chart-cursor\" hidden aria-hidden=\"true\"><line y1=\"20\" y2=\"280\"/><path class=\"chart-marker\"/><path class=\"chart-marker chart-marker-previous\"/></g></svg>");
    let _ = write!(
        content,
        "<div class=\"chart-tooltip\" role=\"tooltip\" hidden><header><p>{}</p><small></small></header><div class=\"chart-tooltip-current\"><span></span><strong></strong></div><div class=\"chart-tooltip-previous\"><span></span><strong></strong></div><footer>Right click to pin tooltip</footer><a class=\"chart-tooltip-zoom\" hidden></a></div>",
        match metric {
            Metric::Visitors => "Unique visitors",
            Metric::Pageviews => "Total pageviews",
            _ => metric.name(),
        }
    );
    content.push_str(if report.realtime {
        "<div class=\"chart-labels chart-labels-realtime\" aria-hidden=\"true\">"
    } else {
        "<div class=\"chart-labels chart-labels-positioned\" aria-hidden=\"true\">"
    });
    if !report.realtime {
        // SVG coordinates position labels without inline CSS, preserving CSP
        // and the same axis geometry when JavaScript is disabled.
        content.push_str("<svg class=\"chart-ticks\" width=\"100%\" height=\"20\">");
    }
    let point_at = |index| data.get(index).unwrap_or_else(|| &old[index]);
    let mut tick_minutes = 0;
    let indices = if report.realtime {
        (0..count.min(6)).map(|i| i * 5).collect::<Vec<_>>()
    } else if data[0].wall_minute.is_some() {
        let rough = (count.saturating_sub(1) as i64 * report.timeline_step + 419) / 420;
        tick_minutes = [1, 2, 5, 10, 15, 30, 60, 120, 180, 240, 360, 720, 1440, 2880]
            .into_iter()
            .find(|step| *step >= rough)
            .unwrap_or(2880);
        let indices = (0..count)
            .filter(|&i| {
                point_at(i)
                    .wall_minute
                    .is_some_and(|minute| minute.rem_euclid(tick_minutes) == 0)
            })
            .collect::<Vec<_>>();
        if indices.is_empty() { vec![0] } else { indices }
    } else {
        (0..count).step_by(count.div_ceil(6)).collect::<Vec<_>>()
    };
    for index in indices {
        let point = point_at(index);
        if report.realtime {
            let _ = write!(content, "<span>{}</span>", escape(&point.axis));
            continue;
        }
        let axis = if tick_minutes >= 1440 {
            &point.day_axis
        } else {
            &point.axis
        };
        let clock = if tick_minutes < 1440 {
            point.clock.as_deref().unwrap_or("")
        } else {
            ""
        };
        let edge = if index == 0 && count > 1 {
            "start"
        } else if index + 1 == count && count > 1 {
            "end"
        } else {
            "middle"
        };
        let _ = write!(
            content,
            "<text x=\"{:.2}%\" y=\"1em\" text-anchor=\"{edge}\" data-clock=\"{clock}\">",
            position(index) / 10.
        );
        if let Some(short) = &point.axis_short {
            let _ = write!(
                content,
                "<tspan class=\"axis-long\">{}</tspan><tspan class=\"axis-short\">{}</tspan>",
                escape(axis),
                escape(short)
            );
        } else {
            content.push_str(&escape(axis));
        }
        content.push_str("</text>");
    }
    if !report.realtime {
        content.push_str("</svg>");
    }
    content.push_str("</div></div></div>");
    content
}

fn rolling_window(period: Option<&str>, now: i64) -> Option<(i64, i64)> {
    match period {
        // Start at now minus 30 minutes, before making the end exclusive.
        // Advancing both bounds by one second loses a chart bucket at :59.
        Some("realtime") => Some((now - 1800, now + 1)),
        Some("24h") => Some((now + 1 - 86400, now + 1)),
        _ => None,
    }
}

async fn default_period(
    db: &crate::db::Database,
    site: &Site,
    query: &mut ReportQuery,
) -> Result<chrono::NaiveDate, crate::db::Error> {
    let id = site.id;
    let today = site.today();
    let first = db.read(move |c| c.query_row(
        "SELECT MIN(day) FROM (SELECT MIN(day) day FROM events WHERE site_id=?1 UNION ALL SELECT MIN(day) FROM import_daily WHERE site_id=?1)",
        [id], |r| r.get::<_,Option<String>>(0)
    )).await?.and_then(|date| chrono::NaiveDate::parse_from_str(&date,"%Y-%m-%d").ok()).unwrap_or(today);
    if query.period.is_none() {
        if query.date.is_some() {
            query.period = Some("day".into());
        } else if query.from.is_none() && query.to.is_none() {
            query.period = Some(if first < today { "28d" } else { "day" }.into());
        }
    }
    Ok(first)
}

async fn resolve_range(
    db: &crate::db::Database,
    site: &Site,
    query: &mut ReportQuery,
    mut range: Range,
) -> Result<Range, crate::db::Error> {
    if query.period.as_deref() == Some("realtime") {
        query.interval = Some("minute".into());
        if !matches!(query.metric, Metric::Visitors | Metric::Pageviews) {
            query.metric = Metric::Visitors;
        }
    }
    if let Some((start, end)) = rolling_window(query.period.as_deref(), auth::now()) {
        query.window = Some((start, end));
        let zone = site.timezone.parse::<chrono_tz::Tz>().unwrap();
        range.from = chrono::DateTime::from_timestamp(start, 0)
            .unwrap()
            .with_timezone(&zone)
            .date_naive();
        range.to = chrono::DateTime::from_timestamp(end - 1, 0)
            .unwrap()
            .with_timezone(&zone)
            .date_naive();
    }
    if query.period.as_deref() == Some("all") {
        let id = site.id;
        let first=db.read(move|c|c.query_row("SELECT MIN(day) FROM (SELECT MIN(day) day FROM events WHERE site_id=?1 UNION ALL SELECT MIN(day) FROM import_daily WHERE site_id=?1)",[id],|r|r.get::<_,Option<String>>(0))).await?;
        if let Some(first) =
            first.and_then(|day| chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok())
        {
            range.from = first;
        }
    }
    if (range.to - range.from).num_days() > 6
        && matches!(query.interval.as_deref(), Some("hour" | "minute"))
    {
        query.interval = None;
    }
    Ok(range)
}

pub(crate) async fn export(
    State(app): State<App>,
    Path(id): Path<i64>,
    Query(mut query): Query<ReportQuery>,
    headers: HeaderMap,
) -> Response {
    query.normalize_locations();
    if let Err(response) = owner(&app, &headers).await {
        return response;
    }
    let site = match analytics::site(&app.db, id).await {
        Ok(Some(site)) => site,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return unavailable(),
    };
    if default_period(&app.db, &site, &mut query).await.is_err() {
        return unavailable();
    }
    let range = match query.range(site.today()) {
        Ok(range) => range,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let range = match resolve_range(&app.db, &site, &mut query, range).await {
        Ok(range) => range,
        Err(_) => return unavailable(),
    };
    if prepare_comparison(&app.db, &site, &range, &mut query)
        .await
        .is_err()
    {
        return unavailable();
    }
    query.export = true;
    let page_filtered = query.has_filter("page");
    let interval = query.interval.clone();
    let report = match analytics::report(&app.db, id, range.clone(), query).await {
        Ok(report) => report,
        Err(_) => return unavailable(),
    };
    let filename = format!(
        "attachment; filename=\"owlytics-{}-{}-{}.zip\"",
        site.domain, range.from, range.to
    );
    match tokio::task::spawn_blocking(move || {
        crate::exports::archive(&site, &range, &report, page_filtered, interval.as_deref())
    })
    .await
    {
        Ok(Ok(bytes)) => (
            [
                (
                    axum::http::header::CONTENT_TYPE,
                    "application/zip".to_owned(),
                ),
                (axum::http::header::CONTENT_DISPOSITION, filename),
            ],
            bytes,
        )
            .into_response(),
        _ => unavailable(),
    }
}

fn interval_options(range: &Range, query: &ReportQuery) -> Vec<&'static str> {
    let months = |count| {
        range
            .from
            .checked_add_months(chrono::Months::new(count))
            .is_some_and(|end| range.to >= end)
    };
    match query.period.as_deref() {
        Some("realtime") => vec!["minute"],
        Some("day" | "yesterday") => vec!["minute", "hour"],
        Some("24h") => vec!["hour"],
        Some("7d") => vec!["hour", "day"],
        Some("28d" | "30d" | "month" | "lastmonth") => vec!["day", "week"],
        Some("91d" | "6mo" | "12mo" | "year") => vec!["day", "week", "month"],
        Some("all") if months(13) => vec!["week", "month"],
        Some("all") => vec!["day", "week", "month"],
        _ if (range.to - range.from).num_days() < 7 => vec!["day"],
        _ if !months(1) => vec!["day", "week"],
        _ if !months(12) => vec!["day", "week", "month"],
        _ => vec!["week", "month"],
    }
}

fn default_interval(range: &Range, query: &ReportQuery) -> &'static str {
    match query.period.as_deref() {
        Some("day" | "yesterday" | "24h") => "hour",
        Some("6mo" | "12mo" | "year") => "month",
        None if (range.to - range.from).num_days() >= 30 => {
            if range
                .from
                .checked_add_months(chrono::Months::new(6))
                .is_some_and(|end| range.to >= end)
            {
                "month"
            } else {
                "week"
            }
        }
        Some("7d") => "day",
        _ => interval_options(range, query)[0],
    }
}

fn dashboard_menu(
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    report: &analytics::Report,
    previous: Option<(&analytics::Report, &Range)>,
) -> String {
    let mut menu = String::from(
        "<details class=\"dropdown dashboard-menu\" name=\"dashboard-menu\"><summary aria-label=\"Dashboard menu\" data-shortcut=\"i\">⋮</summary><div class=\"dropdown-content\"><p class=\"menu-label\">Graph interval</p>",
    );
    let active = query.interval.as_deref().unwrap_or_else(|| {
        if !report.timeline.is_empty() {
            if report.timeline_step == 60 {
                "minute"
            } else {
                "hour"
            }
        } else {
            "day"
        }
    });
    let days = (range.to - range.from).num_days().max(
        previous
            .map(|(_, range)| (range.to - range.from).num_days())
            .unwrap_or(0),
    );
    let historical = report.has_imports || previous.is_some_and(|(report, _)| report.has_imports);
    let mut options = interval_options(range, query);
    options.retain(|interval| match *interval {
        "minute" => days == 0 && !historical || report.realtime,
        "hour" => days <= 6 && !historical,
        _ => true,
    });
    if options.is_empty() {
        options.push("day");
    }
    for (interval, label) in [
        ("minute", "Minutes"),
        ("hour", "Hours"),
        ("day", "Days"),
        ("week", "Weeks"),
        ("month", "Months"),
    ] {
        if !options.contains(&interval) {
            continue;
        }
        let _ = write!(
            menu,
            "<a data-preference=\"interval\"{} href=\"{}\">{label}{}</a>",
            if active == interval {
                " aria-current=\"true\""
            } else {
                ""
            },
            escape(&link(
                site,
                range,
                query,
                Some(("interval", Some(interval)))
            )),
            if active == interval {
                "<span class=\"menu-check\" aria-hidden=\"true\">✓</span>"
            } else {
                ""
            }
        );
    }
    let url = link(site, range, query, None).replacen(
        &format!("/sites/{}?", site.id),
        &format!("/sites/{}/export?", site.id),
        1,
    );
    let _ = write!(
        menu,
        "<hr><a href=\"{}\" download>Export stats</a>",
        escape(&url)
    );
    if report.imports_available {
        let _ = write!(
            menu,
            "<hr><form class=\"import-switch\" method=\"get\">{}<button type=\"submit\" role=\"switch\" aria-checked=\"{}\" name=\"with_imported\" value=\"{}\">Include imported data<span aria-hidden=\"true\" class=\"switch-track\"></span></button></form>",
            hidden_state(
                site,
                range,
                query,
                &[
                    "with_imported",
                    "offset",
                    "filter",
                    "filter_mode",
                    "filter_op",
                    "filter_search",
                    "filter_offset"
                ]
            ),
            query.imports_enabled(),
            !query.imports_enabled()
        );
    }
    menu.push_str("</div></details>");
    menu
}

fn range_label(range: &Range) -> String {
    use chrono::Datelike;
    if range.from == range.to {
        range.from.format("%b %-d, %Y").to_string()
    } else {
        format!(
            "{} – {}",
            range.from.format(if range.from.year() == range.to.year() {
                "%b %-d"
            } else {
                "%b %-d, %Y"
            }),
            range.to.format("%b %-d, %Y")
        )
    }
}

fn calendar_period<'a>(range: &Range, query: &'a ReportQuery) -> Option<&'a str> {
    match query.period.as_deref() {
        Some("day" | "yesterday") => Some("day"),
        Some("month" | "lastmonth") => Some("month"),
        Some("year") => Some("year"),
        None if range.from == range.to => Some("day"),
        _ => None,
    }
}

fn date_link(
    site: &Site,
    query: &ReportQuery,
    period: &str,
    date: chrono::NaiveDate,
    zoom: bool,
) -> Option<String> {
    let mut target = query.clone();
    target.period = Some(period.into());
    target.date = Some(date.to_string());
    target.from = None;
    target.to = None;
    target.window = None;
    if zoom {
        target.interval = None;
    }
    let range = target.range(site.today()).ok()?;
    Some(link(
        site,
        &range,
        &target,
        Some(("date", target.date.as_deref())),
    ))
}

fn shifted_date(
    range: &Range,
    query: &ReportQuery,
    first: chrono::NaiveDate,
    today: chrono::NaiveDate,
    forward: bool,
) -> Option<chrono::NaiveDate> {
    let period = calendar_period(range, query)?;
    let date = match (period, forward) {
        ("day", false) => range.from.pred_opt(),
        ("day", true) => range.from.succ_opt(),
        ("month", false) => range.from.checked_sub_months(chrono::Months::new(1)),
        ("month", true) => range.from.checked_add_months(chrono::Months::new(1)),
        ("year", false) => range.from.checked_sub_months(chrono::Months::new(12)),
        ("year", true) => range.from.checked_add_months(chrono::Months::new(12)),
        _ => None,
    }?;
    let target = ReportQuery {
        period: Some(period.into()),
        date: Some(date.to_string()),
        ..Default::default()
    }
    .range(today)
    .ok()?;
    if (forward && target.from <= today) || (!forward && target.to >= first) {
        Some(date)
    } else {
        None
    }
}

fn date_controls(
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    previous: &Range,
    first: chrono::NaiveDate,
) -> String {
    use chrono::Datelike;
    const PRESETS: [(&str, &str); 14] = [
        ("day", "Today"),
        ("yesterday", "Yesterday"),
        ("realtime", "Realtime"),
        ("24h", "Last 24 hours"),
        ("7d", "Last 7 days"),
        ("28d", "Last 28 days"),
        ("30d", "Last 30 days"),
        ("91d", "Last 91 days"),
        ("month", "Month to date"),
        ("lastmonth", "Last month"),
        ("year", "Year to date"),
        ("6mo", "Last 6 months"),
        ("12mo", "Last 12 months"),
        ("all", "All time"),
    ];
    let today = site.today();
    let label = match calendar_period(range, query) {
        Some("day") if range.from != today && query.period.as_deref() != Some("yesterday") => {
            Some(range.from.format("%a, %d %b %Y").to_string())
        }
        Some("month") if range.from != today.with_day(1).unwrap() => {
            Some(range.from.format("%B %Y").to_string())
        }
        Some("year") if range.from.year() != today.year() => {
            Some(range.from.format("%Y").to_string())
        }
        _ => None,
    }
    .unwrap_or_else(|| {
        PRESETS
            .iter()
            .find(|(key, _)| query.period.as_deref() == Some(*key))
            .map(|(_, title)| title.to_string())
            .unwrap_or_else(|| headline_dates(range, today))
    });
    let mut body = format!(
        "<details class=\"dropdown date-picker\" name=\"dashboard-menu\"><summary>{}<span>{}</span></summary><div class=\"dropdown-content\">",
        crate::icons::control("calendar"),
        escape(&label)
    );
    let menu_link = |key, value: &str, label: &str, active| {
        let shortcut = if key == "period" {
            match value {
                "day" => "d",
                "yesterday" => "e",
                "realtime" => "r",
                "24h" => "h",
                "7d" => "w",
                "28d" => "f",
                "30d" => "t",
                "91d" => "n",
                "month" => "m",
                "lastmonth" => "p",
                "year" => "y",
                "6mo" => "s",
                "12mo" => "l",
                "all" => "a",
                _ => "",
            }
        } else if label == "Compare" || label == "Disable comparison" {
            "x"
        } else {
            ""
        };
        format!(
            "<a{} data-shortcut=\"{shortcut}\" href=\"{}\">{}{}{}</a>",
            if active { " aria-current=\"true\"" } else { "" },
            escape(&link(site, range, query, Some((key, Some(value))))),
            escape(label),
            if shortcut.is_empty() {
                String::new()
            } else {
                format!(
                    "<kbd class=\"menu-key\" aria-hidden=\"true\">{}</kbd>",
                    shortcut.to_uppercase()
                )
            },
            if active {
                "<span class=\"menu-check\" aria-hidden=\"true\">✓</span>"
            } else {
                ""
            }
        )
    };
    for (period, title) in PRESETS {
        if matches!(period, "24h" | "month" | "year" | "all") {
            body.push_str("<hr>");
        }
        let active = match period {
            "day" => calendar_period(range, query) == Some("day") && range.from == today,
            "yesterday" => {
                calendar_period(range, query) == Some("day") && Some(range.from) == today.pred_opt()
            }
            "month" => {
                calendar_period(range, query) == Some("month")
                    && range.from == today.with_day(1).unwrap()
            }
            "lastmonth" => {
                calendar_period(range, query) == Some("month")
                    && Some(range.from)
                        == today
                            .with_day(1)
                            .unwrap()
                            .checked_sub_months(chrono::Months::new(1))
            }
            "year" => {
                calendar_period(range, query) == Some("year") && range.from.year() == today.year()
            }
            _ => query.period.as_deref() == Some(period),
        };
        body.push_str(&menu_link("period", period, title, active));
    }
    let _ = write!(
        body,
        "<hr><details class=\"custom-range-toggle\"><summary data-shortcut=\"c\"><span>Custom range</span><kbd class=\"menu-key\" aria-hidden=\"true\">C</kbd></summary><form class=\"custom-range\" method=\"get\" data-calendar-min=\"{first}\" data-calendar-max=\"{today}\" data-calendar-selected=\"{}\">",
        query.period.is_none() && query.from.is_some()
    );
    body.push_str(&hidden_state(
        site,
        range,
        query,
        if query.period.is_some() {
            &["from", "to", "period", "date", "offset", "interval"]
        } else {
            &["from", "to", "period", "date", "offset"]
        },
    ));
    let _ = write!(
        body,
        "<label for=\"from\">From</label><input id=\"from\" name=\"from\" type=\"date\" value=\"{}\" required><label for=\"to\">To</label><input id=\"to\" name=\"to\" type=\"date\" value=\"{}\" required><button class=\"button\">Apply</button></form></details>",
        range.from, range.to
    );
    if !matches!(query.period.as_deref(), Some("realtime" | "all")) {
        body.push_str("<hr>");
        body.push_str(&if comparison_enabled(query) {
            menu_link("compare", "off", "Disable comparison", false)
        } else {
            menu_link("compare", "previous", "Compare", false)
        });
    }
    body.push_str("</div></details>");
    if let Some(period) = calendar_period(range, query) {
        body.push_str("<nav class=\"date-navigation\" aria-label=\"Change date\">");
        for (forward, label, key, shape) in [
            (false, "Previous", "arrowleft", "m8 3-4 5 4 5"),
            (true, "Following", "arrowright", "m4 3 4 5-4 5"),
        ] {
            let target = shifted_date(range, query, first, today, forward)
                .and_then(|date| date_link(site, query, period, date, false));
            let icon = format!(
                "<svg width=\"12\" height=\"16\" viewBox=\"0 0 12 16\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.5\" aria-hidden=\"true\"><path d=\"{shape}\"/></svg>"
            );
            if let Some(target) = target {
                let _ = write!(
                    body,
                    "<a href=\"{}\" aria-label=\"{label} {period}\" title=\"{label} {period}\" data-shortcut=\"{key}\">{icon}</a>",
                    escape(&target)
                );
            } else {
                let _ = write!(
                    body,
                    "<span role=\"link\" aria-disabled=\"true\" aria-label=\"{label} {period}\">{icon}</span>"
                );
            }
        }
        body.push_str("</nav>");
    }
    if !comparison_enabled(query) {
        return body;
    }
    let mode = query.compare.as_deref().unwrap_or("previous");
    let label = match mode {
        "year" => "Year over year".to_string(),
        "custom" => headline_dates(previous, today),
        _ => "Previous period".to_string(),
    };
    let _ = write!(
        body,
        "<span class=\"comparison-versus\">vs</span><details class=\"dropdown comparison-picker\" name=\"dashboard-menu\"><summary title=\"{}\">{}<span>{}</span></summary><div class=\"dropdown-content\">",
        escape(&range_label(previous)),
        crate::icons::control("calendar"),
        escape(&label)
    );
    body.push_str(&menu_link("compare", "off", "Disable comparison", false));
    body.push_str("<hr>");
    for (value, label) in [("previous", "Previous period"), ("year", "Year over year")] {
        body.push_str(&menu_link("compare", value, label, mode == value));
    }
    let _ = write!(
        body,
        "<details class=\"custom-range-toggle\"><summary>Custom period</summary><form class=\"custom-range custom-comparison\" method=\"get\" data-calendar-min=\"{first}\" data-calendar-max=\"{today}\" data-calendar-selected=\"{}\">",
        mode == "custom"
    );
    body.push_str(&hidden_state(
        site,
        range,
        query,
        &["compare", "compare_from", "compare_to"],
    ));
    let _ = write!(
        body,
        "<input type=\"hidden\" name=\"compare\" value=\"custom\"><label for=\"compare-from\">Compare from</label><input id=\"compare-from\" name=\"compare_from\" type=\"date\" value=\"{}\" required><label for=\"compare-to\">Compare to</label><input id=\"compare-to\" name=\"compare_to\" type=\"date\" value=\"{}\" required><button class=\"button\">Compare</button></form></details>",
        previous.from, previous.to
    );
    if mode != "custom" {
        body.push_str("<hr>");
        for (value, label) in [
            ("weekday", "Match day of week"),
            ("date", "Match exact date"),
        ] {
            body.push_str(&menu_link(
                "compare_match",
                value,
                label,
                query.compare_match.as_deref().unwrap_or("weekday") == value,
            ));
        }
    }
    body.push_str("</div></details>");
    body
}

fn comparison_enabled(query: &ReportQuery) -> bool {
    matches!(
        query.compare.as_deref(),
        Some("previous" | "year" | "custom")
    ) && !matches!(query.period.as_deref(), Some("realtime" | "all"))
}

fn weekday_shift(source: chrono::NaiveDate, comparison: chrono::NaiveDate) -> i64 {
    use chrono::Datelike;
    let shift = (source.weekday().num_days_from_monday() as i64
        - comparison.weekday().num_days_from_monday() as i64)
        .rem_euclid(7);
    if comparison + chrono::Duration::days(shift) == source {
        shift - 7
    } else {
        shift
    }
}

fn comparison_period(
    range: &Range,
    query: &ReportQuery,
    zone: chrono_tz::Tz,
    now: i64,
) -> (Range, ReportQuery) {
    let custom = query
        .custom_comparison()
        .expect("validated comparison dates");
    let today = chrono::DateTime::from_timestamp(now, 0)
        .unwrap()
        .with_timezone(&zone)
        .date_naive();
    let days = (range.to.min(today).max(range.from) - range.from).num_days();
    let mut previous_range = if let Some(custom) = custom.as_ref() {
        custom.clone()
    } else {
        let from = if query.compare.as_deref() == Some("year") {
            range
                .from
                .checked_sub_months(chrono::Months::new(12))
                .unwrap_or(range.from)
        } else {
            range.from - chrono::Duration::days(days + 1)
        };
        let shift = if query.compare_match.as_deref().unwrap_or("weekday") == "weekday" {
            weekday_shift(range.from, from)
        } else {
            0
        };
        let from = from + chrono::Duration::days(shift);
        Range {
            from,
            to: from + chrono::Duration::days(days),
        }
    };
    let mut previous_query = query.clone();
    previous_query.scope = analytics::ReportScope::Timeline;
    previous_query.period = None;
    previous_query.date = None;
    previous_query.window = None;
    if custom.is_none()
        && let Some((start, end)) = query.window
    {
        let span = end - start;
        let mut previous_end = if query.compare.as_deref() == Some("year") {
            let local = chrono::DateTime::from_timestamp(end, 0)
                .unwrap()
                .with_timezone(&zone)
                .naive_local();
            crate::timeline::local_time(
                zone,
                local.checked_sub_months(chrono::Months::new(12)).unwrap(),
            )
        } else {
            start
        };
        if query.compare_match.as_deref().unwrap_or("weekday") == "weekday" {
            let local = |stamp| {
                chrono::DateTime::from_timestamp(stamp, 0)
                    .unwrap()
                    .with_timezone(&zone)
            };
            let shift = weekday_shift(
                local(start).date_naive(),
                local(previous_end - span).date_naive(),
            );
            previous_end = crate::timeline::local_time(
                zone,
                local(previous_end).naive_local() + chrono::Duration::days(shift),
            );
        }
        previous_query.window = Some((previous_end - span, previous_end));
        if query.period.as_deref() == Some("realtime") && query.interval.is_none() {
            previous_query.interval = Some("minute".into());
        }
        previous_range.from = chrono::DateTime::from_timestamp(previous_end - span, 0)
            .unwrap()
            .with_timezone(&zone)
            .date_naive();
        previous_range.to = chrono::DateTime::from_timestamp(previous_end - 1, 0)
            .unwrap()
            .with_timezone(&zone)
            .date_naive();
    } else if custom.is_none() && query.period.as_deref() == Some("day") {
        let local = chrono::DateTime::from_timestamp(now + 1, 0)
            .unwrap()
            .with_timezone(&zone);
        if range.from == local.date_naive() {
            let cutoff =
                crate::timeline::local_time(zone, previous_range.to.and_time(local.time()));
            previous_query.window = Some((
                crate::timeline::day_start(zone, previous_range.from),
                cutoff,
            ));
        }
    }
    previous_query.from = Some(previous_range.from.to_string());
    previous_query.to = Some(previous_range.to.to_string());
    (previous_range, previous_query)
}

async fn prepare_comparison(
    db: &crate::db::Database,
    site: &Site,
    range: &Range,
    query: &mut ReportQuery,
) -> Result<(Range, ReportQuery), crate::db::Error> {
    let (previous_range, mut previous_query) =
        comparison_period(range, query, site.timezone.parse().unwrap(), auth::now());
    let enabled = comparison_enabled(query);
    let days = (range.to - range.from).num_days().max(if enabled {
        (previous_range.to - previous_range.from).num_days()
    } else {
        0
    });
    // Preset changes select their own interval; an explicit custom URL can
    // still request partial weeks/months or bounded fine-grained exports.
    let options = interval_options(range, query);
    if query.interval.is_none()
        || (query.period.is_some()
            && query
                .interval
                .as_deref()
                .is_some_and(|interval| !options.contains(&interval)))
    {
        query.interval = Some(default_interval(range, query).into());
    }
    if matches!(query.interval.as_deref(), Some("hour") if days > 6)
        || (query.period.as_deref() != Some("realtime")
            && matches!(query.interval.as_deref(), Some("minute") if days > 0))
    {
        query.interval = Some("day".into());
    }
    let fine = matches!(query.interval.as_deref(), Some("hour" | "minute"));
    if fine && query.imports_enabled() {
        let id = site.id;
        let current = (range.clone(), query.clone());
        let previous = (previous_range.clone(), previous_query.clone());
        let historical = db.read(move |c| {
            let included = |(range, query): &(Range, ReportQuery)| -> rusqlite::Result<bool> {
                if query.window.is_some() { return Ok(false); }
                let values: Vec<rusqlite::types::Value> = vec![id.into(),range.from.to_string().into(),range.to.to_string().into()];
                if query.filtered() {
                    let selected = std::collections::BTreeSet::from(["total"]);
                    Ok(crate::import_reports::build(c, &values, query, &selected, false)?.included.contains("total"))
                } else {
                    c.query_row("SELECT EXISTS(SELECT 1 FROM import_daily WHERE site_id=?1 AND day BETWEEN ?2 AND ?3)",rusqlite::params_from_iter(&values),|r|r.get(0))
                }
            };
            Ok(included(&current)? || (enabled && included(&previous)?))
        }).await?;
        if historical {
            query.interval = Some("day".into());
        }
    }
    previous_query.interval = query.interval.clone();
    Ok((previous_range, previous_query))
}

pub(crate) async fn apply_filter(
    State(app): State<App>,
    Path(id): Path<i64>,
    Query(mut query): Query<ReportQuery>,
    RawQuery(raw): RawQuery,
    headers: HeaderMap,
) -> Response {
    let session = match owner(&app, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let site = match analytics::site(&app.db, id).await {
        Ok(Some(site)) => site,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return unavailable(),
    };
    let result = (|| {
        query.range(site.today())?;
        let key = query.filter.clone().ok_or("Choose a filter.")?;
        let op = query.filter_op.unwrap_or_else(|| {
            query
                .active_filters()
                .iter()
                .find(|filter| filter.key == key)
                .map_or(crate::filters::Operator::Is, |filter| filter.op)
        });
        let raw = raw.as_deref().unwrap_or_default();
        if raw.len() > 65536 {
            return Err("Filters are too long.");
        }
        let mut values = Vec::new();
        for (name, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            if (name == "value" || (name == "term" && !value.is_empty()))
                && !values.contains(&value.as_ref().to_owned())
            {
                values.push(value.into_owned());
            }
        }
        let filter = crate::filters::Filter {
            key: key.clone(),
            op,
            values,
        };
        if !filter.values.is_empty() {
            filter.validate()?;
        }
        query.remove_filter(&key);
        for child in match key.as_str() {
            "country" => &["region", "city"][..],
            "region" => &["city"][..],
            "browser" => &["browser_version"][..],
            "os" => &["os_version"][..],
            _ => &[],
        } {
            query.remove_filter(child);
        }
        if !filter.values.is_empty() {
            query.filters.push(filter);
        }
        query.filter_op = Some(op);
        query.filter_mode = Some("values".into());
        query.offset = None;
        let range = query.range(site.today())?;
        Ok(link(&site, &range, &query, None))
    })();
    match result {
        Ok(target) => Redirect::to(&target).into_response(),
        Err(message) => {
            let mut response = page(
                "Invalid filter",
                &session,
                &format!(
                    "<section class=\"form-card\"><h1>Check the filter</h1><p class=\"error\" role=\"alert\">{}</p><a href=\"/sites/{id}\">Back to dashboard</a></section>",
                    escape(message)
                ),
            );
            *response.status_mut() = StatusCode::BAD_REQUEST;
            response
        }
    }
}

pub(crate) async fn show(
    State(app): State<App>,
    Path(id): Path<i64>,
    Query(mut query): Query<ReportQuery>,
    headers: HeaderMap,
) -> Response {
    query.normalize_locations();
    query.metric = match (query.has_filter("page"), query.metric) {
        (true, Metric::ViewsPerVisit) => Metric::ScrollDepth,
        (true, Metric::VisitDuration) => Metric::TimeOnPage,
        (false, Metric::ScrollDepth) => Metric::ViewsPerVisit,
        (false, Metric::TimeOnPage) => Metric::VisitDuration,
        (_, metric) => metric,
    };
    let session = match owner(&app, &headers).await {
        Ok(session) => session,
        Err(response) => return response,
    };
    let site = match analytics::site(&app.db, id).await {
        Ok(Some(site)) => site,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return unavailable(),
    };
    let first = match default_period(&app.db, &site, &mut query).await {
        Ok(first) => first,
        Err(_) => return unavailable(),
    };
    let mut range = match query.range(site.today()) {
        Ok(range) => range,
        Err(error) => {
            let mut response = page(
                "Invalid date range",
                &session,
                &format!(
                    "<section class=\"form-card\"><h1>Check the date range</h1><p class=\"error\" role=\"alert\">{}</p><a class=\"button\" href=\"/sites/{id}\">Back to dashboard</a></section>",
                    escape(error)
                ),
            );
            *response.status_mut() = StatusCode::BAD_REQUEST;
            return response;
        }
    };
    range = match resolve_range(&app.db, &site, &mut query, range).await {
        Ok(range) => range,
        Err(_) => return unavailable(),
    };
    let (previous_range, mut previous_query) =
        match prepare_comparison(&app.db, &site, &range, &mut query).await {
            Ok(comparison) => comparison,
            Err(_) => return unavailable(),
        };
    query.scope = analytics::ReportScope::Dashboard;
    let report = match analytics::report(&app.db, id, range.clone(), query.clone()).await {
        Ok(report) => report,
        Err(_) => return unavailable(),
    };
    let previous = if comparison_enabled(&query) {
        previous_query.comparison_names = Some(report.comparison_names());
        match analytics::report(&app.db, id, previous_range.clone(), previous_query).await {
            Ok(report) => Some(report),
            Err(_) => return unavailable(),
        }
    } else {
        None
    };
    let sites = analytics::sites(&app.db).await.unwrap_or_default();
    let mut body = format!(
        "<div class=\"dashboard-toolbar\" data-site=\"{}\" data-period=\"{}\" data-realtime=\"{}\"><div class=\"site-and-live\"><details class=\"dropdown site-picker\" name=\"dashboard-menu\"><summary>{}<span class=\"site-name\">{}</span>{}</summary><div class=\"dropdown-content\">",
        escape(&site.domain),
        escape(query.period.as_deref().unwrap_or("custom")),
        report.realtime,
        crate::icons::site(site.id, &site.domain),
        escape(&site.domain),
        crate::icons::control("chevron")
    );
    for (index, item) in sites.into_iter().enumerate() {
        let _ = write!(
            body,
            "<a class=\"site-option\" href=\"/sites/{}\"{}{}>{}<span class=\"site-name\">{}</span>{}</a>",
            item.id,
            if index < 9 {
                format!(" data-shortcut=\"{}\"", index + 1)
            } else {
                String::new()
            },
            if item.id == site.id {
                " aria-current=\"page\""
            } else {
                ""
            },
            crate::icons::site(item.id, &item.domain),
            escape(&item.domain),
            if item.id == site.id {
                format!(
                    "<span class=\"menu-check\">{}</span>",
                    crate::icons::control("check")
                )
            } else {
                String::new()
            }
        );
    }
    let _ = write!(
        body,
        "<hr><a href=\"/sites/{id}/settings\">Website settings</a><a href=\"/\">All websites</a><a href=\"/sites/new\">Add website</a></div></details>"
    );
    if !report.realtime {
        let _ = write!(
            body,
            "<a class=\"current-visitors\" href=\"{}\"><i></i>{} current visitors</a>",
            escape(&link(
                &site,
                &range,
                &query,
                Some(("period", Some("realtime")))
            )),
            number(report.current_visitors)
        );
    }
    body.push_str("</div><div class=\"dashboard-controls\">");
    body.push_str(&crate::filter_controls::menu(&site, &range, &query));
    body.push_str(&date_controls(
        &site,
        &range,
        &query,
        &previous_range,
        first,
    ));
    body.push_str(&dashboard_menu(
        &site,
        &range,
        &query,
        &report,
        previous.as_ref().map(|report| (report, &previous_range)),
    ));
    body.push_str("</div></div>");
    let clear = link(&site, &range, &query.without_filters(), None);
    let options = match crate::filter_controls::suggestions(&app.db, &site, &range, &query).await {
        Ok(options) => options,
        Err(_) => return unavailable(),
    };
    body.push_str(&crate::filter_controls::render(
        &site,
        &range,
        &query,
        options.as_ref(),
    ));
    if report.imports_excluded {
        body.push_str(if query.window.is_some() {
            "<p class=\"import-note\">Rolling windows use collected data only.</p>"
        } else if report.has_imports {
            "<p class=\"import-note\">Some historical data cannot be included with these filters.</p>"
        } else {
            "<p class=\"import-note\">Filters show collected data only. Historical data is excluded.</p>"
        });
    }
    if query
        .active_filters()
        .iter()
        .any(|filter| report.acquisition_incomplete.contains(&filter.key))
    {
        body.push_str("<p class=\"import-note\">Some earlier visits have no campaign data and cannot match these filters.</p>");
    }
    if previous
        .as_ref()
        .is_some_and(|report| report.imports_excluded)
        && !report.imports_excluded
    {
        body.push_str(if previous.as_ref().is_some_and(|report| report.has_imports) {
            "<p class=\"import-note\">Some historical comparison data cannot be included with these filters.</p>"
        } else {
            "<p class=\"import-note\">Comparison shows collected data only. Historical data is excluded.</p>"
        });
    }
    body.push_str(if report.realtime {
        "<section class=\"panel overview\"><div class=\"metrics metrics-realtime\">"
    } else {
        "<section class=\"panel overview\"><div class=\"metrics\">"
    });
    if report.realtime {
        let _ = write!(
            body,
            "<div><div class=\"metric realtime-current\" data-value=\"{}\" title=\"All visitors who loaded a page in the last 5 minutes\"><span>Current visitors</span><strong>{}</strong><small class=\"live-status\" role=\"status\" title=\"Updates every 30 seconds\">Live</small></div></div>",
            report.current_visitors,
            compact_number(report.current_visitors)
        );
    }
    let metrics = if report.realtime {
        vec![
            (Metric::Visitors, "Unique visitors (last 30 min)"),
            (Metric::Pageviews, "Total pageviews (last 30 min)"),
        ]
    } else {
        vec![
            (Metric::Visitors, "Unique visitors"),
            (Metric::Visits, "Total visits"),
            (Metric::Pageviews, "Total pageviews"),
            if query.has_filter("page") {
                (Metric::ScrollDepth, "Scroll depth")
            } else {
                (Metric::ViewsPerVisit, "Views per visit")
            },
            (Metric::BounceRate, "Bounce rate"),
            if query.has_filter("page") {
                (Metric::TimeOnPage, "Time on page")
            } else {
                (Metric::VisitDuration, "Visit duration")
            },
        ]
    };
    for (metric, label) in metrics {
        let current = if metric == query.metric {
            " aria-current=\"true\""
        } else {
            ""
        };
        let available = report.totals.available(metric);
        let value = headline_value(metric, &report.totals);
        let mut trend = String::new();
        let mut comparison = String::new();
        if let Some(old) = previous.as_ref() {
            let _ = write!(
                comparison,
                "<small class=\"metric-date\" title=\"{}\">{}</small><div class=\"metric-previous\"><b>{}</b><small class=\"metric-date\" title=\"{}\">{}</small></div>",
                escape(&range_label(&range)),
                headline_dates(&range, site.today()),
                headline_value(metric, &old.totals),
                escape(&range_label(&previous_range)),
                headline_dates(&previous_range, site.today()),
            );
            if let Some(change) = MetricChange::between(
                metric == Metric::BounceRate,
                available.then(|| metric.value(&report.totals)),
                old.totals
                    .available(metric)
                    .then(|| metric.value(&old.totals)),
                old.totals.entrances.unwrap_or(old.totals.mean_visits()) > 0,
            ) {
                trend = format!(
                    "<small class=\"metric-change {}\" title=\"{}\">{} {:.0}%</small>",
                    change.tone(),
                    if change.rate {
                        "Change in percentage points"
                    } else {
                        "Compared with the previous period"
                    },
                    change.arrow(),
                    change.amount.abs()
                );
            }
        }
        let _ = write!(
            body,
            "<div><a class=\"metric\"{current} data-preference=\"metric\" data-available=\"{available}\" data-value=\"{}\" href=\"{}\" aria-label=\"Show {}\"><span>{label}</span><strong>{value}</strong>{trend}{comparison}</a></div>",
            metric.value(&report.totals),
            escape(&link(
                &site,
                &range,
                &ReportQuery {
                    metric,
                    ..query.clone()
                },
                None
            )),
            metric.name().to_lowercase()
        );
    }
    body.push_str("</div>");
    if report.pageviews == 0 && report.visitors == 0 && !report.realtime {
        let (message, label, target) = if query.filtered() {
            ("No matching pageviews", "Clear filters", clear)
        } else if !report.has_data {
            (
                "Waiting for your first pageview",
                "Install tracker",
                format!("/sites/{id}/settings"),
            )
        } else {
            (
                "No pageviews in this period",
                "Last 28 days",
                link(
                    &site,
                    &Range {
                        from: site.today() - chrono::Duration::days(28),
                        to: site.today() - chrono::Duration::days(1),
                    },
                    &ReportQuery {
                        period: Some("28d".into()),
                        ..query.clone()
                    },
                    None,
                ),
            )
        };
        let _ = write!(body, "<div class=\"empty-report\"><h2>{message}</h2>");
        if !query.filtered() {
            let _ = write!(
                body,
                "<a class=\"button\" href=\"{}\">{label}</a>",
                escape(&target)
            );
        }
        body.push_str("</div>");
        // An empty current period must not hide its comparison series.
        if report.has_data
            && let Some(old) = previous.as_ref()
        {
            body.push_str(&chart(
                &report,
                &range,
                &query,
                &site,
                Some((old, &previous_range)),
            ));
        }
    } else if !report.totals.available(query.metric) {
        body.push_str(if matches!(query.metric, Metric::TimeOnPage | Metric::ScrollDepth) {
            "<p class=\"no-data\">No engagement measurements yet.</p>"
        } else {
            "<p class=\"no-data\">Session metrics are unavailable for this historical export.</p>"
        });
    } else {
        body.push_str(&chart(
            &report,
            &range,
            &query,
            &site,
            previous.as_ref().map(|p| (p, &previous_range)),
        ));
    }
    let table_comparison = previous.as_ref().map(|report| TableComparison {
        report,
        current: &range,
        previous: &previous_range,
    });
    body.push_str("</section><div class=\"report-grid\">");
    body.push_str(&panel(
        "sources_tab",
        &[
            ("Channels", "channel", report.acquisition_rows("channel")),
            ("Sources", "source", &report.sources),
            ("Referrers", "referrer", &report.referrers),
            (
                "UTM mediums",
                "utm_medium",
                report.acquisition_rows("utm_medium"),
            ),
            (
                "UTM sources",
                "utm_source",
                report.acquisition_rows("utm_source"),
            ),
            (
                "UTM campaigns",
                "utm_campaign",
                report.acquisition_rows("utm_campaign"),
            ),
            (
                "UTM contents",
                "utm_content",
                report.acquisition_rows("utm_content"),
            ),
            ("UTM terms", "utm_term", report.acquisition_rows("utm_term")),
        ],
        query.source_tab(),
        &site,
        &range,
        &query,
        &report,
        table_comparison.as_ref(),
    ));
    body.push_str(&panel(
        "pages_tab",
        &[
            ("Top pages", "page", &report.pages),
            ("Entry pages", "entry", &report.entry_pages),
            ("Exit pages", "exit", &report.exit_pages),
        ],
        query.pages_tab.as_deref().unwrap_or("page"),
        &site,
        &range,
        &query,
        &report,
        table_comparison.as_ref(),
    ));
    body.push_str(&panel(
        "locations_tab",
        &[
            ("Map", "map", &report.countries),
            ("Countries", "country", &report.countries),
            ("Regions", "region", &report.regions),
            ("Cities", "city", &report.cities),
        ],
        query.location_tab(),
        &site,
        &range,
        &query,
        &report,
        table_comparison.as_ref(),
    ));
    let device_tabs = [
        if query.exact_filter("browser").is_some() {
            (
                "Browser versions",
                "browser_version",
                report.browser_versions.as_slice(),
            )
        } else {
            ("Browsers", "browser", report.browsers.as_slice())
        },
        if query.exact_filter("os").is_some() {
            ("OS versions", "os_version", report.os_versions.as_slice())
        } else {
            ("OS", "os", report.operating_systems.as_slice())
        },
        ("Devices", "device", report.devices.as_slice()),
    ];
    let selected_device = match query.devices_tab.as_deref().unwrap_or(device_tabs[0].1) {
        "browser" if query.exact_filter("browser").is_some() => "browser_version",
        "os" if query.exact_filter("os").is_some() => "os_version",
        other => other,
    };
    body.push_str(&panel(
        "devices_tab",
        &device_tabs,
        selected_device,
        &site,
        &range,
        &query,
        &report,
        table_comparison.as_ref(),
    ));
    if previous.is_some() {
        body.push_str("<div id=\"row-comparison\" class=\"row-tooltip\" role=\"tooltip\" hidden><div class=\"row-tooltip-heading\"><strong></strong><button type=\"button\" aria-label=\"Close comparison\" hidden>×</button></div><span class=\"row-tooltip-metric\"></span><dl><div><dt class=\"row-current-date\"></dt><dd class=\"row-current-value\"></dd></div><div><dt class=\"row-previous-date\"></dt><dd class=\"row-previous-value\"></dd></div></dl><small class=\"row-tooltip-change\"></small></div>");
    }
    body.push_str("</div><footer class=\"report-footer\">");
    if report.breakdown_pageviews {
        body.push_str("<span>Historical breakdowns show pageviews.</span>");
    }
    let _ = write!(body, "<span>{}</span>", escape(&site.timezone));
    if report.geo_enabled {
        body.push_str(
            "<a href=\"https://db-ip.com\" rel=\"noreferrer\">IP Geolocation by DB-IP</a>",
        );
    }
    if report
        .cities
        .iter()
        .any(|row| crate::geography::imported_city(&row.name).is_some())
        || query.active_filters().iter().any(|filter| {
            filter.key == "city"
                && filter
                    .values
                    .iter()
                    .any(|value| crate::geography::imported_city(value).is_some())
        })
        || options.as_ref().is_some_and(|report| {
            report
                .cities
                .iter()
                .any(|row| crate::geography::imported_city(&row.name).is_some())
        })
    {
        body.push_str("<span>City names: <a href=\"https://www.geonames.org/\" rel=\"noreferrer\">GeoNames</a> · <a href=\"https://creativecommons.org/licenses/by/4.0/\" rel=\"noreferrer\">CC BY 4.0</a></span>");
    }
    body.push_str("</footer><script defer src=\"/assets/dashboard.js\"></script>");
    page(&site.domain, &session, &body)
}

#[cfg(test)]
#[path = "chart_tests.rs"]
mod chart_tests;

#[cfg(test)]
#[path = "row_comparison_tests.rs"]
mod row_comparison_tests;

#[cfg(test)]
mod tests {
    use super::compact_number;

    #[test]
    fn interval_defaults_and_menus_follow_calendar_periods() {
        use crate::analytics::{Range, ReportQuery};
        for (period, from, to, options, default) in [
            (
                Some("day"),
                "2024-02-29",
                "2024-02-29",
                "minute,hour",
                "hour",
            ),
            (Some("7d"), "2024-02-01", "2024-02-07", "hour,day", "day"),
            (Some("28d"), "2024-02-01", "2024-02-28", "day,week", "day"),
            (Some("month"), "2024-02-01", "2024-02-29", "day,week", "day"),
            (
                Some("year"),
                "2024-01-01",
                "2024-01-02",
                "day,week,month",
                "month",
            ),
            (
                Some("6mo"),
                "2024-01-01",
                "2024-06-30",
                "day,week,month",
                "month",
            ),
            (None, "2024-02-01", "2024-02-01", "day", "day"),
            (None, "2024-02-01", "2024-02-07", "day", "day"),
            (None, "2024-02-01", "2024-02-08", "day,week", "day"),
            (None, "2024-01-31", "2024-02-29", "day,week,month", "day"),
            (None, "2024-01-01", "2024-01-31", "day,week", "week"),
            (None, "2024-01-01", "2024-06-30", "day,week,month", "week"),
            (None, "2024-01-01", "2024-07-01", "day,week,month", "month"),
            (None, "2024-01-01", "2025-01-01", "week,month", "month"),
            (
                Some("all"),
                "2024-01-01",
                "2025-02-01",
                "week,month",
                "week",
            ),
        ] {
            let range = Range {
                from: from.parse().unwrap(),
                to: to.parse().unwrap(),
            };
            let query = ReportQuery {
                period: period.map(str::to_owned),
                ..Default::default()
            };
            assert_eq!(
                super::interval_options(&range, &query).join(","),
                options,
                "{period:?}/{range:?}"
            );
            assert_eq!(
                super::default_interval(&range, &query),
                default,
                "{period:?}/{range:?}"
            );
        }
    }

    #[test]
    fn date_arrows_follow_calendar_boundaries_and_available_history() {
        use crate::analytics::ReportQuery;
        let today = "2024-03-15".parse().unwrap();
        let first = "2023-02-17".parse().unwrap();
        for (period, date, back, next) in [
            ("day", "2024-03-01", Some("2024-02-29"), Some("2024-03-02")),
            ("day", "2024-03-15", Some("2024-03-14"), None),
            ("day", "2023-02-17", None, Some("2023-02-18")),
            ("month", "2024-03-31", Some("2024-02-01"), None),
            (
                "month",
                "2023-03-31",
                Some("2023-02-01"),
                Some("2023-04-01"),
            ),
            ("month", "2023-02-28", None, Some("2023-03-01")),
            ("year", "2024-02-29", Some("2023-01-01"), None),
            ("year", "2023-02-28", None, Some("2024-01-01")),
        ] {
            let query = ReportQuery {
                period: Some(period.into()),
                date: Some(date.into()),
                ..Default::default()
            };
            let range = query.range(today).unwrap();
            for (forward, expected) in [(false, back), (true, next)] {
                assert_eq!(
                    super::shifted_date(&range, &query, first, today, forward)
                        .map(|d| d.to_string()),
                    expected.map(str::to_owned),
                    "{period}/{date}/{forward}"
                );
            }
        }
        for period in ["7d", "28d", "6mo", "24h", "realtime", "all"] {
            let query = ReportQuery {
                period: Some(period.into()),
                ..Default::default()
            };
            assert_eq!(
                super::calendar_period(&query.range(today).unwrap(), &query),
                None
            );
        }
    }

    #[test]
    fn comparisons_preserve_duration_and_match_weekdays() {
        use crate::analytics::{Range, ReportQuery};
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-27T12:34:56Z")
            .unwrap()
            .timestamp();
        for (from, to, mode, weekday, expected_from, expected_to) in [
            (
                "2024-02-01",
                "2024-02-29",
                "year",
                false,
                "2023-02-01",
                "2023-03-01",
            ),
            (
                "2023-02-01",
                "2023-02-28",
                "year",
                false,
                "2022-02-01",
                "2022-02-28",
            ),
            (
                "2023-01-01",
                "2023-01-31",
                "previous",
                true,
                "2022-12-04",
                "2023-01-03",
            ),
            (
                "2023-03-01",
                "2023-03-02",
                "previous",
                true,
                "2023-02-22",
                "2023-02-23",
            ),
            (
                "2023-01-01",
                "2023-03-01",
                "year",
                true,
                "2022-01-02",
                "2022-03-02",
            ),
            (
                "2026-10-20",
                "2026-10-26",
                "previous",
                true,
                "2026-10-13",
                "2026-10-19",
            ),
        ] {
            let range = Range {
                from: from.parse().unwrap(),
                to: to.parse().unwrap(),
            };
            let query = ReportQuery {
                compare: Some(mode.into()),
                compare_match: Some(if weekday { "weekday" } else { "date" }.into()),
                ..Default::default()
            };
            let (previous, _) = super::comparison_period(&range, &query, chrono_tz::UTC, now);
            assert_eq!(
                (previous.from.to_string(), previous.to.to_string()),
                (expected_from.into(), expected_to.into()),
                "{from}/{to}/{mode}/{weekday}"
            );
            assert_eq!(
                (previous.to - previous.from).num_days(),
                (range.to - range.from).num_days()
            );
        }
        // Custom comparisons retain both endpoints, even for Today and with a
        // weekday preference left over from an automatic comparison.
        let query = ReportQuery {
            period: Some("day".into()),
            compare: Some("custom".into()),
            compare_from: Some("2025-01-01".into()),
            compare_to: Some("2025-01-17".into()),
            compare_match: Some("weekday".into()),
            ..Default::default()
        };
        let range = query.range("2026-10-27".parse().unwrap()).unwrap();
        let (previous, previous_query) =
            super::comparison_period(&range, &query, chrono_tz::UTC, now);
        assert_eq!(
            (previous.from.to_string(), previous.to.to_string()),
            ("2025-01-01".into(), "2025-01-17".into())
        );
        assert_eq!(previous_query.window, None);
        assert_eq!(previous_query.period, None);
    }

    #[tokio::test]
    async fn comparisons_share_intervals_without_discarding_history() {
        use crate::{
            analytics::{self, ReportQuery},
            db::Database,
        };
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
        let site = analytics::site(&db, 1).await.unwrap().unwrap();
        let base = ReportQuery {
            from: Some("2025-02-01".into()),
            to: Some("2025-02-01".into()),
            compare: Some("custom".into()),
            compare_from: Some("2025-01-01".into()),
            compare_to: Some("2025-01-01".into()),
            ..Default::default()
        };
        let range = base.range(site.today()).unwrap();
        for (filtered, enabled, interval) in [
            (false, true, Some("day")),
            (true, true, Some("day")),
            (false, false, Some("day")),
        ] {
            let mut query = ReportQuery {
                browser: filtered.then(|| "Chrome".into()),
                compare: Some(if enabled { "custom" } else { "off" }.into()),
                ..base.clone()
            };
            let (_, previous) = super::prepare_comparison(&db, &site, &range, &mut query)
                .await
                .unwrap();
            assert_eq!(query.interval.as_deref(), interval);
            assert_eq!(previous.interval, query.interval);
        }
        let mut query = ReportQuery {
            compare_from: Some("2025-01-02".into()),
            compare_to: Some("2025-01-08".into()),
            ..base
        };
        let (_, previous) = super::prepare_comparison(&db, &site, &range, &mut query)
            .await
            .unwrap();
        assert_eq!(
            query.interval.as_deref(),
            Some("day"),
            "Unequal ranges use the same bucket size"
        );
        assert_eq!(previous.interval, query.interval);
    }

    #[tokio::test]
    async fn realtime_keeps_thirty_intervals_at_every_second_of_the_minute() {
        use crate::{
            analytics::{self, ReportQuery, ReportScope},
            db::Database,
        };
        let db = Database::open(":memory:").unwrap();
        let minute = 1793059200 + 12 * 3600; // 2026-10-27 12:00 UTC
        db.call(move |c| {
            c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", [])?;
            for stamp in [minute - 1800 + 59, minute + 59] {
                c.execute("INSERT INTO events(site_id,received_at,day,visitor,request_id,path,source,country,browser,device) VALUES(1,?1,'2026-10-27',randomblob(16),randomblob(16),'/','','US','Chrome','Desktop')", [stamp])?;
            }
            Ok(())
        }).await.unwrap();
        for second in 0..60 {
            let query = ReportQuery {
                period: Some("realtime".into()),
                scope: ReportScope::Timeline,
                window: super::rolling_window(Some("realtime"), minute + second),
                ..Default::default()
            };
            let range = query.range("2026-10-27".parse().unwrap()).unwrap();
            let report = analytics::report(&db, 1, range, query).await.unwrap();
            assert_eq!(report.timeline.len(), 30, "second={second}");
            assert_eq!(report.timeline[0].start, minute - 1800);
            assert_eq!(report.timeline[29].start, minute - 60);
            assert_eq!(report.timeline[0].pageviews, 1);
            assert_eq!(
                report
                    .timeline
                    .iter()
                    .map(|bucket| bucket.pageviews)
                    .sum::<i64>(),
                1,
                "The current incomplete minute is omitted"
            );
        }
        assert_eq!(
            super::rolling_window(Some("24h"), minute),
            Some((minute + 1 - 86400, minute + 1))
        );
    }

    #[test]
    fn compact_counts_promote_units_when_rounding() {
        for (count, expected) in [
            (0, "0"),
            (999, "999"),
            (1000, "1k"),
            (12_300, "12.3k"),
            (999_949, "999.9k"),
            (999_950, "1m"),
            (1_000_000, "1m"),
            (999_950_000, "1b"),
        ] {
            assert_eq!(compact_number(count), expected);
        }
    }

    #[test]
    fn today_comparisons_use_the_same_local_clock_time() {
        use crate::analytics::{Range, ReportQuery};
        let stamp = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .timestamp()
        };
        let q = ReportQuery {
            period: Some("day".into()),
            compare_match: Some("date".into()),
            ..Default::default()
        };
        let range = Range {
            from: "2026-10-27".parse().unwrap(),
            to: "2026-10-27".parse().unwrap(),
        };
        let (previous, query) =
            super::comparison_period(&range, &q, chrono_tz::UTC, stamp("2026-10-27T12:34:56Z"));
        assert_eq!(previous.from.to_string(), "2026-10-26");
        assert_eq!(
            query.window,
            Some((stamp("2026-10-26T00:00:00Z"), stamp("2026-10-26T12:34:57Z")))
        );
        let range = Range {
            from: "2026-03-09".parse().unwrap(),
            to: "2026-03-09".parse().unwrap(),
        };
        let (_, query) = super::comparison_period(
            &range,
            &q,
            chrono_tz::America::New_York,
            stamp("2026-03-09T06:30:00Z"),
        );
        // Yesterday skipped 02:30; stop at the next real local time, 03:00.
        assert_eq!(
            query.window,
            Some((stamp("2026-03-08T05:00:00Z"), stamp("2026-03-08T07:00:00Z")))
        );
        let (_, query) = super::comparison_period(
            &range,
            &ReportQuery::default(),
            chrono_tz::America::New_York,
            stamp("2026-03-09T06:30:00Z"),
        );
        assert_eq!(
            query.window, None,
            "explicit calendar ranges retain full-day comparisons"
        );
    }

    #[test]
    fn year_comparisons_handle_leap_days_and_missing_local_times() {
        use crate::analytics::{Range, ReportQuery};
        let stamp = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .unwrap()
                .timestamp()
        };
        let q = ReportQuery {
            period: Some("day".into()),
            compare_match: Some("date".into()),
            compare: Some("year".into()),
            ..Default::default()
        };
        let range = Range {
            from: "2024-02-29".parse().unwrap(),
            to: "2024-02-29".parse().unwrap(),
        };
        let (previous, query) =
            super::comparison_period(&range, &q, chrono_tz::UTC, stamp("2024-02-29T10:15:00Z"));
        assert_eq!(previous.from.to_string(), "2023-02-28");
        assert_eq!(
            query.window,
            Some((stamp("2023-02-28T00:00:00Z"), stamp("2023-02-28T10:15:01Z")))
        );
        let range = Range {
            from: "2026-03-08".parse().unwrap(),
            to: "2026-03-09".parse().unwrap(),
        };
        let end = stamp("2026-03-09T06:30:00Z");
        let q = ReportQuery {
            period: Some("24h".into()),
            compare_match: Some("date".into()),
            compare: Some("year".into()),
            window: Some((end - 86400, end)),
            ..Default::default()
        };
        let (_, query) = super::comparison_period(&range, &q, chrono_tz::America::New_York, end);
        let previous_end = stamp("2025-03-09T07:00:00Z");
        assert_eq!(query.window, Some((previous_end - 86400, previous_end)));
    }
}
