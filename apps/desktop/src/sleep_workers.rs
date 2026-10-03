use anyhow::{anyhow, Result};
use serde::Deserialize;
use shared_types::*;
use std::{collections::VecDeque, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::watch,
    task::JoinHandle,
};

const GRU: &str = include_str!("../python/sleep_gru.py");
const OPENVR: &str = include_str!("../python/hmd_openvr.py");
#[derive(Clone)]
pub struct ModelRequest {
    pub generation: u64,
    pub measurements: Vec<Measurement>,
}
pub struct Workers {
    input: watch::Sender<Option<ModelRequest>>,
    model: watch::Receiver<(u64, ModelStatus)>,
    hmd: watch::Receiver<HmdStatus>,
    tasks: Vec<JoinHandle<()>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pose(t: i64, v: f64, worn: Option<bool>) -> Pose {
        Pose {
            timestamp: t,
            available: true,
            velocity: [v, 0.0, 0.0],
            angular_velocity: [0.0; 3],
            worn,
            ..Default::default()
        }
    }
    #[test]
    fn constant_zero_is_rejected_even_when_proximity_is_true() {
        let mut window = MotionWindow::default();
        let mut s = HmdStatus::default();
        for i in 0..310 {
            s = window.update(pose(i * 100, 0.0, Some(true)));
        }
        assert_eq!(s.acceleration_mean, Some(0.0));
        assert!(s.rejected);
        assert!(!s.still);
    }
    #[test]
    fn alternating_acceleration_does_not_cancel_and_removed_hmd_is_rejected() {
        let mut window = MotionWindow::default();
        let mut s = HmdStatus::default();
        for i in 0..310 {
            s = window.update(pose(i * 100, if i % 2 == 0 { 0.001 } else { 0.0 }, None));
        }
        assert!((s.acceleration_mean.unwrap() - 0.01).abs() < 1e-6);
        assert!(s.still);
        assert!(!s.rejected);
        s = window.update(pose(31000, 0.0, Some(false)));
        assert!(s.rejected);
        s = window.update(Pose {
            timestamp: 31100,
            ..Default::default()
        });
        assert!(!s.available);
        assert!(s.acceleration_mean.is_none());
        assert!(window.values.is_empty());
    }
    #[test]
    fn window_is_bounded_and_sampling_gap_needs_new_warmup() {
        let mut window = MotionWindow::default();
        for i in 0..2000 {
            window.update(pose(i * 100, 0.0, None));
        }
        assert!(window.values.len() <= 300);
        let s = window.update(pose(250000, 0.0, None));
        assert!(s.acceleration_mean.is_none());
    }
    #[tokio::test]
    #[ignore = "requires dedicated Python 3.12 SleepECG/TensorFlow environment"]
    async fn official_gru_worker_protocol_and_generation() {
        let python = std::env::var("NEMURIBEAT_TEST_PYTHON").expect("Set NEMURIBEAT_TEST_PYTHON");
        let mut workers = Workers::start(python, false);
        let mut timestamp = now_ms() - 1200000;
        let mut measurements = Vec::new();
        for i in 0..1200 {
            let rr = 1000.0 + 35.0 * (i as f64 * 0.3).sin() + 20.0 * (i as f64 * 0.11).sin();
            timestamp += rr as i64;
            measurements.push(Measurement {
                timestamp,
                sensor_id: "TEST".into(),
                source_timestamp: None,
                heart_rate: 60,
                rr_ms: vec![rr],
                contact: Some(true),
            });
        }
        workers.request(ModelRequest {
            generation: 7,
            measurements,
        });
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                workers.model.changed().await.unwrap();
                let (g, s) = workers.model.borrow().clone();
                if g == 7 && s.ready {
                    break;
                }
            }
        })
        .await
        .expect("Official model did not respond");
        let (status, _) = workers.latest(7);
        assert!(status.ready);
        assert!(status.probability.unwrap().is_finite());
        assert!(
            !workers.latest(8).0.ready,
            "Old sensor generation must be discarded"
        );
        workers.stop().await;
    }
    #[tokio::test]
    async fn missing_python_reports_unknown_without_stalling() {
        let mut workers = Workers::start("/nonexistent/nemuribeat-python".into(), false);
        workers.request(ModelRequest {
            generation: 1,
            measurements: vec![],
        });
        tokio::time::timeout(Duration::from_secs(2), workers.model.changed())
            .await
            .unwrap()
            .unwrap();
        let status = workers.latest(1).0;
        assert!(!status.ready);
        assert!(status.probability.is_none());
        assert!(status.message.contains("確認"));
        workers.stop().await;
    }
}
fn process(python: &str, script: &str) -> Result<Child> {
    let mut command = Command::new(python);
    command
        .arg("-u")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    Ok(command.spawn()?)
}
async fn status_line(reader: &mut BufReader<tokio::process::ChildStdout>) -> Result<String> {
    // Bound a worker reply even if the configured Python executable is broken/noisy.
    let mut bytes = Vec::new();
    (&mut *reader)
        .take(8193)
        .read_until(b'\n', &mut bytes)
        .await?;
    if bytes.is_empty() || bytes.len() > 8192 || bytes.last() != Some(&b'\n') {
        return Err(anyhow!("Invalid worker reply"));
    }
    Ok(String::from_utf8(bytes)?)
}
impl Workers {
    pub fn start(python: String, use_hmd: bool) -> Self {
        let (input, mut requests) = watch::channel::<Option<ModelRequest>>(None);
        let (model_tx, model) = watch::channel((
            0,
            ModelStatus {
                model: "ws-gru-mesa".into(),
                message: "RR蓄積待ち（GRUはRR取得時にロード）".into(),
                ..Default::default()
            },
        ));
        let (hmd_tx, hmd) = watch::channel(HmdStatus::default());
        let model_python = python.clone();
        let model_task = tokio::spawn(async move {
            loop {
                let result = async {
                    let mut pending = requests.borrow_and_update().clone();
                    if pending.is_none() {
                        if requests.changed().await.is_err() {
                            return Ok(());
                        }
                        pending = requests.borrow_and_update().clone();
                    }
                    let mut child = process(&model_python, GRU)?;
                    let mut stdin = child.stdin.take().unwrap();
                    let mut stdout = BufReader::new(child.stdout.take().unwrap());
                    let line =
                        tokio::time::timeout(Duration::from_secs(90), status_line(&mut stdout))
                            .await??;
                    let initial: ModelStatus = serde_json::from_str(&line)?;
                    model_tx.send_replace((0, initial));
                    while let Some(request) = pending.take() {
                        let bytes = serde_json::to_vec(
                            &serde_json::json!({"measurements":request.measurements}),
                        )?;
                        let inference = async {
                            stdin.write_all(&bytes).await?;
                            stdin.write_all(b"\n").await?;
                            stdin.flush().await?;
                            let line = status_line(&mut stdout).await?;
                            let mut status: ModelStatus = serde_json::from_str(&line)?;
                            status.received_at = Some(now_ms());
                            if status
                                .probability
                                .is_some_and(|p| !p.is_finite() || !(0.0..=1.0).contains(&p))
                            {
                                status = ModelStatus::default();
                            }
                            model_tx.send_replace((request.generation, status));
                            Ok::<_, anyhow::Error>(())
                        };
                        tokio::time::timeout(Duration::from_secs(30), inference).await??;
                        if requests.changed().await.is_err() {
                            break;
                        }
                        pending = requests.borrow_and_update().clone();
                    }
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                if result.is_ok() {
                    break;
                }
                model_tx.send_replace((
                    0,
                    ModelStatus {
                        model: "ws-gru-mesa".into(),
                        message: "GRUワーカー停止。Python環境を確認してください（30秒後に再試行）"
                            .into(),
                        ..Default::default()
                    },
                ));
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
        let mut tasks = vec![model_task];
        if use_hmd {
            tasks.push(tokio::spawn(async move {
                loop {
                    let result = async {
                        let mut child = process(&python, OPENVR)?;
                        let mut stdout = BufReader::new(child.stdout.take().unwrap());
                        let mut motion = MotionWindow::default();
                        loop {
                            let line = tokio::time::timeout(
                                Duration::from_secs(10),
                                status_line(&mut stdout),
                            )
                            .await??;
                            let pose: Pose = serde_json::from_str(&line)?;
                            hmd_tx.send_replace(motion.update(pose));
                        }
                        #[allow(unreachable_code)]
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    let _ = result;
                    hmd_tx.send_replace(HmdStatus {
                        message: "OpenVRワーカー停止。30秒後に再試行します".into(),
                        ..Default::default()
                    });
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }));
        }
        Self {
            input,
            model,
            hmd,
            tasks,
        }
    }
    pub fn request(&self, request: ModelRequest) {
        self.input.send_replace(Some(request));
    }
    pub fn latest(&self, generation: u64) -> (ModelStatus, HmdStatus) {
        let (g, status) = self.model.borrow().clone();
        let status = if g == generation || !status.ready {
            status
        } else {
            ModelStatus {
                model: "ws-gru-mesa".into(),
                message: "入力変更後のGRU推論待ち".into(),
                ..Default::default()
            }
        };
        (status, self.hmd.borrow().clone())
    }
    pub async fn stop(self) {
        for task in self.tasks {
            task.abort();
            let _ = task.await;
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Pose {
    timestamp: i64,
    available: bool,
    velocity: [f64; 3],
    angular_velocity: [f64; 3],
    worn: Option<bool>,
    message: String,
}
#[derive(Default)]
struct MotionWindow {
    previous: Option<Pose>,
    values: VecDeque<(i64, f64, f64, f64, f64)>,
}
fn norm(v: [f64; 3]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}
impl MotionWindow {
    fn update(&mut self, pose: Pose) -> HmdStatus {
        let mut status = HmdStatus {
            timestamp: pose.timestamp,
            available: pose.available,
            worn: pose.worn,
            message: pose.message.clone(),
            ..Default::default()
        };
        if !pose.available
            || pose
                .velocity
                .iter()
                .chain(pose.angular_velocity.iter())
                .any(|v| !v.is_finite())
        {
            self.previous = None;
            self.values.clear();
            status.available = false;
            return status;
        }
        if let Some(previous) = &self.previous {
            let dt = (pose.timestamp - previous.timestamp) as f64 / 1000.0;
            if (0.02..=0.5).contains(&dt) {
                let acceleration = norm(std::array::from_fn(|i| {
                    (pose.velocity[i] - previous.velocity[i]) / dt
                }));
                self.values.push_back((
                    pose.timestamp,
                    dt,
                    acceleration,
                    norm(pose.angular_velocity),
                    norm(pose.velocity),
                ));
            } else {
                self.values.clear();
            }
        }
        self.previous = Some(pose);
        while self
            .values
            .front()
            .is_some_and(|v| status.timestamp - v.0 >= 30000)
            || self.values.len() > 600
        {
            self.values.pop_front();
        }
        let duration = self.values.iter().map(|v| v.1).sum::<f64>();
        if duration < 29.5 {
            status.message = "HMD静止判定を蓄積中（30秒）".into();
            return status;
        }
        let acceleration = self.values.iter().map(|v| v.1 * v.2).sum::<f64>() / duration;
        let angular = self.values.iter().map(|v| v.1 * v.3).sum::<f64>() / duration;
        let speed = self.values.iter().map(|v| v.1 * v.4).sum::<f64>() / duration;
        status.acceleration_mean = Some(acceleration);
        status.angular_speed_mean = Some(angular);
        status.rejected = status.worn == Some(false) || acceleration == 0.0;
        status.still = !status.rejected && acceleration <= 0.03 && angular <= 0.04 && speed <= 0.02;
        status.message = if status.worn == Some(false) {
            "HMD未装着を検出。睡眠判定を棄却"
        } else if acceleration == 0.0 {
            "加速度の大きさの移動平均が0。睡眠判定を棄却"
        } else if status.still {
            "HMDは静止範囲内"
        } else {
            "HMDの動きを検出"
        }
        .into();
        status
    }
}
