use super::*;

#[test]
fn row_changes_use_percentage_points_for_bounce_and_percent_for_counts() {
    let current = Range {
        from: "2025-02-02".parse().unwrap(),
        to: "2025-02-02".parse().unwrap(),
    };
    let previous = Range {
        from: "2025-02-01".parse().unwrap(),
        to: "2025-02-01".parse().unwrap(),
    };
    let report = analytics::Report {
        sources: vec![Row {
            name: "Google".into(),
            visitors: 2,
            visits: 2,
            bounces: 1,
            duration: 30,
            ..Default::default()
        }],
        ..Default::default()
    };
    let comparison = TableComparison {
        report: &report,
        current: &current,
        previous: &previous,
    };
    let row = Row {
        name: "Google".into(),
        visitors: 3,
        visits: 3,
        bounces: 2,
        duration: 120,
        ..Default::default()
    };
    let count = comparison.value("source", &row, "visitors", "Visitors", true);
    assert!(count.contains("data-current=\"3\" data-previous=\"2\""));
    assert!(count.contains("data-change=\"↗ 50%\" data-tone=\"positive\""));
    let bounce = comparison.value("source", &row, "bounce_rate", "Bounce rate", false);
    assert!(bounce.contains("data-current=\"67%\" data-previous=\"50%\""));
    assert!(bounce.contains("data-change=\"↗ 17 pp\" data-tone=\"negative\""));
    let duration = comparison.value("source", &row, "visit_duration", "Visit duration", false);
    assert!(duration.contains("data-current=\"40s\" data-previous=\"15s\""));
    assert!(duration.contains("data-change=\"↗ 167%\""));
}

#[test]
fn new_rows_compare_with_zero_but_missing_history_remains_unknown() {
    let range = Range {
        from: "2025-02-02".parse().unwrap(),
        to: "2025-02-02".parse().unwrap(),
    };
    let mut report = analytics::Report::default();
    let row = Row {
        name: "New".into(),
        visitors: 3,
        visits: 3,
        bounces: 1,
        duration: 90,
        percentage: Some((3, 10)),
        ..Default::default()
    };
    let render = |report: &analytics::Report, column| {
        TableComparison {
            report,
            current: &range,
            previous: &range,
        }
        .value("source", &row, column, "Visitors", false)
    };
    assert!(render(&report, "visitors").contains("data-change=\"↗ 100%\""));
    assert!(render(&report, "percentage").contains("data-previous=\"0.0%\""));
    let bounce = render(&report, "bounce_rate");
    assert!(bounce.contains("data-previous=\"0%\""));
    assert!(bounce.contains("data-change=\"\""));
    let duration = render(&report, "visit_duration");
    assert!(duration.contains("data-current=\"30s\" data-previous=\"—\""));
    assert!(duration.contains("data-change=\"\""));
    report.sources.push(Row {
        name: "New".into(),
        visits: 1,
        ..Default::default()
    });
    assert!(render(&report, "bounce_rate").contains("data-change=\"↗ 33 pp\""));
    assert!(render(&report, "visit_duration").contains("data-change=\"↗ 100%\""));
    report.sources.clear();
    report.comparison_missing.insert("source".into());
    assert!(render(&report, "visitors").contains("data-previous=\"—\""));
    assert!(render(&report, "visitors").contains("data-change=\"\""));
    report.sources.push(Row {
        name: "New".into(),
        visitors: 0,
        unavailable: 1,
        ..Default::default()
    });
    assert!(render(&report, "visitors").contains("data-previous=\"—\""));
    assert!(render(&report, "visitors").contains("data-change=\"\""));
}

#[test]
fn row_tooltip_values_are_escaped_and_retain_exact_dates_and_counts() {
    let range = Range {
        from: "2025-01-01".parse().unwrap(),
        to: "2025-02-01".parse().unwrap(),
    };
    let report = analytics::Report::default();
    let row = Row {
        name: "<script>\"&".into(),
        visitors: 12345,
        ..Default::default()
    };
    let html = TableComparison {
        report: &report,
        current: &range,
        previous: &range,
    }
    .value("source", &row, "visitors", "Visitors", true);
    assert!(!html.contains("<script>"));
    assert!(html.contains("data-name=\"&lt;script&gt;&quot;&amp;\""));
    assert!(html.contains("data-current=\"12,345\""));
    assert!(html.contains("data-current-date=\"1 Jan 2025 – 1 Feb 2025\""));
    assert!(html.contains("title=\""));
}
