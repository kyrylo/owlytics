use crate::analytics::{Metric, Range, Report, ReportQuery, Row, Site};
use crate::timeline::Bucket;

fn stamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp()
}

fn site(zone: &str) -> Site {
    Site {
        id: 1,
        domain: "example.com".into(),
        timezone: zone.into(),
    }
}

fn daily(from: &str, to: &str, now: &str) -> (Report, Range) {
    let range = Range {
        from: from.parse().unwrap(),
        to: to.parse().unwrap(),
    };
    let report = Report {
        generated_at: stamp(now),
        daily: (0..=(range.to - range.from).num_days())
            .map(|i| Row {
                name: (range.from + chrono::Duration::days(i)).to_string(),
                visitors: 1,
                pageviews: 1,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    (report, range)
}

fn draw(report: &Report, range: &Range, zone: &str, interval: &str) -> String {
    super::chart(
        report,
        range,
        &ReportQuery {
            interval: Some(interval.into()),
            metric: Metric::Pageviews,
            ..Default::default()
        },
        &site(zone),
        None,
    )
}

#[test]
fn visit_series_uses_session_counts_but_page_filters_use_matching_arrivals() {
    let (mut report, range) = daily("2026-10-26", "2026-10-27", "2026-10-28T12:00:00Z");
    report.daily[0] = Row {
        name: "2026-10-26".into(),
        session_visits: Some(2),
        session_pageviews: Some(6),
        duration: 600,
        bounces: 1,
        ..Default::default()
    };
    report.daily[1] = Row {
        name: "2026-10-27".into(),
        visits: 2,
        pageviews: 2,
        session_visits: Some(1),
        session_pageviews: Some(4),
        duration: 120,
        ..Default::default()
    };
    for (metric, page, value) in [
        (Metric::Visits, None, "3"),
        (Metric::Visits, Some("/".into()), "2"),
        (Metric::ViewsPerVisit, None, "3.33"),
        (Metric::VisitDuration, None, "4m 00s"),
    ] {
        let html = super::chart(
            &report,
            &range,
            &ReportQuery {
                interval: Some("week".into()),
                metric,
                page,
                ..Default::default()
            },
            &site("UTC"),
            None,
        );
        let point = html.split("class=\"chart-hit\"").nth(1).unwrap();
        let actual = point
            .split("data-value=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_eq!(actual, value, "{metric:?}");
    }
}

#[test]
fn completed_partial_weeks_and_months_have_labels_but_no_live_stroke() {
    let (report, range) = daily("2024-02-03", "2024-02-14", "2026-10-27T12:00:00Z");
    let html = draw(&report, &range, "UTC", "week");
    for (label, views) in [
        ("Partial week of 3 Feb 24", 2),
        ("Week of 5 Feb 24", 7),
        ("Partial week of 12 Feb 24", 3),
    ] {
        assert!(
            html.contains(&format!("data-date=\"{label}\" data-value=\"{views}\"")),
            "{label}"
        );
    }
    assert!(!html.contains("chart-incomplete"));
    assert!(!html.contains("data-incomplete=\"true\""));
    let (report, range) = daily("2024-02-01", "2024-02-29", "2026-10-27T12:00:00Z");
    let html = draw(&report, &range, "UTC", "month");
    assert!(html.contains("data-date=\"February 2024\" data-value=\"29\""));
    assert!(html.contains("class=\"axis-long\">February 2024"));
    assert!(html.contains("class=\"axis-short\">Feb 24"));
    assert!(!html.contains("Partial"));
}

#[test]
fn only_current_intervals_are_dotted_and_future_dates_are_not_plotted() {
    let (report, range) = daily("2026-10-01", "2026-10-31", "2026-10-27T12:00:00Z");
    for (interval, count, label, dotted) in [
        ("day", 27, "Tue, 27 Oct 2026", 1),
        ("week", 5, "Partial week of 26 Oct", 1),
        ("month", 1, "Partial of October 2026", 0),
    ] {
        let html = draw(&report, &range, "UTC", interval);
        assert_eq!(html.matches("class=\"chart-hit\"").count(), count);
        assert_eq!(html.matches("data-incomplete=\"true\"").count(), 1);
        assert_eq!(
            html.matches("class=\"chart-line chart-incomplete\"")
                .count(),
            dotted
        );
        assert!(html.contains(&format!("data-date=\"{label}\"")));
        assert!(!html.contains("data-date=\"Wed, 28 Oct 2026\""));
    }
    let (mut report, range) = daily("2026-10-26", "2026-11-01", "2026-11-01T23:59:59Z");
    assert!(draw(&report, &range, "UTC", "week").contains("Partial week of 26 Oct"));
    report.generated_at += 1;
    let html = draw(&report, &range, "UTC", "week");
    assert!(html.contains("data-date=\"Week of 26 Oct\""));
    assert!(!html.contains("data-incomplete=\"true\""));
    let (report, range) = daily("2026-10-26", "2026-10-28", "2026-10-27T15:00:00Z");
    assert_eq!(
        draw(&report, &range, "UTC", "day")
            .matches("class=\"chart-hit\"")
            .count(),
        2
    );
    assert_eq!(
        draw(&report, &range, "Asia/Tokyo", "day")
            .matches("class=\"chart-hit\"")
            .count(),
        3
    );
}

#[test]
fn minute_axes_use_round_site_local_ticks_at_the_correct_dst_positions() {
    let (mut report, range) = daily("2025-11-02", "2025-11-02", "2025-11-04T00:00:00Z");
    report.timeline_start = stamp("2025-11-02T04:00:00Z");
    report.timeline_end = stamp("2025-11-03T05:00:00Z");
    report.timeline_step = 60;
    report.timeline = (report.timeline_start..report.timeline_end)
        .step_by(60)
        .map(|start| Bucket {
            start,
            end: start + 60,
            pageviews: 1,
            ..Default::default()
        })
        .collect();
    let html = draw(&report, &range, "America/New_York", "minute");
    assert_eq!(html.matches("class=\"chart-hit\"").count(), 1500);
    assert!(html.contains("data-date=\"Sun, 02 Nov 2025, 01:00 EDT\""));
    assert!(html.contains("data-date=\"Sun, 02 Nov 2025, 01:00 EST\""));
    let axis = html
        .split("class=\"chart-labels chart-labels-positioned\"")
        .nth(1)
        .unwrap();
    for time in ["00:00", "04:00", "08:00", "12:00", "16:00", "20:00"] {
        assert!(axis.contains(&format!("data-clock=\"{time}\"")), "{time}");
    }
    assert!(axis.contains("x=\"20.01%\" y=\"1em\" text-anchor=\"middle\" data-clock=\"04:00\""));
    assert!(!axis.contains("03:59"));
    assert!(!html.contains("chart-incomplete"));
}

#[test]
fn live_hour_and_completed_realtime_minutes_use_different_strokes() {
    let (mut report, range) = daily("2025-11-02", "2025-11-02", "2025-11-02T06:30:00Z");
    report.timeline_start = stamp("2025-11-02T04:00:00Z");
    report.timeline_end = report.generated_at + 1;
    report.timeline_step = 3600;
    report.timeline = (0..25)
        .map(|i| {
            let start = report.timeline_start + i * 3600;
            Bucket {
                start,
                end: start + 3600,
                pageviews: 1,
                ..Default::default()
            }
        })
        .collect();
    let html = draw(&report, &range, "America/New_York", "hour");
    assert_eq!(html.matches("class=\"chart-hit\"").count(), 3);
    assert_eq!(html.matches("chart-incomplete").count(), 1);
    assert!(html.contains("Sun, 02 Nov 2025, 01:00 EDT"));
    assert!(html.contains("Sun, 02 Nov 2025, 01:00 EST"));
    report.realtime = true;
    report.timeline_start = report.generated_at - 1800;
    report.timeline_end = report.generated_at;
    report.timeline_step = 60;
    report.timeline = (report.timeline_start..report.timeline_end)
        .step_by(60)
        .map(|start| Bucket {
            start,
            end: start + 60,
            pageviews: 1,
            ..Default::default()
        })
        .collect();
    let html = draw(&report, &range, "America/New_York", "minute");
    assert_eq!(html.matches("class=\"chart-hit\"").count(), 30);
    assert!(!html.contains("chart-incomplete"));
    assert!(!html.contains("data-incomplete=\"true\""));
    for minute in [30, 25, 20, 15, 10, 5] {
        assert!(html.contains(&format!("<span>-{minute}m</span>")));
    }
}

#[tokio::test]
async fn rolling_chart_retains_the_requested_observation_time() {
    let db = crate::db::Database::open(":memory:").unwrap();
    db.call(|c| c.execute("INSERT INTO sites VALUES(1,'example.com','UTC',0)", []))
        .await
        .unwrap();
    let end = stamp("2025-11-02T10:15:01Z");
    let range = Range {
        from: "2025-11-01".parse().unwrap(),
        to: "2025-11-02".parse().unwrap(),
    };
    let query = ReportQuery {
        period: Some("24h".into()),
        window: Some((end - 86400, end)),
        interval: Some("hour".into()),
        metric: Metric::Pageviews,
        ..Default::default()
    };
    let report = crate::analytics::report(&db, 1, range.clone(), query.clone())
        .await
        .unwrap();
    assert_eq!(
        report.generated_at,
        end - 1,
        "Rendering later must not move a rolling request's clock"
    );
    let html = super::chart(&report, &range, &query, &site("UTC"), None);
    assert_eq!(html.matches("chart-incomplete").count(), 1);
    assert!(html.contains("data-date=\"Sun, 02 Nov 2025, 10:00 UTC\""));
}
