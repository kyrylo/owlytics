//! Civil-time chart buckets and their distinct traffic/session measurements.
use crate::analytics::{Metric, ReportQuery, Row};
use chrono::{Duration, LocalResult, TimeZone};
use chrono_tz::Tz;
use rusqlite::{Connection, types::Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bucket {
    pub start: i64,
    pub end: i64,
    pub visitors: i64,
    pub visits: i64,
    pub pageviews: i64,
    /// Session metrics belong to the bucket containing the last observed action.
    /// Engagement measurements retain their parent pageview's bucket.
    pub completed: Row,
}
impl Bucket {
    pub fn value(&self, metric: Metric) -> Option<f64> {
        match metric {
            Metric::Visitors => Some(self.visitors as f64),
            Metric::Visits => Some(self.visits as f64),
            Metric::Pageviews => Some(self.pageviews as f64),
            Metric::VisitDuration if self.completed.visits == 0 => None,
            _ => self
                .completed
                .available(metric)
                .then(|| metric.value(&self.completed)),
        }
    }
}

pub(crate) struct Window {
    pub start: i64,
    pub end: i64,
    pub step: i64,
    pub zone: Tz,
}
pub(crate) struct Input<'a> {
    pub from: &'a str,
    pub values: &'a [Value],
}

pub(crate) fn day_start(zone: Tz, day: chrono::NaiveDate) -> i64 {
    let midnight = day.and_hms_opt(0, 0, 0).unwrap();
    (0..=2880)
        .find_map(|minute| {
            zone.from_local_datetime(&(midnight + Duration::minutes(minute)))
                .earliest()
        })
        .expect("a civil date resolves within two days")
        .timestamp()
}

pub(crate) fn local_time(zone: Tz, time: chrono::NaiveDateTime) -> i64 {
    use chrono::Timelike;
    if let Some(resolved) = zone.from_local_datetime(&time).earliest() {
        return resolved.timestamp();
    }
    let minute = time.with_second(0).unwrap().with_nanosecond(0).unwrap();
    (1..=2880)
        .find_map(|shift| {
            zone.from_local_datetime(&(minute + Duration::minutes(shift)))
                .earliest()
        })
        .expect("a civil time resolves within two days")
        .timestamp()
}

fn boundaries(window: &Window) -> Vec<Bucket> {
    let date = |stamp| {
        chrono::DateTime::from_timestamp(stamp, 0)
            .unwrap()
            .with_timezone(&window.zone)
            .date_naive()
    };
    let first = date(window.start);
    let last = date(window.end - 1).succ_opt().unwrap();
    let mut starts = Vec::new();
    for offset in 0..=(last - first).num_days() {
        let day = first + Duration::days(offset);
        for minute in (0..1440).step_by((window.step / 60) as usize) {
            let local = day.and_hms_opt(minute / 60, minute % 60, 0).unwrap();
            // Some zones advance by half an hour. Keep the partial civil hour
            // beginning after that gap, rather than shifting all later labels.
            for adjustment in 0..window.step / 60 {
                match window
                    .zone
                    .from_local_datetime(&(local + Duration::minutes(adjustment)))
                {
                    LocalResult::Single(time) => {
                        starts.push(time.timestamp());
                        break;
                    }
                    LocalResult::Ambiguous(first, second) => {
                        starts.extend([first.timestamp(), second.timestamp()]);
                        break;
                    }
                    LocalResult::None => {}
                }
            }
        }
    }
    starts.sort_unstable();
    starts.dedup();
    starts
        .windows(2)
        .filter(|pair| pair[1] > window.start && pair[0] < window.end)
        .map(|pair| Bucket {
            start: pair[0],
            end: pair[1],
            ..Default::default()
        })
        .collect()
}

fn containing(buckets: &[Bucket], window: &Window, stamp: i64) -> Option<usize> {
    if !(window.start..window.end).contains(&stamp) {
        return None;
    }
    let index = buckets
        .partition_point(|bucket| bucket.start <= stamp)
        .checked_sub(1)?;
    (stamp < buckets[index].end).then_some(index)
}

pub(crate) fn build(
    c: &Connection,
    window: Window,
    query: &ReportQuery,
    events: Input<'_>,
    sessions: Input<'_>,
) -> rusqlite::Result<Vec<Bucket>> {
    let mut buckets = boundaries(&window);
    if query.has_filter("page") {
        for bucket in &mut buckets {
            bucket.completed.entrances = Some(0);
        }
    }
    let event_counts = crate::calendar::event_counts(query, window.step);
    let mut last_event_visitor = vec![None; buckets.len()];
    let mut last_event_session = vec![None; buckets.len()];
    let mut event_statement = c.prepare(&format!(
        "SELECT e.received_at{} {}{}",
        if event_counts {
            ",e.visitor,e.session_id"
        } else {
            ""
        },
        events.from,
        if event_counts {
            " ORDER BY e.visitor,e.session_id"
        } else {
            ""
        }
    ))?;
    let mut rows = event_statement.query(rusqlite::params_from_iter(events.values))?;
    while let Some(row) = rows.next()? {
        if let Some(index) = containing(&buckets, &window, row.get(0)?) {
            buckets[index].pageviews += 1;
            if event_counts {
                let visitor: [u8; 16] = row.get(1)?;
                let session: Option<i64> = row.get(2)?;
                if last_event_visitor[index] != Some(visitor) {
                    buckets[index].visitors += 1;
                    last_event_visitor[index] = Some(visitor);
                }
                if session.is_some() && last_event_session[index] != session {
                    buckets[index].visits += 1;
                    last_event_session[index] = session;
                }
            }
        }
    }
    drop(rows);
    drop(event_statement);

    // A visitor can have several visits in one bucket. Ordering by visitor
    // permits exact deduplication with one boundary ID per bucket, not a set.
    let mut last_visitor = vec![None; buckets.len()];
    let mut statement = c.prepare(&format!(
        "SELECT s.visitor,s.started_at,s.last_at,s.pageviews,s.entry_page {} ORDER BY s.visitor",
        sessions.from
    ))?;
    let mut rows = statement.query(rusqlite::params_from_iter(sessions.values))?;
    while let Some(row) = rows.next()? {
        let visitor: [u8; 16] = row.get(0)?;
        let start: i64 = row.get(1)?;
        let last: i64 = row.get(2)?;
        let first = buckets.partition_point(|bucket| bucket.end <= start.max(window.start));
        if !event_counts {
            for index in first..buckets.len() {
                if buckets[index].start > last.min(window.end - 1) {
                    break;
                }
                buckets[index].visits += 1;
                if last_visitor[index] != Some(visitor) {
                    last_visitor[index] = Some(visitor);
                    buckets[index].visitors += 1;
                }
            }
        }
        if let Some(index) = containing(&buckets, &window, last) {
            let completed = &mut buckets[index].completed;
            let pageviews: i64 = row.get(3)?;
            let entry = row.get_ref(4)?.as_str()?;
            let entrance = query
                .has_filter("page")
                .then(|| i64::from(query.matches_filter("page", entry)));
            completed.visits += 1;
            completed.pageviews += pageviews;
            completed.duration += last - start;
            completed.bounces += i64::from(pageviews == 1 && entrance != Some(0));
            if let Some(entrance) = entrance {
                *completed.entrances.get_or_insert(0) += entrance;
            }
        }
    }
    drop(rows);
    drop(statement);

    if query.has_filter("page") {
        let mut statement=c.prepare(&format!("SELECT e.session_id,e.path,e.received_at,g.time_ms,g.scroll_depth FROM (SELECT e.id,e.session_id,e.path,e.received_at {}) e JOIN page_engagement g ON g.event_id=e.id ORDER BY e.session_id,e.path,e.received_at",events.from))?;
        let mut rows = statement.query(rusqlite::params_from_iter(events.values))?;
        let mut measured: Option<(Option<i64>, usize, i64, i64, String)> = None;
        let finish =
            |buckets: &mut [Bucket],
             (_, index, time, depth, _): (Option<i64>, usize, i64, i64, String)| {
                let row = &mut buckets[index].completed;
                row.time_on_page_ms += time;
                row.time_on_page_visits += 1;
                row.total_scroll_depth += depth;
                row.scroll_depth_visits += 1;
            };
        while let Some(row) = rows.next()? {
            let Some(index) = containing(&buckets, &window, row.get(2)?) else {
                continue;
            };
            let session = row.get(0)?;
            let path = row.get_ref(1)?.as_str()?;
            let time = row.get(3)?;
            let depth = row.get(4)?;
            if let Some(current) = measured
                .as_mut()
                .filter(|current| current.0 == session && current.1 == index && current.4 == path)
            {
                current.2 += time;
                current.3 = current.3.max(depth);
            } else {
                if let Some(previous) = measured.take() {
                    finish(&mut buckets, previous);
                }
                measured = Some((session, index, time, depth, path.to_owned()));
            }
        }
        if let Some(last) = measured {
            finish(&mut buckets, last);
        }
    }
    Ok(buckets)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn day(zone: Tz, date: &str) -> Window {
        let date: chrono::NaiveDate = date.parse().unwrap();
        Window {
            zone,
            step: 3600,
            start: zone
                .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .unwrap()
                .timestamp(),
            end: zone
                .from_local_datetime(&date.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .unwrap()
                .timestamp(),
        }
    }
    #[test]
    fn hours_follow_civil_time_across_half_hour_offsets_and_dst() {
        for (zone, date, count) in [
            (chrono_tz::Asia::Kolkata, "2026-10-27", 24),
            (chrono_tz::America::New_York, "2026-03-08", 23),
            (chrono_tz::America::New_York, "2026-11-01", 25),
        ] {
            let window = day(zone, date);
            let buckets = boundaries(&window);
            assert_eq!(buckets.len(), count);
            assert_eq!(buckets[0].start, window.start);
            assert_eq!(buckets.last().unwrap().end, window.end);
            assert!(buckets.iter().all(|bucket| {
                chrono::DateTime::from_timestamp(bucket.start, 0)
                    .unwrap()
                    .with_timezone(&zone)
                    .format("%M:%S")
                    .to_string()
                    == "00:00"
            }));
        }
        let window = day(chrono_tz::Australia::Lord_Howe, "2026-10-04");
        let buckets = boundaries(&window);
        assert_eq!(buckets.len(), 24);
        assert_eq!(buckets[2].end - buckets[2].start, 1800);
        assert_eq!(
            chrono::DateTime::from_timestamp(buckets[2].start, 0)
                .unwrap()
                .with_timezone(&window.zone)
                .format("%H:%M")
                .to_string(),
            "02:30"
        );
        let fall = boundaries(&day(chrono_tz::Australia::Lord_Howe, "2026-04-05"));
        assert_eq!(fall[1].end - fall[1].start, 5400);
    }
}
