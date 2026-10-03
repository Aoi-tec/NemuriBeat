mod mode;
pub use mode::MeasurementMode;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measurement {
    pub timestamp: i64,
    pub sensor_id: String,
    /// Upstream measurement clock; timestamp remains the local receive clock.
    #[serde(default)]
    pub source_timestamp: Option<i64>,
    pub heart_rate: u16,
    pub rr_ms: Vec<f64>,
    pub contact: Option<bool>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Features {
    pub timestamp: i64,
    pub hr_mean: Option<f64>,
    pub hr_min: Option<u16>,
    pub hr_max: Option<u16>,
    pub rmssd: Option<f64>,
    pub sdnn: Option<f64>,
    pub pnn50: Option<f64>,
    pub hr_slope: Option<f64>,
    pub baseline_delta: Option<f64>,
    pub quality: f64,
    pub valid_rr: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SleepEstimate {
    pub probability: f64,
    pub tracking: bool,
    pub sleep_like: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PhysioSnapshot {
    #[serde(default)]
    pub sleep_model: ModelStatus,
    #[serde(default)]
    pub hmd: HmdStatus,
    #[serde(default)]
    pub mode: MeasurementMode,
    #[serde(default)]
    pub rr_available: bool,
    #[serde(default)]
    pub source_timestamp: Option<i64>,
    pub sequence_id: u64,
    pub timestamp: i64,
    pub heart_rate: Option<u16>,
    pub heart_rate_average: Option<f64>,
    pub last_rr_ms: Option<f64>,
    pub rmssd_fast: Option<f64>,
    pub signal_quality: f64,
    pub sensor_connected: bool,
    pub sensor_id: Option<String>,
    pub features: Features,
    pub sleep: SleepEstimate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureMinute {
    pub timestamp: i64,
    pub features: Features,
    pub sleep_probability: Option<f64>,
    pub session_id: Option<String>,
    pub sample_count: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Baseline {
    pub resting_hr: Option<f64>,
    pub rmssd: Option<f64>,
    pub sample_minutes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SleepSession {
    #[serde(default)]
    pub sleep_day: Option<String>,
    pub session_id: String,
    pub started_at: i64,
    pub estimated_sleep_at: Option<i64>,
    pub final_wake_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub estimated_sleep_minutes: f64,
    pub avg_hr: Option<f64>,
    pub min_hr: Option<u16>,
    pub median_rmssd: Option<f64>,
    pub awakening_count: u32,
    pub baseline_delta: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Event {
    SensorConnected(String),
    SensorDisconnected,
    HeartBeat { timestamp: i64, rr_ms: f64 },
    SleepSessionStarted(String),
    SleepSessionEnded(String),
    SleepProbabilityChanged(SleepEstimate),
    BaselineUpdated(Baseline),
    StorageError(String),
    VrchatConnected,
    VrchatDisconnected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub sleep_python: String,
    pub sleep_hmd_enabled: bool,
    pub sleep_day_start_hour: u32,
    pub oyasumi_enabled: bool,
    pub oyasumi_allow_hr_only: bool,
    pub oyasumi_query_port: u16,
    pub osc_enabled: bool,
    pub oscquery_enabled: bool,
    pub osc_host: String,
    pub osc_port: u16,
    pub heartrate_output: bool,
    pub sleep_output: bool,
    pub sleep_enabled: bool,
    pub baseline_days: u32,
    pub api_enabled: bool,
    pub overlay_enabled: bool,
    pub close_to_tray: bool,
    pub hyperate_id: String,
    pub api_port: u16,
    pub sleep_confirm_secs: u32,
    pub pseudo_sleep_hr: u16,
    pub auto_sleep_recording: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            sleep_python: std::env::var("NEMURIBEAT_PYTHON")
                .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.into()),
            sleep_hmd_enabled: true,
            sleep_day_start_hour: 12,
            oyasumi_enabled: false,
            oyasumi_allow_hr_only: false,
            oyasumi_query_port: 0,
            osc_enabled: false,
            oscquery_enabled: true,
            osc_host: "127.0.0.1".into(),
            osc_port: 9000,
            heartrate_output: true,
            sleep_output: true,
            sleep_enabled: true,
            baseline_days: 14,
            api_enabled: false,
            overlay_enabled: true,
            close_to_tray: true,
            hyperate_id: String::new(),
            api_port: 8791,
            sleep_confirm_secs: 300,
            pseudo_sleep_hr: 60,
            auto_sleep_recording: true,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.sleep_python.trim().is_empty() || self.sleep_python.contains(['\n', '\r', '\0']) {
            return Err("睡眠モデル用Pythonの実行ファイルを指定してください".into());
        }
        if self.sleep_day_start_hour > 23 {
            return Err("睡眠日の区切りは0〜23時で指定してください".into());
        }
        if !(60..=1800).contains(&self.sleep_confirm_secs) {
            return Err("睡眠確定時間は60〜1800秒で指定してください".into());
        }
        if !(35..=100).contains(&self.pseudo_sleep_hr) {
            return Err("擬似睡眠の基準心拍は35〜100 bpmで指定してください".into());
        }
        if self.osc_host.parse::<std::net::IpAddr>().is_err() {
            return Err("OSC host must be an IP address".into());
        }
        if self.osc_port == 0 || self.api_port == 0 {
            return Err("Ports must be nonzero".into());
        }
        if ![14, 30].contains(&self.baseline_days) {
            return Err("Baseline period must be 14 or 30 days".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelStatus {
    pub model: String,
    pub ready: bool,
    pub probability: Option<f64>,
    pub evaluated_at: Option<i64>,
    pub received_at: Option<i64>,
    pub message: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HmdStatus {
    pub timestamp: i64,
    pub available: bool,
    pub worn: Option<bool>,
    pub acceleration_mean: Option<f64>,
    pub angular_speed_mean: Option<f64>,
    pub still: bool,
    pub rejected: bool,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DailySleep {
    pub sleep_day: String,
    pub summary: SleepSession,
    pub segments: Vec<SleepSession>,
}
