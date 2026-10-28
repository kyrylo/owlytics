use crate::analytics::{Metric, Row};

pub(crate) fn label(dimension: &str, name: &str) -> String {
    if name.is_empty() {
        if dimension == "source" || dimension == "referrer" {
            "Direct / None".into()
        } else {
            "(not set)".into()
        }
    } else if dimension == "source" {
        crate::analytics::source_name(name)
    } else {
        crate::geography::label(dimension, name)
    }
}

pub(crate) fn columns(dimension: &str, pageviews: bool) -> Vec<(&'static str, &'static str)> {
    let mut columns = vec![(
        "visitors",
        match dimension {
            "entry" => "Unique entrances",
            "exit" => "Unique exits",
            _ => "Visitors",
        },
    )];
    if dimension == "exit" {
        columns.extend([("visits", "Total exits"), ("exit_rate", "Exit rate")]);
    } else {
        if dimension == "page" {
            columns.push(("pageviews", "Pageviews"));
        }
        if dimension == "entry" {
            columns.push(("visits", "Total entrances"));
        }
        columns.push(("bounce_rate", "Bounce rate"));
        if dimension == "page" {
            columns.extend([
                ("time_on_page", "Time on page"),
                ("scroll_depth", "Scroll depth"),
            ]);
        } else {
            columns.push(("visit_duration", "Visit duration"));
        }
    }
    if pageviews {
        // Old exports may contain only pageviews. Keep that known count visible
        // instead of inventing visitors, without duplicating the Top Pages column.
        if columns.iter().any(|(key, _)| *key == "pageviews") {
            columns.retain(|(key, _)| *key != "visitors");
        } else {
            columns[0] = ("pageviews", "Pageviews");
        }
    }
    columns
}

pub(crate) fn value(row: &Row, column: &str) -> Option<f64> {
    let metric = match column {
        "percentage" => {
            return row.percentage.map(|(count, total)| {
                if total > 0 {
                    100. * count as f64 / total as f64
                } else {
                    0.
                }
            });
        }
        "visitors" => Metric::Visitors,
        "pageviews" => Metric::Pageviews,
        "visits" => Metric::Visits,
        "bounce_rate" => Metric::BounceRate,
        "visit_duration" => Metric::VisitDuration,
        "time_on_page" => Metric::TimeOnPage,
        "scroll_depth" => Metric::ScrollDepth,
        "exit_rate" => {
            return (row.unavailable & 16 == 0).then(|| {
                if row.pageviews > 0 {
                    row.exits as f64 * 100. / row.pageviews as f64
                } else {
                    0.
                }
            });
        }
        _ => return None,
    };
    (row.available(metric) && (metric != Metric::VisitDuration || row.mean_visits() > 0))
        .then(|| metric.value(row))
}

pub(crate) fn display(row: &Row, column: &str) -> String {
    let Some(value) = value(row, column) else {
        return "—".into();
    };
    match column {
        "percentage" => format!("{value:.1}%"),
        "bounce_rate" | "exit_rate" | "scroll_depth" => format!("{value:.0}%"),
        "visit_duration" | "time_on_page" => {
            let seconds = value.round() as i64;
            if seconds >= 60 {
                format!("{}m {:02}s", seconds / 60, seconds % 60)
            } else {
                format!("{seconds}s")
            }
        }
        _ => {
            let raw = format!("{value:.0}");
            let mut out = String::new();
            for (i, ch) in raw.chars().enumerate() {
                if i > 0 && (raw.len() - i).is_multiple_of(3) {
                    out.push(',');
                }
                out.push(ch);
            }
            out
        }
    }
}
