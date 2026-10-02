use shared_types::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, watch};
const WINDOW_MS: i64 = 20 * 60 * 1000;
#[derive(Clone)]
pub struct RealtimeCore {
    snapshot: watch::Sender<PhysioSnapshot>,
    pub events: broadcast::Sender<Event>,
    measurements: Arc<Mutex<VecDeque<Measurement>>>,
}
impl Default for RealtimeCore {
    fn default() -> Self {
        Self::new()
    }
}
impl RealtimeCore {
    pub fn new() -> Self {
        let (snapshot, _) = watch::channel(PhysioSnapshot::default());
        let (events, _) = broadcast::channel(1024);
        Self {
            snapshot,
            events,
            measurements: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
    pub fn latest(&self) -> PhysioSnapshot {
        self.snapshot.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<PhysioSnapshot> {
        self.snapshot.subscribe()
    }
    pub fn connected(&self, id: String) {
        self.measurements.lock().unwrap().clear();
        self.snapshot.send_modify(|s| {
            let seq = s.sequence_id;
            *s = PhysioSnapshot::default();
            s.sequence_id = seq;
            s.sensor_connected = true;
            s.sensor_id = Some(id.clone());
        });
        let _ = self.events.send(Event::SensorConnected(id));
    }
    pub fn disconnected(&self) {
        self.snapshot.send_modify(|s| {
            s.sensor_connected = false;
            s.mode = MeasurementMode::Unavailable;
            s.rr_available = false;
            s.source_timestamp = None;
            s.signal_quality = 0.0;
            s.sleep = SleepEstimate::default();
            s.heart_rate = None;
            s.heart_rate_average = None;
            s.last_rr_ms = None;
            s.rmssd_fast = None;
            s.features = Features::default();
        });
        self.measurements.lock().unwrap().clear();
        let _ = self.events.send(Event::SensorDisconnected);
    }
    pub fn ingest(&self, m: Measurement) -> u64 {
        if !self.latest().sensor_connected {
            self.connected(m.sensor_id.clone());
        }
        let mut q = self.measurements.lock().unwrap();
        q.push_back(m.clone());
        while q
            .front()
            .is_some_and(|x| x.timestamp < m.timestamp - WINDOW_MS)
            || q.len() > 6000
        {
            q.pop_front();
        }
        let recent: Vec<f64> = q
            .iter()
            .filter(|x| x.timestamp > m.timestamp - 10000)
            .map(|x| x.heart_rate as f64)
            .collect();
        let average = recent.iter().sum::<f64>() / recent.len() as f64;
        let rr_available = q
            .iter()
            .rev()
            .find(|x| !x.rr_ms.is_empty())
            .is_some_and(|x| m.timestamp - x.timestamp <= 5000);
        drop(q);
        self.snapshot.send_modify(|s| {
            s.sequence_id += 1;
            s.timestamp = m.timestamp;
            s.source_timestamp = m.source_timestamp;
            s.rr_available = rr_available;
            let next_mode = if rr_available {
                MeasurementMode::HrRr
            } else {
                MeasurementMode::HrOnly
            };
            if s.mode != next_mode {
                s.sleep = SleepEstimate::default();
            }
            s.mode = next_mode;
            if !rr_available {
                s.rmssd_fast = None;
                s.features.rmssd = None;
                s.features.sdnn = None;
                s.features.pnn50 = None;
            }
            s.heart_rate = Some(m.heart_rate);
            s.heart_rate_average = Some(average);
            s.last_rr_ms = if rr_available {
                m.rr_ms.last().copied().or(s.last_rr_ms)
            } else {
                None
            };
        });
        // These events represent observed RR intervals, never extrapolated beats.
        let mut beat_time = m.timestamp;
        for rr in m.rr_ms.iter().rev() {
            let _ = self.events.send(Event::HeartBeat {
                timestamp: beat_time,
                rr_ms: *rr,
            });
            beat_time -= *rr as i64;
        }
        self.latest().sequence_id
    }
    pub fn window(&self, now: i64, window_ms: i64) -> Vec<Measurement> {
        self.measurements
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.timestamp >= now - window_ms && m.timestamp <= now)
            .cloned()
            .collect()
    }
    pub fn set_analysis(&self, features: Features, sleep: SleepEstimate) {
        self.snapshot.send_modify(|s| {
            let mut features = features;
            if !s.rr_available {
                features.rmssd = None;
                features.sdnn = None;
                features.pnn50 = None;
            }
            s.rmssd_fast = features.rmssd;
            s.signal_quality = features.quality;
            s.features = features;
            s.sleep = sleep.clone();
        });
        let _ = self.events.send(Event::SleepProbabilityChanged(sleep));
    }
    pub fn expire(&self, now: i64) -> bool {
        let s = self.latest();
        if s.sensor_connected && s.timestamp > 0 && now - s.timestamp > 5000 {
            self.disconnected();
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnect_invalidates_tracking() {
        let c = RealtimeCore::new();
        c.connected("x".into());
        c.set_analysis(
            Features::default(),
            SleepEstimate {
                probability: 0.9,
                tracking: true,
                sleep_like: true,
            },
        );
        c.disconnected();
        assert!(!c.latest().sleep.tracking);
    }
    #[test]
    fn ring_is_bounded_and_stale_data_expires() {
        let c = RealtimeCore::new();
        c.connected("x".into());
        for i in 1..7000 {
            c.ingest(Measurement {
                timestamp: i,
                sensor_id: "x".into(),
                source_timestamp: None,
                heart_rate: 60,
                rr_ms: vec![1000.0],
                contact: None,
            });
        }
        assert_eq!(c.window(7000, 2000000).len(), 6000);
        assert!(c.expire(14000));
        assert!(c.window(14000, 2000000).is_empty());
    }
}

#[cfg(test)]
mod modes {
    use super::*;
    #[test]
    fn missing_rr_removes_hrv_and_previous_sleep() {
        let c = RealtimeCore::new();
        let m = Measurement {
            timestamp: 1000,
            source_timestamp: None,
            sensor_id: "s".into(),
            heart_rate: 60,
            rr_ms: vec![1000.0],
            contact: None,
        };
        c.ingest(m.clone());
        c.set_analysis(
            Features {
                rmssd: Some(40.0),
                ..Default::default()
            },
            SleepEstimate {
                tracking: true,
                sleep_like: true,
                probability: 0.9,
            },
        );
        c.ingest(Measurement {
            timestamp: 7000,
            rr_ms: vec![],
            ..m
        });
        let s = c.latest();
        assert_eq!(s.mode, MeasurementMode::HrOnly);
        assert!(!s.rr_available);
        assert!(s.rmssd_fast.is_none());
        assert!(s.last_rr_ms.is_none());
        assert!(!s.sleep.tracking);
    }
}
