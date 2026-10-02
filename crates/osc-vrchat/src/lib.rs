use anyhow::Result;
use rosc::{OscMessage, OscPacket, OscType};
use shared_types::*;
use std::net::{SocketAddr, UdpSocket};
pub fn parameters(s: &PhysioSnapshot, settings: &Settings, beat: bool) -> Vec<OscMessage> {
    let mut out = Vec::new();
    let mut add = |name: &str, value: OscType| {
        out.push(OscMessage {
            addr: format!("/avatar/parameters/VRCOSC/{name}"),
            args: vec![value],
        })
    };
    if settings.heartrate_output {
        let hr = s.heart_rate.unwrap_or(0) as i32;
        let enabled = settings.osc_enabled;
        add(
            "Heartrate/Connected",
            OscType::Bool(s.sensor_connected && enabled),
        );
        add("Heartrate/Enabled", OscType::Bool(enabled));
        add("Heartrate/Value", OscType::Int(hr));
        add("Heartrate/Normalised", OscType::Float(hr as f32 / 240.0));
        add(
            "Heartrate/Average",
            OscType::Int(s.heart_rate_average.unwrap_or(0.0).round() as i32),
        );
        add(
            "Heartrate/Beat",
            OscType::Bool(beat && s.sensor_connected && enabled),
        );
        add("Heartrate/Units", OscType::Float((hr % 10) as f32 / 10.0));
        add(
            "Heartrate/Tens",
            OscType::Float((hr / 10 % 10) as f32 / 10.0),
        );
        add(
            "Heartrate/Hundreds",
            OscType::Float((hr / 100 % 10) as f32 / 10.0),
        );
    }
    if settings.sleep_output {
        let enabled = settings.osc_enabled && settings.sleep_enabled;
        add("Sleep/Enabled", OscType::Bool(enabled));
        add("Sleep/Tracking", OscType::Bool(enabled && s.sleep.tracking));
        add(
            "Sleep/Probability",
            OscType::Float(if enabled && s.sleep.tracking {
                s.sleep.probability.clamp(0.0, 1.0) as f32
            } else {
                0.0
            }),
        );
        add(
            "Sleep/Asleep",
            OscType::Bool(enabled && s.sleep.tracking && s.sleep.sleep_like),
        );
        add(
            "Sleep/Estimated",
            OscType::Bool(enabled && s.sleep.tracking && s.mode == MeasurementMode::HrOnly),
        );
        add(
            "Sleep/MeasurementAvailable",
            OscType::Bool(
                enabled && s.rr_available && s.mode == MeasurementMode::HrRr && s.sleep.tracking,
            ),
        );
    }
    out
}
pub struct OscSender {
    socket: UdpSocket,
    target: SocketAddr,
}
impl OscSender {
    pub fn new(host: &str, port: u16) -> Result<Self> {
        let ip = host.parse::<std::net::IpAddr>()?;
        let socket = UdpSocket::bind(if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            target: SocketAddr::new(ip, port),
        })
    }
    pub fn send(&self, s: &PhysioSnapshot, settings: &Settings, beat: bool) -> Result<()> {
        for m in parameters(s, settings, beat) {
            self.socket
                .send_to(&rosc::encoder::encode(&OscPacket::Message(m))?, self.target)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_types_and_disconnect() {
        let s = PhysioSnapshot::default();
        let cfg = Settings {
            osc_enabled: true,
            ..Default::default()
        };
        let p = parameters(&s, &cfg, false);
        assert_eq!(p.len(), 15);
        assert!(matches!(p[2].args[0], OscType::Int(0)));
        assert!(matches!(p[10].args[0], OscType::Bool(false)));
        for m in p {
            let bytes = rosc::encoder::encode(&OscPacket::Message(m)).unwrap();
            assert!(rosc::decoder::decode_udp(&bytes).is_ok());
        }
    }
    #[test]
    fn sends_local_udp() {
        let recv = UdpSocket::bind("127.0.0.1:0").unwrap();
        recv.set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let sender = OscSender::new("127.0.0.1", recv.local_addr().unwrap().port()).unwrap();
        sender
            .send(&PhysioSnapshot::default(), &Settings::default(), false)
            .unwrap();
        let mut b = [0; 1024];
        let n = recv.recv(&mut b).unwrap();
        assert!(rosc::decoder::decode_udp(&b[..n]).is_ok());
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    #[test]
    fn normalisation_and_digits_match_heartosc() {
        let s = PhysioSnapshot {
            heart_rate: Some(123),
            heart_rate_average: Some(120.0),
            ..Default::default()
        };
        let p = parameters(&s, &Settings::default(), false);
        assert_eq!(p[3].args, vec![OscType::Float(123.0 / 240.0)]);
        assert_eq!(p[4].args, vec![OscType::Int(120)]);
        assert_eq!(p[6].args, vec![OscType::Float(0.3)]);
        assert_eq!(p[7].args, vec![OscType::Float(0.2)]);
        assert_eq!(p[8].args, vec![OscType::Float(0.1)]);
    }
}

#[cfg(test)]
mod sleep_outputs {
    use super::*;
    #[test]
    fn pseudo_sleep_is_explicit_and_not_measurement() {
        let s = PhysioSnapshot {
            mode: MeasurementMode::HrOnly,
            sleep: SleepEstimate {
                tracking: true,
                sleep_like: true,
                probability: 0.9,
            },
            ..Default::default()
        };
        let cfg = Settings {
            osc_enabled: true,
            ..Default::default()
        };
        let messages = parameters(&s, &cfg, false);
        let boolean = |suffix: &str| {
            messages
                .iter()
                .find(|m| m.addr.ends_with(suffix))
                .unwrap()
                .args[0]
                .clone()
        };
        assert_eq!(boolean("/Asleep"), OscType::Bool(true));
        assert_eq!(boolean("/Estimated"), OscType::Bool(true));
        assert_eq!(boolean("/MeasurementAvailable"), OscType::Bool(false));
        let disabled = parameters(&s, &Settings::default(), false);
        assert_eq!(
            disabled
                .iter()
                .find(|m| m.addr.ends_with("/Asleep"))
                .unwrap()
                .args[0],
            OscType::Bool(false)
        );
    }
}

/// One discovery client for the application lifetime; HTTP checks run off the sensor loop.
pub struct QuerySender {
    client: std::sync::Arc<vrchat_osc::VRChatOSC>,
    targets: std::sync::Arc<std::sync::Mutex<Vec<SocketAddr>>>,
    status: std::sync::Arc<std::sync::Mutex<String>>,
    enabled: tokio::sync::watch::Sender<bool>,
    probe: tokio::task::JoinHandle<()>,
}
#[derive(Clone, Default)]
struct Candidate {
    osc: Option<SocketAddr>,
    query: Option<SocketAddr>,
}
fn vrchat_name(name: &str, query: bool) -> Option<&str> {
    let suffix = if query {
        "._oscjson._tcp.local."
    } else {
        "._osc._udp.local."
    };
    name.strip_suffix(suffix)
        .filter(|name| name.starts_with("VRChat-Client-"))
}
impl QuerySender {
    pub async fn new() -> Result<Self> {
        let client = match vrchat_osc::VRChatOSC::new(None).await {
            Ok(client) => client,
            Err(vrchat_osc::Error::NoValidInterface) => {
                vrchat_osc::VRChatOSC::new(Some("127.0.0.1".parse()?)).await?
            }
            Err(error) => return Err(error.into()),
        };
        let targets = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let status = std::sync::Arc::new(std::sync::Mutex::new("OSCQuery: VRChat検出待ち".into()));
        let candidates = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
            String,
            Candidate,
        >::new()));
        let discovered = candidates.clone();
        client
            .on_connect(move |service| {
                let (name, addr, query) = match service {
                    vrchat_osc::ServiceType::Osc(name, addr) => (name, addr, false),
                    vrchat_osc::ServiceType::OscQuery(name, addr) => (name, addr, true),
                };
                if let Some(name) = vrchat_name(&name, query) {
                    let mut map = discovered.lock().unwrap();
                    // VRChat normally exposes one client. Bound unsolicited network advertisements.
                    if map.len() >= 8 && !map.contains_key(name) {
                        let oldest = map.keys().next().cloned();
                        if let Some(key) = oldest {
                            map.remove(&key);
                        }
                    }
                    let candidate = map.entry(name.into()).or_default();
                    if query {
                        candidate.query = Some(addr);
                    } else {
                        candidate.osc = Some(addr);
                    }
                }
            })
            .await;
        let (enabled, mut active) = tokio::sync::watch::channel(true);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(2))
            .build()?;
        let t = targets.clone();
        let s = status.clone();
        let probe = tokio::spawn(async move {
            let mut timer = tokio::time::interval(std::time::Duration::from_secs(5));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _=timer.tick()=>{}, changed=active.changed()=>{if changed.is_err(){break;}} }
                if !*active.borrow() {
                    t.lock().unwrap().clear();
                    continue;
                }
                let candidates = candidates.lock().unwrap().clone();
                let checks = candidates
                    .into_values()
                    .filter_map(|candidate| Some((candidate.osc?, candidate.query?)))
                    .map(|(osc, query)| {
                        let http = http.clone();
                        async move {
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(2),
                                verify_query(&http, query),
                            )
                            .await
                            {
                                Ok(Ok(true)) => Some(osc),
                                _ => None,
                            }
                        }
                    });
                let checked = futures_util::future::join_all(checks).await;
                if !*active.borrow() {
                    t.lock().unwrap().clear();
                    continue;
                }
                let verified = checked.into_iter().flatten().collect::<Vec<_>>();
                *s.lock().unwrap() = if verified.is_empty() {
                    "OSCQuery: VRChat検出待ち（VRChat側のOSCを有効にしてください）".into()
                } else {
                    format!(
                        "OSCQuery: VRChat検出・送信中 ({})",
                        verified
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                *t.lock().unwrap() = verified;
            }
        });
        Ok(Self {
            client,
            targets,
            status,
            enabled,
            probe,
        })
    }
    pub fn set_enabled(&self, enabled: bool) {
        if *self.enabled.borrow() != enabled {
            self.targets.lock().unwrap().clear();
            *self.status.lock().unwrap() = "OSCQuery: VRChat検出待ち".into();
        }
        self.enabled.send_if_modified(|v| {
            if *v == enabled {
                false
            } else {
                *v = enabled;
                true
            }
        });
    }
    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }
    pub async fn send(&self, s: &PhysioSnapshot, settings: &Settings, beat: bool) -> Result<()> {
        let targets = self.targets.lock().unwrap().clone();
        for target in targets {
            for message in parameters(s, settings, beat) {
                tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    self.client
                        .send_to_addr(OscPacket::Message(message), target),
                )
                .await??;
            }
        }
        Ok(())
    }
    pub async fn shutdown(&self) {
        self.probe.abort();
        let _ = self.client.shutdown().await;
    }
}
impl Drop for QuerySender {
    fn drop(&mut self) {
        self.probe.abort();
    }
}
async fn verify_query(client: &reqwest::Client, address: SocketAddr) -> Result<bool> {
    let mut response = client
        .get(format!("http://{address}/"))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 4 * 1024 * 1024,
            "OSCQuery response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let node: vrchat_osc::models::OscNode = serde_json::from_slice(&bytes)?;
    Ok(node.full_path == "/")
}
#[cfg(test)]
mod discovery_tests {
    use super::*;
    #[test]
    fn only_pairs_vrchat_services() {
        assert_eq!(
            vrchat_name("VRChat-Client-123._osc._udp.local.", false),
            Some("VRChat-Client-123")
        );
        assert_eq!(
            vrchat_name("VRChat-Client-123._oscjson._tcp.local.", true),
            Some("VRChat-Client-123")
        );
        assert_eq!(vrchat_name("Other._osc._udp.local.", false), None);
        assert_eq!(
            vrchat_name("VRChat-Client-123._oscjson._tcp.local.", false),
            None
        );
    }
}
