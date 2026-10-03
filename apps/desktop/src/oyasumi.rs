//! One-way sleep control using OyasumiVR's public OSC method and OSCQuery readback.
use anyhow::{ensure, Result};
use serde_json::Value;
use shared_types::{MeasurementMode, PhysioSnapshot, Settings};
use std::{
    collections::BTreeSet,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{oneshot, watch};
const ADDRESS: &str = "/OyasumiVR/SleepMode";

pub struct Worker {
    config: watch::Sender<Settings>,
    status: watch::Receiver<String>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Worker {
    pub fn start(snapshots: watch::Receiver<PhysioSnapshot>, settings: Settings) -> Self {
        let (config, rx) = watch::channel(settings);
        let (status_tx, status) = watch::channel("無効".into());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(run(snapshots, rx, status_tx, stopped));
        Self {
            config,
            status,
            stop,
            task,
        }
    }
    pub fn configure(&self, settings: Settings) {
        self.config.send_replace(settings);
    }
    pub fn status(&self) -> String {
        self.status.borrow().clone()
    }
    pub async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }
}

fn desired(s: &PhysioSnapshot, cfg: &Settings, now: i64) -> Option<bool> {
    if !cfg.oyasumi_enabled
        || !cfg.sleep_enabled
        || !s.sensor_connected
        || !s.sleep.tracking
        || !(0..=5000).contains(&(now - s.timestamp))
        || !(0..=2500).contains(&(now - s.features.timestamp))
    {
        return None;
    }
    match s.mode {
        MeasurementMode::HrRr if s.rr_available => Some(s.sleep.sleep_like),
        MeasurementMode::HrOnly if cfg.oyasumi_allow_hr_only => Some(s.sleep.sleep_like),
        _ => None,
    }
}
fn is_oyasumi_service(name: &str) -> bool {
    name.strip_suffix("._oscjson._tcp.local.")
        .is_some_and(|n| n == "OyasumiVR" || n.starts_with("OyasumiVR-"))
}
struct Discovery {
    client: Arc<vrchat_osc::VRChatOSC>,
    ports: Arc<Mutex<BTreeSet<u16>>>,
}
impl Discovery {
    async fn new() -> Result<Self> {
        let client = match vrchat_osc::VRChatOSC::new(None).await {
            Ok(client) => client,
            Err(vrchat_osc::Error::NoValidInterface) => {
                vrchat_osc::VRChatOSC::new(Some("127.0.0.1".parse()?)).await?
            }
            Err(e) => return Err(e.into()),
        };
        let ports = Arc::new(Mutex::new(BTreeSet::new()));
        let found = ports.clone();
        client
            .on_connect(move |service| {
                if let vrchat_osc::ServiceType::OscQuery(name, addr) = service {
                    if is_oyasumi_service(&name) && addr.port() != 0 {
                        let mut ports = found.lock().unwrap();
                        if ports.len() >= 8 && !ports.contains(&addr.port()) {
                            ports.pop_first();
                        }
                        ports.insert(addr.port());
                    }
                }
            })
            .await;
        Ok(Self { client, ports })
    }
}
async fn json(http: &reqwest::Client, url: String) -> Result<Value> {
    let mut response = http.get(url).send().await?.error_for_status()?;
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            data.len() + chunk.len() <= 65536,
            "OSCQuery response too large"
        );
        data.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&data)?)
}
fn endpoint(host: &Value, node: &Value) -> Result<(SocketAddr, bool)> {
    ensure!(
        host["NAME"] == "OyasumiVR" && host["OSC_TRANSPORT"] == "UDP",
        "Not OyasumiVR UDP"
    );
    let port = host["OSC_PORT"]
        .as_u64()
        .filter(|p| (1..=65535).contains(p))
        .ok_or_else(|| anyhow::anyhow!("Invalid OSC port"))? as u16;
    ensure!(
        node["FULL_PATH"] == ADDRESS && node["ACCESS"].as_u64().is_some_and(|a| a & 3 == 3),
        "SleepMode must be readable and writable"
    );
    // The official server serializes VALUE_TYPE; also accept standard OSCQuery TYPE.
    let kind = node
        .get("TYPE")
        .or_else(|| node.get("VALUE_TYPE"))
        .and_then(Value::as_str);
    ensure!(matches!(kind, Some("T" | "F")), "SleepMode must be a Bool");
    let value = node["VALUE"][0]
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("Missing Bool readback"))?;
    // OyasumiVR runs on this computer. Never relay commands to an advertised remote IP.
    Ok((SocketAddr::from(([127, 0, 0, 1], port)), value))
}
async fn probe(http: &reqwest::Client, port: u16) -> Result<(SocketAddr, bool)> {
    let host = json(http, format!("http://127.0.0.1:{port}/?HOST_INFO")).await?;
    let node = json(http, format!("http://127.0.0.1:{port}{ADDRESS}")).await?;
    endpoint(&host, &node)
}
async fn run(
    snapshots: watch::Receiver<PhysioSnapshot>,
    mut config: watch::Receiver<Settings>,
    status: watch::Sender<String>,
    mut stopped: oneshot::Receiver<()>,
) {
    let http = match reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(1))
        .build()
    {
        Ok(http) => http,
        Err(_) => {
            status.send_replace("HTTP初期化失敗".into());
            return;
        }
    };
    let socket = match tokio::net::UdpSocket::bind("127.0.0.1:0").await {
        Ok(socket) => socket,
        Err(_) => {
            status.send_replace("UDP初期化失敗".into());
            return;
        }
    };
    let mut discovery: Option<Discovery> = None;
    let mut timer = tokio::time::interval(Duration::from_secs(5));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let cfg = config.borrow().clone();
        // Cancellation also interrupts pending HTTP checks; sensor and storage loops never wait.
        tokio::select! {
            biased;
            _ = &mut stopped => break,
            changed = config.changed() => {
                if changed.is_err() { break; }
                status.send_replace(if config.borrow().oyasumi_enabled {"接続確認中"}else{"無効"}.into());
                timer.reset_immediately();
            }
            _ = async {
                timer.tick().await;
                if !cfg.oyasumi_enabled {status.send_replace("無効".into());return;}
                if cfg.oyasumi_query_port == 0 && discovery.is_none() {
                    match Discovery::new().await {
                        Ok(d) => discovery = Some(d),
                        Err(_) => {status.send_replace("OSCQuery初期化失敗・再試行中".into());return;}
                    }
                }
                let ports = if cfg.oyasumi_query_port != 0 {vec![cfg.oyasumi_query_port]} else {
                    discovery.as_ref().unwrap().ports.lock().unwrap().iter().copied().collect::<Vec<_>>()
                };
                let checked = futures_util::future::join_all(ports.into_iter().map(|port| { let http = &http; async move {
                    tokio::time::timeout(Duration::from_secs(3), probe(http, port)).await.ok().and_then(Result::ok)
                }})).await;
                let peers = checked.into_iter().flatten().collect::<BTreeSet<_>>();
                if peers.len() != 1 {
                    status.send_replace(if peers.len()>1 {"複数のOyasumiVRを検出・送信保留"} else {"OyasumiVR検出待ち（OSCサーバーを有効にしてください）"}.into());
                    return;
                }
                let (target, current) = *peers.first().unwrap();
                let now = shared_types::now_ms();
                let Some(value) = desired(&snapshots.borrow(), &cfg, now) else {
                    status.send_replace(format!("接続確認済み · 判定不能／対象外のため保持 ({target})"));return;
                };
                if current == value {
                    status.send_replace(format!("{} · OSCQueryで確認済み ({target})", if value {"睡眠"}else{"覚醒"}));return;
                }
                let packet = rosc::OscPacket::Message(rosc::OscMessage { addr: ADDRESS.into(), args: vec![rosc::OscType::Bool(value)] });
                let result = match rosc::encoder::encode(&packet) {Ok(bytes)=>socket.send_to(&bytes,target).await.map(|_|()),Err(_)=>{status.send_replace("OSC符号化失敗".into());return;}};
                status.send_replace(if result.is_ok() {format!("{}を送信 · 応答確認待ち ({target})",if value {"睡眠"}else{"覚醒"})} else {"OSC送信失敗・再試行中".into()});
            } => {}
        }
    }
    if let Some(d) = discovery {
        let _ = d.client.shutdown().await;
    }
    // Disabled, unknown, or app shutdown does not manufacture an awake transition.
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_stale_and_pseudo_are_not_awake_commands() {
        let mut cfg = Settings {
            oyasumi_enabled: true,
            ..Default::default()
        };
        let mut s = PhysioSnapshot {
            timestamp: 10000,
            sensor_connected: true,
            mode: MeasurementMode::HrRr,
            rr_available: true,
            sleep: shared_types::SleepEstimate {
                tracking: true,
                sleep_like: true,
                probability: 0.9,
            },
            ..Default::default()
        };
        s.features.timestamp = 10000;
        assert_eq!(desired(&s, &cfg, 10000), Some(true));
        assert_eq!(desired(&s, &cfg, 16000), None);
        assert_eq!(desired(&s, &cfg, 9000), None);
        s.sleep.sleep_like = false;
        assert_eq!(desired(&s, &cfg, 10000), Some(false));
        s.sleep.tracking = false;
        assert_eq!(desired(&s, &cfg, 10000), None);
        s.sleep.tracking = true;
        s.mode = MeasurementMode::HrOnly;
        s.rr_available = false;
        assert_eq!(desired(&s, &cfg, 10000), None);
        cfg.oyasumi_allow_hr_only = true;
        assert_eq!(desired(&s, &cfg, 10000), Some(false));
        cfg.sleep_enabled = false;
        assert_eq!(desired(&s, &cfg, 10000), None);
        cfg.sleep_enabled = true;
        cfg.oyasumi_enabled = false;
        assert_eq!(desired(&s, &cfg, 10000), None);
    }
    #[test]
    fn official_contract_and_invalid_targets() {
        let mut host = serde_json::json!({"NAME":"OyasumiVR","OSC_PORT":12345,"OSC_TRANSPORT":"UDP","OSC_IP":"10.1.2.3"});
        let mut node =
            serde_json::json!({"FULL_PATH":ADDRESS,"ACCESS":3,"VALUE_TYPE":"F","VALUE":[true]});
        assert_eq!(
            endpoint(&host, &node).unwrap(),
            ("127.0.0.1:12345".parse().unwrap(), true)
        );
        node["VALUE_TYPE"] = "i".into();
        assert!(endpoint(&host, &node).is_err());
        node["TYPE"] = "T".into();
        assert!(endpoint(&host, &node).is_ok());
        node["ACCESS"] = 1.into();
        assert!(endpoint(&host, &node).is_err());
        node["ACCESS"] = 3.into();
        host["NAME"] = "VRChat".into();
        assert!(endpoint(&host, &node).is_err());
        host["NAME"] = "OyasumiVR".into();
        host["OSC_PORT"] = 0.into();
        assert!(endpoint(&host, &node).is_err());
        assert!(is_oyasumi_service("OyasumiVR._oscjson._tcp.local."));
        assert!(!is_oyasumi_service("VRChat-Client-1._oscjson._tcp.local."));
        assert!(!is_oyasumi_service("OyasumiVR._osc._udp.local."));
    }
    #[tokio::test]
    async fn worker_controls_transitions_and_holds_unknown_disabled_shutdown() {
        use axum::{routing::get, Json, Router};
        use std::sync::atomic::{AtomicBool, Ordering};
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let osc_port = udp.local_addr().unwrap().port();
        let current = Arc::new(AtomicBool::new(true));
        let state = current.clone();
        let app = Router::new()
            .route("/", get(move || async move {Json(serde_json::json!({"NAME":"OyasumiVR","OSC_PORT":osc_port,"OSC_TRANSPORT":"UDP"}))}))
            .route(ADDRESS, get(move || {let state=state.clone();async move {Json(serde_json::json!({"FULL_PATH":ADDRESS,"ACCESS":3,"VALUE_TYPE":"F","VALUE":[state.load(Ordering::SeqCst)]}))}}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut cfg = Settings {
            oyasumi_enabled: true,
            oyasumi_query_port: listener.local_addr().unwrap().port(),
            ..Default::default()
        };
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let (tx, rx) = watch::channel(PhysioSnapshot::default());
        let publish = |asleep: bool, tracking: bool| {
            let now = shared_types::now_ms();
            let mut s = PhysioSnapshot {
                timestamp: now,
                sensor_connected: true,
                rr_available: true,
                mode: MeasurementMode::HrRr,
                sleep: shared_types::SleepEstimate {
                    tracking,
                    sleep_like: asleep,
                    probability: 0.9,
                },
                ..Default::default()
            };
            s.features.timestamp = now;
            tx.send_replace(s);
        };
        // Keep timestamps fresh, as the real Core does once a second.
        let freshness = tx.clone();
        let refresh = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(200)).await;
                freshness.send_modify(|s| {
                    s.timestamp = shared_types::now_ms();
                    s.features.timestamp = s.timestamp;
                });
            }
        });
        let receive = |expected: bool| {
            let udp = &udp;
            let current = &current;
            async move {
                let mut bytes = [0; 128];
                let n = tokio::time::timeout(Duration::from_secs(7), udp.recv(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    rosc::decoder::decode_udp(&bytes[..n]).unwrap().1,
                    rosc::OscPacket::Message(rosc::OscMessage {
                        addr: ADDRESS.into(),
                        args: vec![rosc::OscType::Bool(expected)]
                    })
                );
                current.store(expected, Ordering::SeqCst);
            }
        };
        publish(false, true);
        let worker = Worker::start(rx, cfg.clone());
        receive(false).await;
        publish(true, true);
        receive(true).await;
        publish(false, false);
        let mut bytes = [0; 128];
        assert!(
            tokio::time::timeout(Duration::from_secs(6), udp.recv(&mut bytes))
                .await
                .is_err()
        );
        assert!(current.load(Ordering::SeqCst));
        cfg.oyasumi_enabled = false;
        worker.configure(cfg.clone());
        publish(false, true);
        assert!(
            tokio::time::timeout(Duration::from_secs(6), udp.recv(&mut bytes))
                .await
                .is_err()
        );
        cfg.oyasumi_enabled = true;
        worker.configure(cfg.clone());
        receive(false).await;
        publish(true, true);
        receive(true).await;
        worker.stop().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(200), udp.recv(&mut bytes))
                .await
                .is_err()
        );
        assert!(current.load(Ordering::SeqCst));
        refresh.abort();
        server.abort();
    }
}
