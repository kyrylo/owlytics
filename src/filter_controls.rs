//! Server-rendered filter menus. The optional script submits these same forms.
use crate::{
    analytics::{Range, Report, ReportQuery, ReportScope, Row, Site},
    dashboard::{hidden_state, link},
    db::{Database, Error},
    filters::{DIMENSIONS, Filter, Operator},
    web::escape,
};
use std::fmt::Write;

pub(crate) fn menu(site: &Site, range: &Range, query: &ReportQuery) -> String {
    let mut html = String::from(
        "<details class=\"dropdown filter-menu-toggle\" name=\"dashboard-menu\"><summary>☷ Filter</summary><div class=\"dropdown-content filter-menu\">",
    );
    for (group, dimensions) in [
        ("Page", &["page", "entry", "exit"][..]),
        ("Source", &["source", "referrer"][..]),
        ("Channel", &["channel"][..]),
        (
            "Campaign",
            &[
                "utm_medium",
                "utm_source",
                "utm_campaign",
                "utm_content",
                "utm_term",
            ][..],
        ),
        ("Location", &["country", "region", "city"][..]),
        ("Screen size", &["device"][..]),
        ("Browser", &["browser", "browser_version"][..]),
        ("Operating system", &["os", "os_version"][..]),
    ] {
        if let [key] = dimensions {
            let _ = write!(
                html,
                "<a data-filter-link=\"open:{key}\" href=\"{}\">{group}</a>",
                escape(&link(site, range, query, Some(("filter", Some(key)))))
            );
            continue;
        }
        let _ = write!(
            html,
            "<details class=\"filter-group\" name=\"filter-group\"><summary>{group}<span aria-hidden=\"true\">›</span></summary>"
        );
        for (key, _, label) in DIMENSIONS
            .iter()
            .filter(|(key, _, _)| dimensions.contains(key))
        {
            let _ = write!(
                html,
                "<a data-filter-link=\"open:{key}\" href=\"{}\">{label}</a>",
                escape(&link(site, range, query, Some(("filter", Some(key)))))
            );
        }
        html.push_str("</details>");
    }
    html.push_str("</div></details>");
    html
}

pub(crate) async fn suggestions(
    db: &Database,
    site: &Site,
    range: &Range,
    query: &ReportQuery,
) -> Result<Option<Report>, Error> {
    let Some(key) = query.filter.as_deref() else {
        return Ok(None);
    };
    if query.filter_mode.as_deref() == Some("operator") {
        return Ok(None);
    }
    let op = query
        .active_filters()
        .iter()
        .find(|filter| filter.key == key)
        .map(|filter| filter.op)
        .or(query.filter_op)
        .unwrap_or_default();
    if matches!(op, Operator::Contains | Operator::ContainsNot) {
        return Ok(None);
    }
    let mut options = query.clone();
    options.remove_filter(key);
    options.scope = ReportScope::Suggestions;
    options.detail = Some(key.into());
    options.search = query.filter_search.clone();
    options.sort = None;
    options.order = None;
    options.offset = query.filter_offset;
    // Child selections must not hide alternatives to their current parent.
    for child in match key {
        "country" => &["region", "city"][..],
        "region" => &["city"][..],
        "browser" => &["browser_version"][..],
        "os" => &["os_version"][..],
        _ => &[],
    } {
        options.remove_filter(child);
    }
    crate::analytics::report(db, site.id, range.clone(), options)
        .await
        .map(Some)
}

fn rows<'a>(report: &'a Report, key: &str) -> &'a [Row] {
    match key {
        "page" => &report.pages,
        "entry" => &report.entry_pages,
        "exit" => &report.exit_pages,
        "source" => &report.sources,
        "referrer" => &report.referrers,
        key if crate::acquisition::is_dimension(key) => report.acquisition_rows(key),
        "country" => &report.countries,
        "region" => &report.regions,
        "city" => &report.cities,
        "browser" => &report.browsers,
        "browser_version" => &report.browser_versions,
        "os" => &report.operating_systems,
        "os_version" => &report.os_versions,
        "device" => &report.devices,
        _ => &[],
    }
}

fn value_label(filter: &Filter, value: &str) -> String {
    if matches!(filter.op, Operator::Contains | Operator::ContainsNot) {
        value.to_owned()
    } else {
        crate::report_table::label(&filter.key, value)
    }
}

fn edit_link(site: &Site, range: &Range, query: &ReportQuery, key: &str, mode: &str) -> String {
    let mut edit = query.clone();
    if edit.filter.as_deref() != Some(key) {
        edit.filter_search = None;
        edit.filter_offset = None;
        edit.filter_op = None;
    }
    edit.filter = Some(key.into());
    edit.filter_mode = Some(mode.into());
    link(site, range, &edit, None)
}

fn fields(site: &Site, range: &Range, query: &ReportQuery, filter: &Filter) -> String {
    let mut html = hidden_state(
        site,
        range,
        query,
        &["filter", "filter_op", "filter_mode", "offset"],
    );
    let _ = write!(
        html,
        "<input type=\"hidden\" name=\"filter\" value=\"{}\">",
        filter.key
    );
    html
}

fn picker(
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    filter: &Filter,
    options: Option<&Report>,
) -> String {
    let mut html = format!(
        "<div class=\"filter-picker\" role=\"group\" aria-label=\"{} filter\">",
        DIMENSIONS
            .iter()
            .find(|(key, _, _)| *key == filter.key)
            .unwrap()
            .2
    );
    let action = format!("/sites/{}/filter", site.id);
    if query.filter_mode.as_deref() == Some("operator") {
        let _ = write!(
            html,
            "<form class=\"filter-apply operator-options\" action=\"{action}\">{}",
            fields(site, range, query, filter)
        );
        for value in &filter.values {
            let _ = write!(
                html,
                "<input type=\"hidden\" name=\"value\" value=\"{}\">",
                escape(value)
            );
        }
        for &op in crate::filters::operators(&filter.key) {
            let _ = write!(
                html,
                "<button name=\"filter_op\" value=\"{}\" aria-pressed=\"{}\">{}{}</button>",
                op.key(),
                op == filter.op,
                op.label(),
                if op == filter.op {
                    " <span aria-hidden=\"true\">✓</span>"
                } else {
                    ""
                }
            );
        }
        html.push_str("</form>");
    } else {
        let substring = matches!(filter.op, Operator::Contains | Operator::ContainsNot);
        if !substring {
            let _ = write!(
                html,
                "<form class=\"filter-search\" action=\"/sites/{}\">{}<input id=\"filter-search\" name=\"filter_search\" value=\"{}\" placeholder=\"Search…\" aria-label=\"Search filter values\" autocomplete=\"off\" maxlength=\"2048\"><button class=\"button filter-fallback\">Search</button></form>",
                site.id,
                hidden_state(site, range, query, &["filter_search", "filter_offset"]),
                escape(query.filter_search.as_deref().unwrap_or_default())
            );
        }
        let _ = write!(
            html,
            "<form class=\"filter-apply filter-values\" action=\"{action}\">{}<input type=\"hidden\" name=\"filter_op\" value=\"{}\"><div class=\"filter-options\">",
            fields(site, range, query, filter),
            filter.op.key()
        );
        let mut values: Vec<&str> = filter.values.iter().map(String::as_str).collect();
        if !substring {
            for value in options
                .into_iter()
                .flat_map(|report| rows(report, &filter.key))
            {
                if !values.contains(&value.name.as_str()) {
                    values.push(&value.name);
                }
            }
            // Page/source strings can be selected before they have any traffic.
            if matches!(
                filter.key.as_str(),
                "page"
                    | "entry"
                    | "exit"
                    | "source"
                    | "referrer"
                    | "utm_medium"
                    | "utm_source"
                    | "utm_campaign"
                    | "utm_content"
                    | "utm_term"
            ) && let Some(search) = query
                .filter_search
                .as_deref()
                .filter(|value| !value.is_empty())
                && !values.contains(&search)
            {
                values.push(search);
            }
        }
        for (index, value) in values.iter().enumerate() {
            let selected = filter.values.iter().any(|candidate| candidate == value);
            let _ = write!(
                html,
                "<label><input id=\"filter-value-{index}\" type=\"checkbox\" name=\"value\" value=\"{}\"{}><span>{}</span></label>",
                escape(value),
                if selected { " checked" } else { "" },
                escape(&value_label(filter, value))
            );
        }
        if values.is_empty() && !substring {
            html.push_str("<p class=\"muted\">No values found</p>");
        }
        html.push_str("</div>");
        if substring {
            html.push_str("<div class=\"filter-term\"><input id=\"filter-term\" name=\"term\" placeholder=\"Add text…\" aria-label=\"Text to match\" maxlength=\"2048\" autocomplete=\"off\"><button class=\"button\">Add</button></div>");
        }
        html.push_str("<button class=\"button filter-fallback\">Apply</button></form>");
        if !substring && let Some(report) = options {
            let offset = query.filter_offset.unwrap_or(0);
            if offset > 0 || offset + crate::reports::PAGE_SIZE < report.detail_total {
                html.push_str("<nav class=\"filter-pages\" aria-label=\"Filter values pages\">");
                for (label, next) in [
                    ("Previous", offset.checked_sub(crate::reports::PAGE_SIZE)),
                    (
                        "Next",
                        (offset + crate::reports::PAGE_SIZE < report.detail_total)
                            .then_some(offset + crate::reports::PAGE_SIZE),
                    ),
                ] {
                    if let Some(next) = next {
                        let mut paged = query.clone();
                        paged.filter_offset = Some(next);
                        let _ = write!(
                            html,
                            "<a data-filter-link=\"page:{label}\" href=\"{}\">{label}</a>",
                            escape(&link(site, range, &paged, None))
                        );
                    }
                }
                html.push_str("</nav>");
            }
        }
    }
    let _ = write!(
        html,
        "<a class=\"filter-done\" data-filter-link=\"done\" href=\"{}\">Done</a></div>",
        escape(&link(site, range, query, Some(("filter", None))))
    );
    html
}

pub(crate) fn render(
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    options: Option<&Report>,
) -> String {
    let mut filters = query.active_filters();
    if let Some(key) = query
        .filter
        .as_ref()
        .filter(|key| !filters.iter().any(|filter| &filter.key == *key))
    {
        filters.push(Filter {
            key: key.clone(),
            op: query.filter_op.unwrap_or_default(),
            values: Vec::new(),
        });
    }
    if filters.is_empty() {
        return String::new();
    }
    let mut html = String::from("<div class=\"filters\">");
    for filter in filters {
        let label = DIMENSIONS
            .iter()
            .find(|(key, _, _)| *key == filter.key)
            .unwrap()
            .2;
        let values = filter
            .values
            .iter()
            .map(|value| value_label(&filter, value))
            .collect::<Vec<_>>()
            .join(", ");
        let values = if values.is_empty() {
            "Select…"
        } else {
            &values
        };
        let key = &filter.key;
        let _ = write!(
            html,
            "<div class=\"filter\" data-filter-key=\"{key}\"><span class=\"filter-dimension\">{label}</span><a data-filter-link=\"operator:{key}\" class=\"filter-operator\" aria-label=\"Edit {label} operator\" href=\"{}\">{}</a><a data-filter-link=\"values:{key}\" class=\"filter-selected\" aria-label=\"Edit {label} values\" title=\"{}\" href=\"{}\">{}</a><a data-filter-link=\"remove:{key}\" class=\"filter-remove\" aria-label=\"Remove {key} filter\" href=\"{}\">×</a>",
            escape(&edit_link(site, range, query, &filter.key, "operator")),
            filter.op.label(),
            escape(values),
            escape(&edit_link(site, range, query, &filter.key, "values")),
            escape(values),
            escape(&link(site, range, query, Some((&filter.key, None))))
        );
        if query.filter.as_deref() == Some(&filter.key) {
            html.push_str(&picker(site, range, query, &filter, options));
        }
        html.push_str("</div>");
    }
    if query.filtered() {
        let _ = write!(
            html,
            "<a class=\"clear-filters\" href=\"{}\" data-filter-link=\"clear\">Clear filters</a>",
            escape(&link(site, range, &query.without_filters(), None))
        );
    }
    html.push_str("</div>");
    html
}
