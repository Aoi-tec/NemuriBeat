use chrono::{Local, TimeZone, Timelike};
use shared_types::*;
use std::collections::BTreeMap;

/// Noon-to-noon by default, so midnight and reconnects do not split one night's display.
pub fn sleep_day(timestamp: i64, start_hour: u32) -> String {
    let Some(local) = Local.timestamp_millis_opt(timestamp).single() else {
        return "unknown".into();
    };
    let date = local.date_naive();
    let date = if local.hour() < start_hour {
        date.pred_opt().unwrap_or(date)
    } else {
        date
    };
    date.to_string()
}

pub fn group_sleep_days(sessions: &[SleepSession], start_hour: u32) -> Vec<DailySleep> {
    let mut groups: BTreeMap<String, Vec<SleepSession>> = BTreeMap::new();
    for session in sessions {
        let date = session.sleep_day.clone().unwrap_or_else(|| {
            sleep_day(
                session.estimated_sleep_at.unwrap_or(session.started_at),
                start_hour,
            )
        });
        groups.entry(date).or_default().push(session.clone());
    }
    groups
        .into_iter()
        .rev()
        .map(|(date, mut segments)| {
            segments.sort_by_key(|s| s.started_at);
            let mut summary = segments[0].clone();
            summary.session_id = format!("day:{date}");
            summary.sleep_day = Some(date.clone());
            summary.estimated_sleep_minutes = segments
                .iter()
                .map(|s| s.estimated_sleep_minutes.max(0.0))
                .sum();
            summary.estimated_sleep_at = segments.iter().filter_map(|s| s.estimated_sleep_at).min();
            summary.ended_at = if segments.iter().all(|s| s.ended_at.is_some()) {
                segments.iter().filter_map(|s| s.ended_at).max()
            } else {
                None
            };
            summary.final_wake_at = segments.last().and_then(|s| s.final_wake_at);
            summary.min_hr = segments.iter().filter_map(|s| s.min_hr).min();
            summary.awakening_count = segments.iter().map(|s| s.awakening_count).sum();
            let weighted: Vec<_> = segments
                .iter()
                .filter_map(|s| s.avg_hr.map(|hr| (hr, s.estimated_sleep_minutes.max(0.0))))
                .collect();
            let weight: f64 = weighted.iter().map(|(_, w)| w).sum();
            summary.avg_hr =
                (weight > 0.0).then(|| weighted.iter().map(|(v, w)| v * w).sum::<f64>() / weight);
            // A median of per-segment medians is not the median of a day's measurements.
            if segments.len() > 1 {
                summary.median_rmssd = None;
                summary.baseline_delta = None;
            }
            DailySleep {
                sleep_day: date,
                summary,
                segments,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(id: &str, date: &str, start: i64, minutes: f64) -> SleepSession {
        SleepSession {
            sleep_day: Some(date.into()),
            session_id: id.into(),
            started_at: start,
            estimated_sleep_at: Some(start),
            final_wake_at: Some(start + 60000),
            ended_at: Some(start + 60000),
            estimated_sleep_minutes: minutes,
            avg_hr: Some(60.0),
            min_hr: Some(50),
            median_rmssd: Some(30.0),
            awakening_count: 0,
            baseline_delta: None,
        }
    }
    #[test]
    fn fragmented_night_is_one_day_without_filling_gaps() {
        let groups = group_sleep_days(
            &[
                segment("a", "2026-10-02", 1000, 60.0),
                segment("b", "2026-10-02", 36000000, 90.0),
                segment("c", "2026-10-03", 86400000, 30.0),
            ],
            12,
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[1].summary.estimated_sleep_minutes, 150.0);
        assert_eq!(groups[1].segments.len(), 2);
        assert_eq!(groups[1].summary.session_id, "day:2026-10-02");
        assert!(groups[1].summary.median_rmssd.is_none());
    }
    #[test]
    fn local_midnight_does_not_split_a_night() {
        let before = Local
            .with_ymd_and_hms(2026, 10, 2, 23, 0, 0)
            .unwrap()
            .timestamp_millis();
        let after = Local
            .with_ymd_and_hms(2026, 10, 3, 7, 0, 0)
            .unwrap()
            .timestamp_millis();
        assert_eq!(sleep_day(before, 12), sleep_day(after, 12));
        assert_ne!(sleep_day(before, 0), sleep_day(after, 0));
    }
}
