use crate::analytics::{Metric, Range, Report, Row, Site};
use std::io::{Cursor, Write};
use zip::{ZipWriter, write::SimpleFileOptions};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Csv(#[from] csv::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

// Spreadsheet applications may execute cells beginning with a formula marker.
// CSV quoting alone does not prevent that for untrusted report labels.
fn cell(value: &str) -> String {
    if value.trim_start().starts_with(['=', '+', '-', '@']) || value.starts_with(['\t', '\r', '\n'])
    {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

fn metrics(row: &Row, page_filtered: bool) -> Vec<String> {
    metric_values(page_filtered, |metric| {
        row.available(metric).then(|| metric.value(row))
    })
}

fn metric_values(page_filtered: bool, mut value: impl FnMut(Metric) -> Option<f64>) -> Vec<String> {
    [
        Metric::Visitors,
        Metric::Visits,
        Metric::Pageviews,
        if page_filtered {
            Metric::ScrollDepth
        } else {
            Metric::ViewsPerVisit
        },
        Metric::BounceRate,
        if page_filtered {
            Metric::TimeOnPage
        } else {
            Metric::VisitDuration
        },
    ]
    .into_iter()
    .map(|metric| {
        let Some(value) = value(metric) else {
            return String::new();
        };
        match metric {
            Metric::ViewsPerVisit | Metric::BounceRate | Metric::ScrollDepth => {
                format!("{value:.2}")
            }
            Metric::VisitDuration | Metric::TimeOnPage => format!("{value:.1}"),
            _ => format!("{value:.0}"),
        }
    })
    .collect()
}

pub(crate) fn archive(
    site: &Site,
    range: &Range,
    report: &Report,
    page_filtered: bool,
    interval: Option<&str>,
) -> Result<Vec<u8>, Error> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let headers = if report.realtime {
        vec!["current_visitors", "visitors", "pageviews"]
    } else {
        vec![
            "visitors",
            "visits",
            "pageviews",
            if page_filtered {
                "scroll_depth_percent"
            } else {
                "views_per_visit"
            },
            "bounce_rate_percent",
            if page_filtered {
                "time_on_page_seconds"
            } else {
                "visit_duration_seconds"
            },
        ]
    };
    zip.start_file("summary.csv", options)?;
    {
        let mut csv = csv::Writer::from_writer(&mut zip);
        csv.write_record(
            ["site", "from", "to"]
                .into_iter()
                .chain(headers.iter().copied()),
        )?;
        csv.write_record(
            [
                site.domain.clone(),
                range.from.to_string(),
                range.to.to_string(),
            ]
            .into_iter()
            .chain(if report.realtime {
                vec![
                    report.current_visitors.to_string(),
                    report.visitors.to_string(),
                    report.pageviews.to_string(),
                ]
            } else {
                metrics(&report.totals, page_filtered)
            }),
        )?;
        csv.flush()?;
    }
    zip.start_file("visitors.csv", options)?;
    {
        let mut csv = csv::Writer::from_writer(&mut zip);
        csv.write_record(
            ["date"].into_iter().chain(
                headers
                    .iter()
                    .copied()
                    .filter(|key| *key != "current_visitors"),
            ),
        )?;
        if !report.timeline.is_empty() {
            let zone = site
                .timezone
                .parse::<chrono_tz::Tz>()
                .expect("validated timezone");
            for bucket in &report.timeline {
                let date = chrono::DateTime::from_timestamp(bucket.start, 0)
                    .unwrap()
                    .with_timezone(&zone)
                    .to_rfc3339();
                csv.write_record([date].into_iter().chain(if report.realtime {
                    vec![bucket.visitors.to_string(), bucket.pageviews.to_string()]
                } else {
                    metric_values(page_filtered, |metric| {
                        if metric == Metric::VisitDuration
                            && (bucket.visits > 0 || bucket.pageviews > 0)
                        {
                            Some(metric.value(&bucket.completed))
                        } else {
                            bucket.value(metric)
                        }
                    })
                }))?;
            }
        } else {
            let calendar: std::collections::BTreeMap<_, _> = report
                .calendar
                .as_ref()
                .unwrap_or(&report.daily)
                .iter()
                .map(|row| (row.name.as_str(), row))
                .collect();
            let mut day = range.from;
            let mut last_key = None;
            loop {
                let key = crate::calendar::interval_key(interval, &day.to_string());
                if last_key.as_ref() != Some(&key) {
                    let empty = Row {
                        unavailable: 8,
                        ..Default::default()
                    };
                    let row = calendar.get(key.as_str()).copied().unwrap_or(&empty);
                    // A clipped first week starts at the selected date; month
                    // labels retain the first day of the calendar month.
                    let label = if interval == Some("week") {
                        key.clone().max(range.from.to_string())
                    } else {
                        key.clone()
                    };
                    csv.write_record([label].into_iter().chain(metrics(row, page_filtered)))?;
                    last_key = Some(key);
                }
                if day >= range.to {
                    break;
                }
                day = day.succ_opt().expect("validated range");
            }
        }
        csv.flush()?;
    }
    for (file, dimension, rows) in [
        ("pages", "page", report.pages.as_slice()),
        ("entry_pages", "entry", &report.entry_pages),
        ("exit_pages", "exit", &report.exit_pages),
        ("sources", "source", &report.sources),
        ("referrers", "referrer", &report.referrers),
        ("channels", "channel", report.acquisition_rows("channel")),
        (
            "utm_mediums",
            "utm_medium",
            report.acquisition_rows("utm_medium"),
        ),
        (
            "utm_sources",
            "utm_source",
            report.acquisition_rows("utm_source"),
        ),
        (
            "utm_campaigns",
            "utm_campaign",
            report.acquisition_rows("utm_campaign"),
        ),
        (
            "utm_contents",
            "utm_content",
            report.acquisition_rows("utm_content"),
        ),
        ("utm_terms", "utm_term", report.acquisition_rows("utm_term")),
        ("countries", "country", &report.countries),
        ("regions", "region", &report.regions),
        ("cities", "city", &report.cities),
        ("browsers", "browser", &report.browsers),
        (
            "browser_versions",
            "browser_version",
            &report.browser_versions,
        ),
        ("operating_systems", "os", &report.operating_systems),
        (
            "operating_system_versions",
            "os_version",
            &report.os_versions,
        ),
        ("devices", "device", &report.devices),
    ] {
        zip.start_file(format!("{file}.csv"), options)?;
        let columns = crate::report_table::columns(dimension, false);
        let mut csv = csv::Writer::from_writer(&mut zip);
        csv.write_record(
            [dimension]
                .into_iter()
                .chain(columns.iter().map(|(key, _)| *key)),
        )?;
        for row in rows {
            let values = columns.iter().map(|(key, _)| {
                crate::report_table::value(row, key)
                    .map(|v| {
                        if matches!(
                            *key,
                            "bounce_rate"
                                | "percentage"
                                | "exit_rate"
                                | "visit_duration"
                                | "time_on_page"
                                | "scroll_depth"
                        ) {
                            format!("{v:.2}")
                        } else {
                            format!("{v:.0}")
                        }
                    })
                    .unwrap_or_default()
            });
            csv.write_record(
                [cell(&crate::report_table::label(dimension, &row.name))]
                    .into_iter()
                    .chain(values),
            )?;
        }
        csv.flush()?;
    }
    zip.start_file("readme.txt", options)?;
    write!(
        zip,
        "Owlytics dashboard export\nSite: {}\nTimezone: {}\nDates: {} to {} (inclusive)\n\nReports respect dashboard date and audience filters. Report search, pagination, chart metric and comparison do not restrict the export.\nPage reports: up to 100 rows each. Other breakdowns: up to 300 rows each, sorted by visitors (pageviews for older exports without visitors).\nRates are percentages; durations are seconds. Empty numeric cells mean the historical aggregate is unavailable.\nImported history included: {}\nImported history excluded by filters or rolling window: {}\n\nThis is a dashboard snapshot. Use the backup command for full-fidelity backup and restore.\n",
        site.domain,
        site.timezone,
        range.from,
        range.to,
        report.has_imports,
        report.imports_excluded
    )?;
    writeln!(
        zip,
        "Imported history switched off: {}",
        report.imports_disabled
    )?;
    if report.realtime {
        writeln!(
            zip,
            "Realtime: current visitors cover the last 5 minutes without audience filters. Summary visitors and pageviews cover the last 30 minutes; the chart omits the unfinished minute. Breakdown tables and their percentages cover the last 5 minutes and respect audience filters."
        )?;
    }
    writeln!(
        zip,
        "UTM tables include tagged visits only. Acquisition counts omit visits whose campaign fields were not captured. Imported visitor counters retain their export granularity."
    )?;
    if !report.imports_excluded_dimensions.is_empty() {
        writeln!(
            zip,
            "Historical data partly or wholly excluded from: {}",
            report
                .imports_excluded_dimensions
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    if !report.acquisition_incomplete.is_empty() {
        writeln!(
            zip,
            "Incomplete acquisition history: {}",
            report
                .acquisition_incomplete
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    if report
        .cities
        .iter()
        .any(|row| crate::geography::imported_city(&row.name).is_some())
    {
        write!(
            zip,
            "\nCity names derived from GeoNames (https://www.geonames.org/), copyright GeoNames contributors, CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/). City and administrative names are combined for display; identifiers and counts are unchanged.\n"
        )?;
    }
    if report.timeline_start > 0 {
        writeln!(
            zip,
            "Time bounds: {} to {} (UTC Unix seconds, end exclusive)",
            report.timeline_start, report.timeline_end
        )?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::cell;
    #[test]
    fn spreadsheet_formulas_are_neutralized_without_changing_paths() {
        for input in [
            "=HYPERLINK(\"x\")",
            " +cmd",
            "-1+2",
            "@SUM(1)",
            "\ranything",
            "\tvalue",
        ] {
            assert!(cell(input).starts_with('\''));
        }
        assert_eq!(cell("/docs?q=1"), "/docs?q=1");
        assert_eq!(cell("Google"), "Google");
    }
}
