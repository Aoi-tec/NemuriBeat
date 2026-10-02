use shared_types::*;
#[derive(Default)]
pub struct SleepEngine {
    ema: Option<f64>,
    last: Option<i64>,
    candidate: Option<i64>,
    asleep: bool,
    last_mode: MeasurementMode,
    pub confirmed_since: Option<i64>,
}
impl SleepEngine {
    /// Experimental heuristic, not a trained or calibrated probability model.
    pub fn update(
        &mut self,
        f: &Features,
        b: &Baseline,
        connected: bool,
        enabled: bool,
        now: i64,
    ) -> SleepEstimate {
        self.update_configured(f, b, connected, enabled, now, 120)
    }
    pub fn update_configured(
        &mut self,
        f: &Features,
        b: &Baseline,
        connected: bool,
        enabled: bool,
        now: i64,
        confirm_secs: u32,
    ) -> SleepEstimate {
        if self.last_mode != MeasurementMode::HrRr {
            *self = Self::default();
            self.last_mode = MeasurementMode::HrRr;
        }
        if !enabled
            || !connected
            || f.quality < 0.8
            || f.valid_rr < 30
            || b.sample_minutes < 30
            || b.resting_hr.is_none()
            || f.hr_mean.is_none()
            || f.rmssd.is_none()
        {
            *self = Self::default();
            return SleepEstimate::default();
        }
        let delta = b.resting_hr.unwrap() - f.hr_mean.unwrap_or(80.0);
        let hrv = match (f.rmssd, b.rmssd) {
            (Some(r), Some(base)) if base > 0.0 => (r / base - 1.0).clamp(-1.0, 1.0),
            _ => 0.0,
        };
        let score = delta / 5.0 + hrv * 0.5 - f.hr_slope.unwrap_or(0.0).max(0.0) * 0.15;
        let raw = 1.0 / (1.0 + (-score).exp());
        self.smooth(raw, now, confirm_secs)
    }
    /// HR-only estimate for avatar/external use. Never qualifies for a recorded sleep session.
    pub fn update_hr_only(
        &mut self,
        f: &Features,
        connected: bool,
        enabled: bool,
        now: i64,
        threshold: u16,
        confirm_secs: u32,
    ) -> SleepEstimate {
        if self.last_mode != MeasurementMode::HrOnly {
            *self = Self::default();
            self.last_mode = MeasurementMode::HrOnly;
        }
        if !connected || !enabled || f.hr_mean.is_none() {
            *self = Self::default();
            return SleepEstimate::default();
        }
        let hr = f.hr_mean.unwrap();
        let score =
            (f64::from(threshold) + 4.0 - hr) / 3.0 - f.hr_slope.unwrap_or(0.0).max(0.0) * 0.2;
        self.smooth(1.0 / (1.0 + (-score).exp()), now, confirm_secs)
    }
    fn smooth(&mut self, raw: f64, now: i64, confirm_secs: u32) -> SleepEstimate {
        if self
            .last
            .is_some_and(|last| now - last > 5000 || now < last)
        {
            self.ema = None;
            self.candidate = None;
            self.asleep = false;
            self.confirmed_since = None;
        }
        let dt = self
            .last
            .map(|t| (now - t).clamp(0, 5000) as f64 / 1000.0)
            .unwrap_or(1.0);
        let p = self
            .ema
            .map(|p| p + (1.0 - (-dt / 30.0).exp()) * (raw - p))
            .unwrap_or(raw);
        self.ema = Some(p);
        self.last = Some(now);
        let transition = if self.asleep { p <= 0.40 } else { p >= 0.75 };
        if transition {
            let since = *self.candidate.get_or_insert(now);
            let required = if self.asleep {
                120_000
            } else {
                i64::from(confirm_secs) * 1000
            };
            if now - since >= required {
                self.asleep = !self.asleep;
                self.confirmed_since = if self.asleep { Some(since) } else { None };
                self.candidate = None;
            }
        } else {
            self.candidate = None;
        }
        SleepEstimate {
            probability: p,
            tracking: true,
            sleep_like: self.asleep,
        }
    }
}
pub struct SessionTracker {
    pub session: SleepSession,
    last: i64,
    previous: SleepEstimate,
    hr_sum: f64,
    hr_count: u64,
    rmssd_histogram: Vec<u64>,
    rmssd_count: u64,
}
impl SessionTracker {
    pub fn new(now: i64) -> Self {
        Self {
            session: SleepSession {
                session_id: uuid::Uuid::new_v4().to_string(),
                started_at: now,
                estimated_sleep_at: None,
                final_wake_at: None,
                ended_at: None,
                estimated_sleep_minutes: 0.0,
                avg_hr: None,
                min_hr: None,
                median_rmssd: None,
                awakening_count: 0,
                baseline_delta: None,
            },
            last: now,
            previous: SleepEstimate::default(),
            hr_sum: 0.0,
            hr_count: 0,
            rmssd_histogram: vec![0; 2001],
            rmssd_count: 0,
        }
    }
    pub fn update(&mut self, s: &PhysioSnapshot, now: i64) {
        let dt = (now - self.last).clamp(0, 5000) as f64 / 60000.0;
        if self.previous.tracking && self.previous.sleep_like && s.sleep.tracking {
            self.session.estimated_sleep_minutes += dt;
        }
        if s.sleep.tracking && s.sleep.sleep_like && self.session.estimated_sleep_at.is_none() {
            self.session.estimated_sleep_at = Some(now);
        }
        if self.previous.tracking
            && self.previous.sleep_like
            && s.sleep.tracking
            && !s.sleep.sleep_like
        {
            self.session.awakening_count += 1;
            self.session.final_wake_at = Some(now);
        }
        if s.sleep.tracking && s.sleep.sleep_like {
            self.session.final_wake_at = None;
        }
        self.last = now;
        self.previous = s.sleep.clone();
    }
    pub fn add_minute(&mut self, m: &FeatureMinute) {
        if let Some(hr) = m.features.hr_mean {
            self.hr_sum += hr;
            self.hr_count += 1;
            self.session.avg_hr = Some(self.hr_sum / self.hr_count as f64);
        }
        if let Some(hr) = m.features.hr_min {
            self.session.min_hr = Some(self.session.min_hr.map(|old| old.min(hr)).unwrap_or(hr));
        }
        if let Some(r) = m.features.rmssd {
            let bin = r.round().clamp(0.0, 2000.0) as usize;
            self.rmssd_histogram[bin] += 1;
            self.rmssd_count += 1;
            let low = (self.rmssd_count - 1) / 2;
            let high = self.rmssd_count / 2;
            let mut seen = 0;
            let mut a = None;
            let mut b = 0;
            for (i, count) in self.rmssd_histogram.iter().enumerate() {
                seen += count;
                if a.is_none() && seen > low {
                    a = Some(i);
                }
                if seen > high {
                    b = i;
                    break;
                }
            }
            self.session.median_rmssd = Some((a.unwrap_or(b) + b) as f64 / 2.0);
        }
        self.session.baseline_delta = m.features.baseline_delta;
    }
    pub fn end(&mut self, now: i64) {
        self.session.ended_at = Some(now);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cannot_track_without_baseline_or_connection() {
        let mut e = SleepEngine::default();
        let f = Features {
            quality: 1.0,
            valid_rr: 40,
            hr_mean: Some(55.0),
            rmssd: Some(50.0),
            ..Default::default()
        };
        assert!(!e.update(&f, &Baseline::default(), true, true, 0).tracking);
        let b = Baseline {
            resting_hr: Some(70.0),
            rmssd: Some(40.0),
            sample_minutes: 30,
        };
        assert!(e.update(&f, &b, true, true, 1).tracking);
        assert!(!e.update(&f, &b, false, true, 2).tracking);
    }
    #[test]
    fn gaps_do_not_count_as_sleep() {
        let mut t = SessionTracker::new(0);
        let mut s = PhysioSnapshot {
            sleep: SleepEstimate {
                tracking: true,
                sleep_like: true,
                probability: 0.9,
            },
            ..Default::default()
        };
        t.update(&s, 1000);
        s.sleep.tracking = false;
        t.update(&s, 60000);
        assert_eq!(t.session.estimated_sleep_minutes, 0.0);
    }
}

#[cfg(test)]
mod temporal_tests {
    use super::*;
    #[test]
    fn entry_requires_two_minutes_and_disconnect_resets() {
        let mut engine = SleepEngine::default();
        let baseline = Baseline {
            resting_hr: Some(80.0),
            rmssd: Some(40.0),
            sample_minutes: 40,
        };
        let f = Features {
            hr_mean: Some(50.0),
            rmssd: Some(40.0),
            quality: 1.0,
            valid_rr: 60,
            ..Default::default()
        };
        for t in 0..120 {
            assert!(
                !engine
                    .update(&f, &baseline, true, true, t * 1000)
                    .sleep_like
            );
        }
        assert!(engine.update(&f, &baseline, true, true, 120000).sleep_like);
        assert!(!engine.update(&f, &baseline, false, true, 121000).tracking);
        assert!(!engine.update(&f, &baseline, true, true, 122000).sleep_like);
    }
}

#[cfg(test)]
mod mode_tests {
    use super::*;
    fn low_hr() -> Features {
        Features {
            hr_mean: Some(50.0),
            rmssd: Some(45.0),
            quality: 1.0,
            valid_rr: 60,
            ..Default::default()
        }
    }
    #[test]
    fn hr_only_needs_duration_and_restarts_on_mode_change() {
        let mut e = SleepEngine::default();
        let f = low_hr();
        for t in 0..60 {
            assert!(
                !e.update_hr_only(&f, true, true, t * 1000, 60, 60)
                    .sleep_like
            );
        }
        assert!(e.update_hr_only(&f, true, true, 60000, 60, 60).sleep_like);
        let b = Baseline {
            resting_hr: Some(80.0),
            rmssd: Some(40.0),
            sample_minutes: 30,
        };
        assert!(
            !e.update_configured(&f, &b, true, true, 61000, 60)
                .sleep_like
        );
        assert!(!e.update_hr_only(&f, false, true, 62000, 60, 60).tracking);
    }
    #[test]
    fn clock_gap_does_not_confirm_sleep() {
        let mut e = SleepEngine::default();
        let f = low_hr();
        e.update_hr_only(&f, true, true, 0, 60, 60);
        assert!(!e.update_hr_only(&f, true, true, 90000, 60, 60).sleep_like);
    }
    #[test]
    fn session_accumulator_stays_fixed_size() {
        let mut t = SessionTracker::new(0);
        let m = FeatureMinute {
            timestamp: 0,
            features: low_hr(),
            sleep_probability: Some(0.9),
            session_id: None,
            sample_count: 60,
        };
        for _ in 0..10000 {
            t.add_minute(&m);
        }
        assert_eq!(t.rmssd_histogram.len(), 2001);
        assert_eq!(t.session.avg_hr, Some(50.0));
        assert_eq!(t.session.median_rmssd, Some(45.0));
    }
}
