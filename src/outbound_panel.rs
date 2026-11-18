use crate::{
    analytics::{Range, ReportQuery, Site},
    dashboard::{hidden_state, link},
    outbound::{GOAL, Report},
    web::escape,
};
use std::fmt::Write;

fn rate(uniques: i64, eligible: i64) -> String {
    format!(
        "{:.1}%",
        if eligible > 0 {
            100. * uniques as f64 / eligible as f64
        } else {
            0.
        }
    )
}

pub(crate) fn render(site: &Site, range: &Range, query: &ReportQuery, report: &Report) -> String {
    let selected = query.goal.is_some();
    let detail = if selected { "outbound" } else { "goals" };
    let heading = if selected { "url" } else { "Goal conversions" };
    let expand = escape(&link(site, range, query, Some(("detail", Some(detail)))));
    let mut out = format!(
        "<section class=\"panel report-panel panel-outbound\"><div class=\"report-tabs\"><a href=\"{}\" aria-current=\"page\">Goals</a>{}<a class=\"expand-report\" href=\"{expand}\" aria-label=\"Expand {heading} report\">{}</a></div>",
        escape(&link(site, range, query, Some(("goal", None)))),
        if selected {
            "<span class=\"goal-property\">url</span>"
        } else {
            ""
        },
        crate::icons::control("expand")
    );
    out.push_str(&table(site, range, query, report, false));
    if selected && report.total > 9 {
        let _ = write!(
            out,
            "<a class=\"outbound-more\" href=\"{expand}\">View all</a>"
        );
    }
    out.push_str("</section>");
    if query.detail.as_deref() != Some(detail) {
        return out;
    }
    let close = escape(&link(site, range, query, Some(("detail", None))));
    let _ = write!(
        out,
        "<div class=\"report-overlay\"><a class=\"report-backdrop\" href=\"{close}\" tabindex=\"-1\"></a><section class=\"panel report-panel panel-outbound expanded-report\"><header class=\"report-heading\"><h2>{heading}</h2><form class=\"report-search\" method=\"get\">{}<input id=\"report-search\" type=\"search\" name=\"search\" aria-label=\"Search report\" placeholder=\"Press / to search\" value=\"{}\"><button class=\"quiet\">Search</button></form><a class=\"close-report\" href=\"{close}\" aria-label=\"Close expanded report\">{}</a></header>",
        hidden_state(site, range, query, &["search", "offset"]),
        escape(query.search.as_deref().unwrap_or("")),
        crate::icons::control("close")
    );
    if selected {
        let _ = write!(out, "<p class=\"report-note\">{GOAL}</p>");
    }
    out.push_str(&table(site, range, query, report, true));
    if selected {
        let offset = query.offset.unwrap_or(0);
        out.push_str("<nav class=\"report-pagination\" aria-label=\"Report pages\">");
        if offset > 0 {
            let mut q = query.clone();
            q.offset = Some(offset.saturating_sub(crate::reports::PAGE_SIZE));
            let _ = write!(
                out,
                "<a href=\"{}\">← Previous</a>",
                escape(&link(site, range, &q, None))
            );
        }
        let _ = write!(
            out,
            "<span>{}–{} of {}</span>",
            if report.rows.is_empty() {
                0
            } else {
                offset + 1
            },
            if report.rows.is_empty() {
                0
            } else {
                offset + report.rows.len()
            },
            report.total
        );
        if offset + report.rows.len() < report.total {
            let mut q = query.clone();
            q.offset = Some(offset + crate::reports::PAGE_SIZE);
            let _ = write!(
                out,
                "<a href=\"{}\">Next →</a>",
                escape(&link(site, range, &q, None))
            );
        }
        out.push_str("</nav>");
    }
    out.push_str("</section></div>");
    out
}

fn table(
    site: &Site,
    range: &Range,
    query: &ReportQuery,
    report: &Report,
    expanded: bool,
) -> String {
    let selected = query.goal.is_some();
    let mut out = String::from(
        "<div class=\"report-table-scroll\" tabindex=\"0\" role=\"region\" aria-label=\"Goal conversions report\"><table class=\"report-table outbound-table\"><thead><tr>",
    );
    for (key, label, description) in [
        ("name", if selected { "URL" } else { "Goal" }, ""),
        ("visitors", "Uniques", "Unique conversions"),
        ("clicks", "Total", "Total conversions"),
        ("conversion_rate", "CR", "Conversion rate"),
    ] {
        let active = query.sort.as_deref().unwrap_or("visitors") == key;
        let ascending = query.order.as_deref() == Some("asc");
        if expanded {
            let mut next = query.clone();
            next.sort = Some(key.into());
            next.order = Some(
                if (active && !ascending) || (!active && key == "name") {
                    "asc"
                } else {
                    "desc"
                }
                .into(),
            );
            next.offset = None;
            let _ = write!(
                out,
                "<th scope=\"col\" title=\"{description}\" aria-sort=\"{}\"><a href=\"{}\">{label}{}</a></th>",
                if active {
                    if ascending { "ascending" } else { "descending" }
                } else {
                    "none"
                },
                escape(&link(site, range, &next, None)),
                if active {
                    if ascending { " ↑" } else { " ↓" }
                } else {
                    ""
                }
            );
        } else {
            let _ = write!(
                out,
                "<th scope=\"col\" title=\"{description}\">{label}</th>"
            );
        }
    }
    out.push_str("</tr></thead><tbody>");
    let mut count = 0;
    if selected {
        for row in report
            .rows
            .iter()
            .take(if expanded { usize::MAX } else { 9 })
        {
            let _ = write!(
                out,
                "<tr><th scope=\"row\"><a href=\"{}\" title=\"Filter by url {}\">{}</a><a class=\"open-destination\" href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\" aria-label=\"Open URL in new tab\">↗</a></th><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape(&link(site, range, query, Some(("url", Some(&row.url))))),
                escape(&row.url),
                escape(&row.url),
                escape(&row.url),
                row.visitors,
                row.clicks,
                rate(row.visitors, report.eligible)
            );
            count += 1;
        }
    } else if report.clicks > 0
        && (!expanded
            || query
                .search
                .as_deref()
                .is_none_or(|s| GOAL.to_lowercase().contains(&s.to_lowercase())))
    {
        let _ = write!(
            out,
            "<tr><th scope=\"row\"><a href=\"{}\">{GOAL}</a></th><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(&link(site, range, query, Some(("goal", Some(GOAL))))),
            report.visitors,
            report.clicks,
            rate(report.visitors, report.eligible)
        );
        count = 1;
    }
    if count == 0 {
        out.push_str("<tr><td colspan=\"4\" class=\"no-data\">No matching results</td></tr>");
    }
    out.push_str("</tbody></table></div>");
    out
}

pub(crate) fn filters(site: &Site, range: &Range, query: &ReportQuery) -> String {
    if query.goal.is_none() {
        return String::new();
    }
    let mut out = format!(
        "<div class=\"goal-filters\"><div class=\"filter\"><span class=\"filter-dimension\">Goal</span><span class=\"filter-operator\">is</span><span class=\"goal-filter-value\">{GOAL}</span><a class=\"filter-remove\" href=\"{}\" aria-label=\"Remove goal filter\">×</a></div>",
        escape(&link(site, range, query, Some(("goal", None))))
    );
    if let Some(url) = &query.url {
        let _ = write!(
            out,
            "<div class=\"filter\"><span class=\"filter-dimension\">url</span><span class=\"filter-operator\">is</span><span class=\"goal-filter-value\">{}</span><a class=\"filter-remove\" href=\"{}\" aria-label=\"Remove url filter\">×</a></div>",
            escape(url),
            escape(&link(site, range, query, Some(("url", None))))
        );
    }
    out.push_str("</div>");
    out
}
