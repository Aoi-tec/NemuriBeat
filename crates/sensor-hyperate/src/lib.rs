//! HypeRate heart-rate stream. API keys stay in memory and never enter error messages.
use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use sensor_core::{Device, SensorAdapter, SensorMessage};
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::{now_ms, Measurement};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};

#[derive(Deserialize)]
struct Frame {
    #[serde(default)]
    topic: String,
    event: String,
    #[serde(default)]
    payload: Value,
    #[serde(default, rename = "ref")]
    reference: Value,
}
pub fn parse_message(text: &str, id: &str, received_at: i64) -> Result<Option<Measurement>> {
    let frame: Frame =
        serde_json::from_str(text).map_err(|_| anyhow::anyhow!("HypeRate: invalid message"))?;
    if frame.event != "hr_update" || frame.topic != format!("hr:{id}") {
        return Ok(None);
    }
    let heart_rate = frame
        .payload
        .get("hr")
        .and_then(Value::as_u64)
        .filter(|hr| (1..=300).contains(hr))
        .ok_or_else(|| anyhow::anyhow!("HypeRate: invalid BPM"))?;
    Ok(Some(Measurement {
        timestamp: received_at,
        source_timestamp: None,
        sensor_id: "HypeRate".into(),
        heart_rate: heart_rate as u16,
        rr_ms: vec![],
        contact: None,
    }))
}
pub struct HypeRateAdapter {
    id: String,
    key: String,
}
impl HypeRateAdapter {
    pub fn new(id: String, key: String) -> Result<Self> {
        let id = id.trim().to_string();
        let key = key.trim().to_string();
        anyhow::ensure!(
            !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
            "HypeRate IDを入力してください（英数字・ハイフン・アンダースコア）"
        );
        anyhow::ensure!(
            !key.is_empty() && key.len() <= 4096 && !key.contains(['\r', '\n']),
            "HypeRate APIキーを入力してください"
        );
        Ok(Self { id, key })
    }
    fn url(&self) -> url::Url {
        let mut url = url::Url::parse("wss://app.hyperate.io/ws/").unwrap();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .push(&self.id);
        url.query_pairs_mut().append_pair("token", &self.key);
        url
    }
    async fn run_at(
        &self,
        url: url::Url,
        output: mpsc::Sender<SensorMessage>,
        mut stop: watch::Receiver<bool>,
    ) -> Result<()> {
        let mut delay = 1;
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            let connection = tokio_tungstenite::connect_async_with_config(
                url.as_str(),
                Some(WebSocketConfig::default().max_message_size(Some(65536))),
                false,
            );
            let result = tokio::select! {_=stop.changed()=>return Ok(()),result=tokio::time::timeout(Duration::from_secs(15),connection)=>result};
            match result {
                Ok(Ok((mut socket, _))) => {
                    let join = json!({"topic":format!("hr:{}",self.id),"event":"phx_join","payload":{},"ref":"nemuri-join"});
                    let joined = tokio::time::timeout(
                        Duration::from_secs(2),
                        socket.send(Message::Text(join.to_string().into())),
                    )
                    .await;
                    if matches!(joined, Ok(Ok(()))) {
                        let mut heartbeat = tokio::time::interval_at(
                            tokio::time::Instant::now() + Duration::from_secs(15),
                            Duration::from_secs(15),
                        );
                        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                        let mut live = false;
                        let mut accepted = false;
                        let mut last_data = tokio::time::Instant::now();
                        let join_deadline = last_data + Duration::from_secs(10);
                        loop {
                            tokio::select! {
                                _=stop.changed()=>{
                                    let leave=json!({"topic":format!("hr:{}",self.id),"event":"phx_leave","payload":{},"ref":"nemuri-leave"});
                                    let _=tokio::time::timeout(Duration::from_secs(1),async { let _=socket.send(Message::Text(leave.to_string().into())).await;let _=socket.close(None).await; }).await;
                                    return Ok(());
                                },
                                _=tokio::time::sleep_until(join_deadline),if !accepted=>break,
                                _=tokio::time::sleep_until(last_data+Duration::from_secs(30))=>break,
                                _=heartbeat.tick()=>{
                                    let ping=json!({"event":"ping","payload":{"timestamp":now_ms()}});
                                    if !matches!(tokio::time::timeout(Duration::from_secs(2),socket.send(Message::Text(ping.to_string().into()))).await,Ok(Ok(()))){break;}
                                },
                                message=socket.next()=>match message {
                                    Some(Ok(Message::Text(text)))=>{
                                        if let Ok(frame)=serde_json::from_str::<Frame>(&text) {
                                            if frame.event=="phx_reply" && frame.reference=="nemuri-join" {
                                                if frame.payload.get("status").and_then(Value::as_str)!=Some("ok") {anyhow::bail!("HypeRateチャンネルへの参加が拒否されました。IDとAPIキーを確認してください");}
                                                accepted=true;
                                            }
                                            if frame.topic==format!("hr:{}",self.id) && matches!(frame.event.as_str(),"phx_error"|"phx_close"){break;}
                                        }
                                        if let Ok(Some(measurement))=parse_message(&text,&self.id,now_ms()) {
                                            accepted=true;last_data=tokio::time::Instant::now();delay=1;
                                            if !live{output.send(SensorMessage::Connected("HypeRate".into())).await?;live=true;}
                                            output.send(SensorMessage::Measurement(measurement)).await?;
                                        }
                                    },
                                    Some(Ok(Message::Ping(data)))=>{if !matches!(tokio::time::timeout(Duration::from_secs(2),socket.send(Message::Pong(data))).await,Ok(Ok(()))){break;}},
                                    Some(Ok(Message::Close(_)))|Some(Err(_))|None=>break,
                                    _=>{}
                                }
                            }
                        }
                    }
                }
                Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
                    if matches!(response.status().as_u16(), 401 | 403) =>
                {
                    anyhow::bail!("HypeRate認証エラー。APIキーとIDを確認してください")
                }
                _ => {}
            }
            output
                .send(SensorMessage::Disconnected(format!(
                    "HypeRateの配信が途切れました（{delay}秒後に再接続）"
                )))
                .await?;
            tokio::select! {_=stop.changed()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
            delay = (delay * 2).min(30);
        }
    }
}
#[async_trait]
impl SensorAdapter for HypeRateAdapter {
    async fn scan(&self) -> Result<Vec<Device>> {
        Ok(vec![Device {
            id: "HypeRate".into(),
            name: "HypeRate · HR only".into(),
        }])
    }
    async fn run(
        &self,
        _id: String,
        output: mpsc::Sender<SensorMessage>,
        stop: watch::Receiver<bool>,
    ) -> Result<()> {
        self.run_at(self.url(), output, stop).await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn does_not_invent_rr_or_source_clock() {
        let m = parse_message(
            r#"{"topic":"hr:abc","event":"hr_update","payload":{"hr":75}}"#,
            "abc",
            1000,
        )
        .unwrap()
        .unwrap();
        assert_eq!(m.timestamp, 1000);
        assert_eq!(m.heart_rate, 75);
        assert_eq!(m.source_timestamp, None);
        assert!(m.rr_ms.is_empty());
    }
    #[test]
    fn ignores_other_channels_and_system_messages() {
        for text in [
            r#"{"topic":"hr:other","event":"hr_update","payload":{"hr":75}}"#,
            r#"{"event":"pong"}"#,
            r#"{"event":"phx_reply","payload":{"status":"ok"}}"#,
        ] {
            assert!(parse_message(text, "abc", 1000).unwrap().is_none());
        }
    }
    #[test]
    fn rejects_invalid_bpm_and_ids() {
        for hr in ["0", "301", "-1", "75.5", "null"] {
            assert!(parse_message(
                &format!(r#"{{"topic":"hr:abc","event":"hr_update","payload":{{"hr":{hr}}}}}"#),
                "abc",
                1000
            )
            .is_err());
        }
        assert!(HypeRateAdapter::new("a/b".into(), "key".into()).is_err());
        assert!(HypeRateAdapter::new("abc".into(), "".into()).is_err());
    }
    #[test]
    fn credentials_are_url_encoded() {
        let adapter = HypeRateAdapter::new("abc".into(), "key&secret=1".into()).unwrap();
        assert_eq!(adapter.url().path(), "/ws/abc");
        assert_eq!(
            adapter.url().query_pairs().collect::<Vec<_>>(),
            vec![("token".into(), "key&secret=1".into())]
        );
    }
    #[tokio::test]
    async fn joins_stream_and_cancels_without_faking_data() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url::Url::parse(&format!("ws://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            let join = ws.next().await.unwrap().unwrap().into_text().unwrap();
            let join: Value = serde_json::from_str(&join).unwrap();
            assert_eq!(join["topic"], "hr:abc");
            assert_eq!(join["event"], "phx_join");
            for text in [
                r#"{"event":"phx_reply","ref":"nemuri-join","payload":{"status":"ok"}}"#,
                r#"{"topic":"hr:other","event":"hr_update","payload":{"hr":90}}"#,
                r#"{"topic":"hr:abc","event":"hr_update","payload":{"hr":75}}"#,
                r#"{"topic":"hr:abc","event":"hr_update","payload":{"hr":75}}"#,
            ] {
                ws.send(Message::Text(text.into())).await.unwrap();
            }
            let leave = ws.next().await.unwrap().unwrap().into_text().unwrap();
            assert!(leave.contains("phx_leave"));
            let _ = ws.close(None).await;
        });
        let (tx, mut rx) = mpsc::channel(8);
        let (stop, watch) = watch::channel(false);
        let adapter = HypeRateAdapter::new("abc".into(), "key".into()).unwrap();
        let task = tokio::spawn(async move { adapter.run_at(url, tx, watch).await });
        assert!(matches!(rx.recv().await, Some(SensorMessage::Connected(_))));
        for _ in 0..2 {
            match rx.recv().await.unwrap() {
                SensorMessage::Measurement(m) => assert_eq!(m.heart_rate, 75),
                _ => panic!("expected HR"),
            }
        }
        stop.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        server.await.unwrap();
    }
}
