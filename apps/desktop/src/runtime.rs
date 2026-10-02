use anyhow::Result;
use physio_core::RealtimeCore;
use sensor_core::{Device, SensorAdapter, SensorMessage};
use shared_types::*;
use sleep_engine::{SessionTracker, SleepEngine};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use storage::{Storage, Write};
use tokio::sync::{mpsc, watch};
#[derive(Default)]
pub struct View {
    pub devices: Vec<Device>,
    pub message: String,
    pub sessions: Vec<SleepSession>,
    pub history: Vec<FeatureMinute>,
    pub baseline: Baseline,
    pub active: Option<SleepSession>,
    pub display: VecDeque<PhysioSnapshot>,
    pub osc_status: String,
    pub connecting: bool,
    pub authorization: Option<sensor_pulsoid::oauth::Authorization>,
    pub history_request: u64,
}
pub enum Command {
    Scan,
    Connect(String),
    ConnectPulsoid(String),
    AuthorizePulsoid,
    Disconnect,
    EndSession,
    Save(Settings),
    SelectSession(String),
    Shutdown,
}
#[derive(Default)]
struct AutoRecorder {
    suppressed: bool,
}
#[derive(Debug, PartialEq)]
enum RecordingAction {
    None,
    Start,
    Stop,
}
impl AutoRecorder {
    fn decide(&mut self, s: &PhysioSnapshot, active: bool, enabled: bool) -> RecordingAction {
        let measured = s.mode == MeasurementMode::HrRr && s.rr_available && s.sleep.tracking;
        if !s.sleep.sleep_like {
            self.suppressed = false;
        }
        if active && (!measured || !s.sleep.sleep_like || !enabled) {
            return RecordingAction::Stop;
        }
        if enabled && measured && s.sleep.sleep_like && !active && !self.suppressed {
            RecordingAction::Start
        } else {
            RecordingAction::None
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub async fn run(
    core: RealtimeCore,
    storage: Storage,
    database: PathBuf,
    cache: Storage,
    cache_path: PathBuf,
    mut settings: Settings,
    view: Arc<Mutex<View>>,
    mut commands: mpsc::Receiver<Command>,
    demo: bool,
    demo_hr_only: bool,
    demo_sleep: bool,
    ipc_path: Option<String>,
) -> Result<()> {
    let (sensor_tx, mut sensor_rx) = mpsc::channel(512);
    let (auth_tx, mut auth_rx) = mpsc::channel(4);
    let mut adapter: Option<Arc<sensor_ble::GenericBleHRAdapter>> = None;
    let mut sensor_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut sensor_stop: Option<watch::Sender<bool>> = None;
    let (baseline_tx, mut baseline_rx) = mpsc::channel(4);
    let mut sleep = SleepEngine::default();
    let mut recorder = AutoRecorder::default();
    let mut session: Option<SessionTracker> = None;
    let run_id = uuid::Uuid::new_v4().to_string();
    let mut baseline = read_baseline(&cache_path, &database, settings.baseline_days)?;
    if demo {
        baseline = Baseline {
            resting_hr: Some(72.0),
            rmssd: Some(35.0),
            sample_minutes: 30,
        };
        core.connected("DEMO · synthetic".into());
    }
    view.lock().unwrap().baseline = baseline.clone();
    refresh(view.clone(), database.clone(), None);
    let mut api_task = settings.api_enabled.then(|| {
        start_api(
            core.clone(),
            database.clone(),
            settings.api_port,
            view.clone(),
        )
    });
    let ipc = ipc_path.map(|path| {
        let c = core.clone();
        let v = view.clone();
        tokio::spawn(async move {
            #[cfg(unix)]
            if let Err(e) = ipc::serve(c, std::path::Path::new(&path)).await {
                v.lock().unwrap().message = format!("IPC: {e}");
            }
            #[cfg(not(unix))]
            {
                let _ = (c, path);
                v.lock().unwrap().message =
                    "Windowsの高速IPCは未対応です。ローカルRESTを使用してください".into();
            }
        })
    });
    let mut query: Option<osc_vrchat::QuerySender> = None;
    let mut osc = osc_vrchat::OscSender::new(&settings.osc_host, settings.osc_port).ok();
    let mut events = core.events.subscribe();
    let mut beat_until = 0;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut osc_tick = tokio::time::interval(Duration::from_millis(250));
    osc_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut minutes = MinuteAccumulator::new(now_ms());
    let mut demo_step = 0u64;
    let mut previous_mode = MeasurementMode::Unavailable;
    loop {
        tokio::select! {
         cmd=commands.recv()=>match cmd {
          Some(Command::Shutdown)|None=>break,
          Some(Command::Scan)=>{
           if sensor_task.is_some(){view.lock().unwrap().message="先に現在の入力を切断してください".into();continue;}
           if demo{view.lock().unwrap().message="デモ中は実機へ接続できません".into();continue;}
           view.lock().unwrap().message="Scanning BLE devices…".into();
           let result=async{if adapter.is_none(){adapter=Some(Arc::new(sensor_ble::GenericBleHRAdapter::new().await?));}adapter.as_ref().unwrap().scan().await}.await;
           match result {Ok(devices)=>{let mut v=view.lock().unwrap();v.devices=devices;v.message="Scan complete; select a heart rate sensor".into();},Err(e)=>view.lock().unwrap().message=e.to_string()}
          },
          Some(Command::Connect(id))=>{
           if demo || sensor_task.is_some(){continue;}
           if let Some(a)=adapter.clone(){let (tx,rx)=watch::channel(false);sensor_stop=Some(tx);view.lock().unwrap().connecting=true;sensor_task=Some(spawn_sensor(a,id,sensor_tx.clone(),rx,view.clone()));}
          },
          Some(Command::AuthorizePulsoid)=>{
           if demo || sensor_task.is_some(){continue;}
           let (tx,rx)=watch::channel(false);sensor_stop=Some(tx);view.lock().unwrap().connecting=true;
           view.lock().unwrap().message="Pulsoid OAuth認証を開始中…".into();
           sensor_task=Some(spawn_sensor(Arc::new(sensor_pulsoid::oauth::DeviceOAuthAdapter{updates:auth_tx.clone()}),"Pulsoid".into(),sensor_tx.clone(),rx,view.clone()));
          },
          Some(Command::ConnectPulsoid(token))=>{
           if demo || sensor_task.is_some(){view.lock().unwrap().message="先に入力を切断してください（デモではPulsoidを使用できません）".into();continue;}
           match sensor_pulsoid::PulsoidAdapter::new(token){Ok(a)=>{let (tx,rx)=watch::channel(false);sensor_stop=Some(tx);view.lock().unwrap().connecting=true;view.lock().unwrap().message="Pulsoidへ接続中…".into();sensor_task=Some(spawn_sensor(Arc::new(a),"Pulsoid".into(),sensor_tx.clone(),rx,view.clone()));},Err(e)=>view.lock().unwrap().message=e.to_string()}
          },
          Some(Command::Disconnect)=>{
           stop_sensor(&mut sensor_stop,&mut sensor_task).await;while sensor_rx.try_recv().is_ok(){}while auth_rx.try_recv().is_ok(){}core.disconnected();sleep=SleepEngine::default();
           minutes.flush(&storage,&cache,&baseline,session.as_mut());minutes.started=now_ms();end_session(&core,&storage,&mut session,now_ms());
           let mut v=view.lock().unwrap();v.connecting=false;v.authorization=None;v.active=None;v.message="入力を切断しました".into();storage.push(Write::Gap{timestamp:now_ms(),reason:"User disconnected input".into()});
          },
          Some(Command::EndSession)=>{minutes.flush(&storage,&cache,&baseline,session.as_mut());minutes.started=now_ms();end_session(&core,&storage,&mut session,now_ms());recorder.suppressed=true;view.lock().unwrap().active=None;refresh(view.clone(),database.clone(),None);},
          Some(Command::Save(new))=>{
           if let Err(e)=new.validate(){view.lock().unwrap().message=e;continue;}
           if !storage.push(Write::Settings(new.clone())){view.lock().unwrap().message="設定の保存キューが満杯です".into();continue;}
           if settings.osc_enabled {let off=Settings{osc_enabled:false,..settings.clone()};let _=send_osc(osc.as_ref(),query.as_ref(),&core.latest(),&off,false).await;}
           if let Some(task)=api_task.take(){task.abort();let _=task.await;}
           if new.api_enabled{api_task=Some(start_api(core.clone(),database.clone(),new.api_port,view.clone()));}
           osc=osc_vrchat::OscSender::new(&new.osc_host,new.osc_port).ok();
           if settings.sleep_confirm_secs!=new.sleep_confirm_secs || settings.pseudo_sleep_hr!=new.pseudo_sleep_hr || settings.sleep_enabled!=new.sleep_enabled {sleep=SleepEngine::default();}
           settings=new;
           if !demo{load_baseline(cache_path.clone(),database.clone(),settings.baseline_days,baseline_tx.clone());}
           view.lock().unwrap().message="設定を適用しました（非同期で保存中）".into();
          },
          Some(Command::SelectSession(id))=>refresh(view.clone(),database.clone(),Some(id)),
         },
         Some(authorization)=auth_rx.recv()=>{view.lock().unwrap().authorization=authorization;},
         Some(result)=baseline_rx.recv()=>match result{Ok((days,b)) if days==settings.baseline_days=>{baseline=b;let _=core.events.send(Event::BaselineUpdated(baseline.clone()));},Ok(_)=>{},Err(e)=>view.lock().unwrap().message=e},
         Some(message)=sensor_rx.recv()=>match message {
          SensorMessage::Connected(id)=>{core.connected(id);sleep=SleepEngine::default();view.lock().unwrap().message="入力を受信しています".into();},
          SensorMessage::Disconnected(reason)=>{
           core.disconnected();sleep=SleepEngine::default();storage.push(Write::Gap{timestamp:now_ms(),reason:reason.clone()});view.lock().unwrap().message=reason;
           minutes.flush(&storage,&cache,&baseline,session.as_mut());minutes.started=now_ms();end_session(&core,&storage,&mut session,now_ms());view.lock().unwrap().active=None;
          },
          SensorMessage::Measurement(m)=>{ingest(&core,&storage,&run_id,&mut minutes,m);},
         },
         event=events.recv()=>if let Ok(Event::HeartBeat{..})=event {beat_until=now_ms()+100;if settings.osc_enabled{let mut cfg=settings.clone();cfg.sleep_output=false;let _=send_osc(osc.as_ref(),query.as_ref(),&core.latest(),&cfg,true).await;}},
         _=tick.tick()=>{
          let now=now_ms();
          if sensor_task.as_ref().is_some_and(|t|t.is_finished()){sensor_task.take();sensor_stop.take();view.lock().unwrap().connecting=false;}
          if now/60000!=minutes.started/60000 {
           minutes.flush(&storage,&cache,&baseline,session.as_mut());minutes.started=now/60000*60000;
           cache.push(Write::PruneCache{before:now-35*86400000});
           if !demo{load_baseline(cache_path.clone(),database.clone(),settings.baseline_days,baseline_tx.clone());}
          }
          if demo {demo_step+=1;let hr=if demo_sleep {50}else{(65.0+8.0*(demo_step as f64/40.0).sin())as u16};ingest(&core,&storage,&run_id,&mut minutes,Measurement{timestamp:now,source_timestamp:None,sensor_id:"DEMO".into(),heart_rate:hr,rr_ms:if demo_hr_only {vec![]}else{vec![60000.0/f64::from(hr)+20.0*(demo_step as f64).sin()]},contact:Some(true)});}
          if core.expire(now){storage.push(Write::Gap{timestamp:now,reason:"No fresh measurement for 5 seconds".into()});}
          let snapshot=core.latest();
          if snapshot.mode!=previous_mode {minutes.flush(&storage,&cache,&baseline,session.as_mut());minutes.started=now;previous_mode=snapshot.mode;}
          let samples=core.window(now,60000);let mut features=feature_engine::calculate(&samples,now,&baseline);
          let estimate=if snapshot.mode==MeasurementMode::HrOnly {
           features.rmssd=None;features.sdnn=None;features.pnn50=None;features.valid_rr=0;
           let valid_hr=snapshot.sensor_connected && samples.last().is_some_and(|m|m.contact!=Some(false)&&(25..=240).contains(&m.heart_rate));
           features.quality=if valid_hr{1.0}else{0.0};
           sleep.update_hr_only(&features,valid_hr,settings.sleep_enabled,now,settings.pseudo_sleep_hr,settings.sleep_confirm_secs)
          }else {sleep.update_configured(&features,&baseline,snapshot.sensor_connected,settings.sleep_enabled,now,settings.sleep_confirm_secs)};
          // Only RR-based minute scores enter sleep history/baseline; HR-only output is ephemeral.
          if estimate.tracking && snapshot.mode==MeasurementMode::HrRr{minutes.probabilities.push(estimate.probability);}
          core.set_analysis(features,estimate);let latest=core.latest();
          match recorder.decide(&latest,session.is_some(),settings.auto_sleep_recording) {
           RecordingAction::Start=>{
            minutes.flush(&storage,&cache,&baseline,None);
            let since=sleep.confirmed_since.unwrap_or(now);let mut tracker=SessionTracker::new(since);
            tracker.session.estimated_sleep_at=Some(since);tracker.session.estimated_sleep_minutes=(now-since).max(0)as f64/60000.0;
            for minute in &minutes.recent {if minute.features.timestamp>=since {let mut minute=minute.clone();minute.session_id=Some(tracker.session.session_id.clone());tracker.add_minute(&minute);storage.push(Write::Minute(minute));}}
            storage.push(Write::Session(tracker.session.clone()));let _=core.events.send(Event::SleepSessionStarted(tracker.session.session_id.clone()));
            session=Some(tracker);minutes.started=now;view.lock().unwrap().message="睡眠の継続を確認し、自動記録を開始しました".into();
           },
           RecordingAction::Stop=>{minutes.flush(&storage,&cache,&baseline,session.as_mut());minutes.started=now;end_session(&core,&storage,&mut session,now);refresh(view.clone(),database.clone(),None);},
           RecordingAction::None=>{}
          }
          if let Some(t)=&mut session{t.update(&latest,now);storage.push(Write::Session(t.session.clone()));}
          let mut v=view.lock().unwrap();v.baseline=baseline.clone();v.active=session.as_ref().map(|s|s.session.clone());v.display.push_back(PhysioSnapshot{timestamp:now,..latest});while v.display.len()>1805 {v.display.pop_front();}
          if let Some(error)=storage.health.error.lock().unwrap().clone(){let _=core.events.send(Event::StorageError(error));}
          if let Some(error)=cache.health.error.lock().unwrap().clone(){v.message=format!("Cache保存を再試行中: {error}");}
         },
         _=osc_tick.tick()=>{
          if settings.osc_enabled && settings.oscquery_enabled && query.is_none(){
           match osc_vrchat::QuerySender::new().await{Ok(q)=>query=Some(q),Err(e)=>{view.lock().unwrap().osc_status=format!("OSC error: OSCQuery initialization: {e}");continue;}}
          }
          if let Some(q)=&query{q.set_enabled(settings.osc_enabled && settings.oscquery_enabled);}
          if settings.osc_enabled{
           let result=send_osc(osc.as_ref(),query.as_ref(),&core.latest(),&settings,now_ms()<beat_until).await;
           view.lock().unwrap().osc_status=match result{Ok(()) if settings.oscquery_enabled=>query.as_ref().map(|q|q.status()).unwrap_or_else(||"OSCQuery: VRChat検出待ち".into()),Ok(())=>format!("UDP → {}:{} (VRChat presence unverified)",settings.osc_host,settings.osc_port),Err(e)=>format!("OSC error: {e}")};
          }else{view.lock().unwrap().osc_status="Disabled".into();}
         }
        }
    }
    stop_sensor(&mut sensor_stop, &mut sensor_task).await;
    while let Ok(message) = sensor_rx.try_recv() {
        if let SensorMessage::Measurement(m) = message {
            ingest(&core, &storage, &run_id, &mut minutes, m);
        }
    }
    minutes.flush(&storage, &cache, &baseline, session.as_mut());
    end_session(&core, &storage, &mut session, now_ms());
    core.disconnected();
    let _ = send_osc(
        osc.as_ref(),
        query.as_ref(),
        &core.latest(),
        &Settings {
            osc_enabled: false,
            ..settings
        },
        false,
    )
    .await;
    if let Some(q) = query {
        q.shutdown().await;
    }
    if let Some(t) = api_task {
        t.abort();
        let _ = t.await;
    }
    if let Some(t) = ipc {
        t.abort();
        let _ = t.await;
    }
    Ok(())
}
struct MinuteAccumulator {
    started: i64,
    raw: Vec<Measurement>,
    probabilities: Vec<f64>,
    recent: VecDeque<FeatureMinute>,
}
impl MinuteAccumulator {
    fn new(now: i64) -> Self {
        Self {
            started: now,
            raw: vec![],
            probabilities: vec![],
            recent: VecDeque::new(),
        }
    }
    fn flush(
        &mut self,
        personal: &Storage,
        cache: &Storage,
        baseline: &Baseline,
        session: Option<&mut SessionTracker>,
    ) {
        if self.raw.is_empty() {
            self.probabilities.clear();
            return;
        }
        let f = feature_engine::calculate(&self.raw, self.raw.last().unwrap().timestamp, baseline);
        let mut m = FeatureMinute {
            timestamp: self.started,
            features: f,
            sleep_probability: if self.probabilities.is_empty() {
                None
            } else {
                Some(self.probabilities.iter().sum::<f64>() / self.probabilities.len() as f64)
            },
            session_id: None,
            sample_count: self.raw.len(),
        };
        cache.push(Write::Minute(m.clone()));
        self.recent.push_back(m.clone());
        while self.recent.len() > 36 {
            self.recent.pop_front();
        }
        if let Some(s) = session {
            m.session_id = Some(s.session.session_id.clone());
            s.add_minute(&m);
            personal.push(Write::Minute(m));
        }
        self.raw.clear();
        self.probabilities.clear();
    }
}
fn ingest(
    core: &RealtimeCore,
    storage: &Storage,
    run_id: &str,
    minutes: &mut MinuteAccumulator,
    m: Measurement,
) {
    let sequence = core.ingest(m.clone());
    let quality =
        feature_engine::calculate(std::slice::from_ref(&m), m.timestamp, &Baseline::default())
            .quality;
    storage.push(Write::Raw {
        run_id: run_id.into(),
        sequence,
        measurement: m.clone(),
        quality,
    });
    minutes.raw.push(m);
}
fn end_session(
    core: &RealtimeCore,
    storage: &Storage,
    session: &mut Option<SessionTracker>,
    now: i64,
) {
    if let Some(mut t) = session.take() {
        t.end(now);
        storage.push(Write::Session(t.session.clone()));
        let _ = core
            .events
            .send(Event::SleepSessionEnded(t.session.session_id));
    }
}
fn spawn_sensor(
    adapter: Arc<dyn SensorAdapter>,
    id: String,
    output: mpsc::Sender<SensorMessage>,
    stop: watch::Receiver<bool>,
    view: Arc<Mutex<View>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = adapter.run(id, output.clone(), stop).await {
            let message = error.to_string();
            view.lock().unwrap().message = message.clone();
            let _ = output.send(SensorMessage::Disconnected(message)).await;
        }
    })
}
async fn stop_sensor(
    stop: &mut Option<watch::Sender<bool>>,
    task: &mut Option<tokio::task::JoinHandle<()>>,
) {
    if let Some(s) = stop.take() {
        let _ = s.send(true);
    }
    if let Some(mut t) = task.take() {
        if tokio::time::timeout(Duration::from_secs(5), &mut t)
            .await
            .is_err()
        {
            t.abort();
            let _ = t.await;
        }
    }
}
fn start_api(
    core: RealtimeCore,
    database: PathBuf,
    port: u16,
    view: Arc<Mutex<View>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = api::serve(api::ApiState { core, database }, port).await {
            view.lock().unwrap().message = format!("API: {e}");
        }
    })
}
fn refresh(view: Arc<Mutex<View>>, path: PathBuf, id: Option<String>) {
    let request = {
        let mut v = view.lock().unwrap();
        v.history_request += 1;
        v.history_request
    };
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || -> Result<_> {
            let sessions = storage::sessions(&path, 0)?;
            let id = id.or_else(|| sessions.first().map(|s| s.session_id.clone()));
            let history = if let Some(id) = id {
                storage::history(&path, 0, Some(&id))?
            } else {
                vec![]
            };
            Ok((sessions, history))
        })
        .await;
        let mut v = view.lock().unwrap();
        if v.history_request != request {
            return;
        }
        match result {
            Ok(Ok((sessions, history))) => {
                v.sessions = sessions;
                v.history = history;
            }
            Ok(Err(e)) => v.message = e.to_string(),
            Err(e) => v.message = e.to_string(),
        }
    });
}
fn read_baseline(
    cache: &std::path::Path,
    personal: &std::path::Path,
    days: u32,
) -> Result<Baseline> {
    let current = storage::baseline(cache, days, now_ms())?;
    if current.sample_minutes >= 30 {
        Ok(current)
    } else {
        let old = storage::baseline(personal, days, now_ms())?;
        Ok(if old.sample_minutes > current.sample_minutes {
            old
        } else {
            current
        })
    }
}
fn load_baseline(
    cache: PathBuf,
    personal: PathBuf,
    days: u32,
    tx: mpsc::Sender<Result<(u32, Baseline), String>>,
) {
    tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || read_baseline(&cache, &personal, days)).await;
        let result = match result {
            Ok(Ok(b)) => Ok((days, b)),
            Ok(Err(e)) => Err(e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(result).await;
    });
}
async fn send_osc(
    manual: Option<&osc_vrchat::OscSender>,
    query: Option<&osc_vrchat::QuerySender>,
    snapshot: &PhysioSnapshot,
    settings: &Settings,
    beat: bool,
) -> Result<()> {
    if settings.oscquery_enabled {
        if let Some(q) = query {
            q.send(snapshot, settings, beat).await
        } else {
            Ok(())
        }
    } else {
        manual
            .ok_or_else(|| anyhow::anyhow!("OSC socket unavailable"))?
            .send(snapshot, settings, beat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hr_only_never_records_and_rr_loss_stops() {
        let mut recorder = AutoRecorder::default();
        let mut s = PhysioSnapshot {
            mode: MeasurementMode::HrOnly,
            sleep: SleepEstimate {
                tracking: true,
                sleep_like: true,
                probability: 0.9,
            },
            ..Default::default()
        };
        assert_eq!(recorder.decide(&s, false, true), RecordingAction::None);
        s.mode = MeasurementMode::HrRr;
        s.rr_available = true;
        assert_eq!(recorder.decide(&s, false, true), RecordingAction::Start);
        s.rr_available = false;
        assert_eq!(recorder.decide(&s, true, true), RecordingAction::Stop);
    }
    #[test]
    fn manual_end_waits_for_next_sleep() {
        let mut recorder = AutoRecorder { suppressed: true };
        let mut s = PhysioSnapshot {
            mode: MeasurementMode::HrRr,
            rr_available: true,
            sleep: SleepEstimate {
                tracking: true,
                sleep_like: true,
                probability: 0.9,
            },
            ..Default::default()
        };
        assert_eq!(recorder.decide(&s, false, true), RecordingAction::None);
        s.sleep.sleep_like = false;
        recorder.decide(&s, false, true);
        s.sleep.sleep_like = true;
        assert_eq!(recorder.decide(&s, false, true), RecordingAction::Start);
    }
}
