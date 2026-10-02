use shared_types::*;
fn mean(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        None
    } else {
        Some(v.iter().sum::<f64>() / v.len() as f64)
    }
}
pub fn calculate(samples: &[Measurement], now: i64, baseline: &Baseline) -> Features {
    let mut f = Features {
        timestamp: now,
        ..Default::default()
    };
    let hr: Vec<f64> = samples
        .iter()
        .filter(|m| m.contact != Some(false) && (25..=240).contains(&m.heart_rate))
        .map(|m| m.heart_rate as f64)
        .collect();
    f.hr_mean = mean(&hr);
    f.hr_min = hr.iter().copied().reduce(f64::min).map(|x| x as u16);
    f.hr_max = hr.iter().copied().reduce(f64::max).map(|x| x as u16);
    let mut good = Vec::new();
    let mut diffs = Vec::new();
    let mut previous: Option<f64> = None;
    let mut previous_time = None;
    let mut total = 0;
    for m in samples {
        if previous_time.is_some_and(|t| m.timestamp - t > 5000) {
            previous = None;
        }
        previous_time = Some(m.timestamp);
        for &rr in &m.rr_ms {
            total += 1;
            let valid = m.contact != Some(false)
                && rr.is_finite()
                && (300.0..=2000.0).contains(&rr)
                && previous.is_none_or(|p| (rr - p).abs() / p <= 0.25);
            if valid {
                if let Some(p) = previous {
                    diffs.push(rr - p);
                }
                good.push(rr);
                previous = Some(rr);
            } else {
                previous = None;
            }
        }
    }
    f.valid_rr = good.len();
    let fresh = samples
        .last()
        .is_some_and(|m| now - m.timestamp <= 5000 && m.contact != Some(false))
        && samples
            .iter()
            .rev()
            .find(|m| !m.rr_ms.is_empty())
            .is_some_and(|m| now - m.timestamp <= 5000);
    f.quality = if total > 0 && fresh {
        good.len() as f64 / total as f64
    } else {
        0.0
    };
    if !diffs.is_empty() {
        f.rmssd = Some((diffs.iter().map(|d| d * d).sum::<f64>() / diffs.len() as f64).sqrt());
        f.pnn50 = Some(
            diffs.iter().filter(|d| d.abs() > 50.0).count() as f64 / diffs.len() as f64 * 100.0,
        );
    }
    if good.len() > 1 {
        let avg = mean(&good).unwrap();
        f.sdnn = Some(
            (good.iter().map(|x| (x - avg).powi(2)).sum::<f64>() / (good.len() - 1) as f64).sqrt(),
        );
    }
    if let (Some(h), Some(b)) = (f.hr_mean, baseline.resting_hr) {
        f.baseline_delta = Some(h - b);
    }
    let points: Vec<(f64, f64)> = samples
        .iter()
        .filter(|m| m.contact != Some(false) && (25..=240).contains(&m.heart_rate))
        .map(|m| ((m.timestamp - now) as f64 / 60000.0, m.heart_rate as f64))
        .collect();
    if points.len() > 1 {
        let n = points.len() as f64;
        let x = points.iter().map(|p| p.0).sum::<f64>() / n;
        let y = points.iter().map(|p| p.1).sum::<f64>() / n;
        let denom = points.iter().map(|p| (p.0 - x).powi(2)).sum::<f64>();
        if denom > 0.0 {
            f.hr_slope = Some(points.iter().map(|p| (p.0 - x) * (p.1 - y)).sum::<f64>() / denom);
        }
    }
    f
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_hrv_and_artifact_gap() {
        let m = Measurement {
            timestamp: 1000,
            sensor_id: "x".into(),
            source_timestamp: None,
            heart_rate: 60,
            rr_ms: vec![1000.0, 1100.0, 1000.0],
            contact: Some(true),
        };
        let f = calculate(std::slice::from_ref(&m), 1000, &Baseline::default());
        assert_eq!(f.rmssd, Some(100.0));
        assert_eq!(f.pnn50, Some(100.0));
        assert!((f.sdnn.unwrap() - 57.735).abs() < 0.001);
        let f = calculate(
            &[Measurement {
                rr_ms: vec![1000.0, 10.0, 1100.0],
                ..m
            }],
            1000,
            &Baseline::default(),
        );
        assert!(f.rmssd.is_none());
        assert_eq!(f.valid_rr, 2);
    }
    #[test]
    fn stale_has_zero_quality() {
        let m = Measurement {
            timestamp: 0,
            sensor_id: "x".into(),
            source_timestamp: None,
            heart_rate: 60,
            rr_ms: vec![1000.0; 40],
            contact: None,
        };
        assert_eq!(calculate(&[m], 6000, &Baseline::default()).quality, 0.0);
    }
}

#[cfg(test)]
mod rr_freshness_tests {
    use super::*;
    #[test]
    fn hr_only_notifications_cannot_keep_rr_tracking_alive() {
        let first = Measurement {
            timestamp: 0,
            sensor_id: "x".into(),
            source_timestamp: None,
            heart_rate: 60,
            rr_ms: vec![1000.0; 40],
            contact: Some(true),
        };
        let last = Measurement {
            timestamp: 6000,
            rr_ms: vec![],
            ..first.clone()
        };
        assert_eq!(
            calculate(&[first, last], 6000, &Baseline::default()).quality,
            0.0
        );
    }
}
