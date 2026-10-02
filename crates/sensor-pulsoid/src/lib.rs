//! Pulsoid's documented push stream. Credentials stay in memory and errors never contain URLs.
pub mod oauth;
use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use sensor_core::{Device, SensorAdapter, SensorMessage};
use serde::Deserialize;
use shared_types::{now_ms, Measurement};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};
#[derive(Deserialize)]
struct Frame {
    measured_at: i64,
    data: HeartRate,
}
#[derive(Deserialize)]
struct HeartRate {
    heart_rate: u16,
}
pub fn parse_message(text: &str, received_at: i64) -> Result<Measurement> {
    let frame: Frame = serde_json::from_str(text)
        .map_err(|_| anyhow::anyhow!("Pulsoid: malformed heart-rate message"))?;
    anyhow::ensure!(
        frame.measured_at > 0 && (1..=300).contains(&frame.data.heart_rate),
        "Pulsoid: invalid timestamp or BPM"
    );
    Ok(Measurement {
        timestamp: received_at,
        source_timestamp: Some(frame.measured_at),
        sensor_id: "Pulsoid".into(),
        heart_rate: frame.data.heart_rate,
        rr_ms: vec![],
        contact: None,
    })
}
pub struct PulsoidAdapter {
    token: String,
}
impl PulsoidAdapter {
    pub fn new(token: String) -> Result<Self> {
        anyhow::ensure!(
            !token.trim().is_empty(),
            "Pulsoidのアクセストークンを入力してください"
        );
        Ok(Self {
            token: token.trim().into(),
        })
    }
}
#[async_trait]
impl SensorAdapter for PulsoidAdapter {
    async fn scan(&self) -> Result<Vec<Device>> {
        Ok(vec![Device {
            id: "Pulsoid".into(),
            name: "Pulsoid · HR only".into(),
        }])
    }
    async fn run(
        &self,
        _id: String,
        output: mpsc::Sender<SensorMessage>,
        mut stop: watch::Receiver<bool>,
    ) -> Result<()> {
        let mut url = url::Url::parse("wss://dev.pulsoid.net/api/v1/data/real_time").unwrap();
        url.query_pairs_mut()
            .append_pair("access_token", &self.token);
        let mut delay = 1u64;
        let mut previous_source = 0;
        loop {
            if *stop.borrow() {
                return Ok(());
            }
            let connect = tokio_tungstenite::connect_async_with_config(
                url.as_str(),
                Some(WebSocketConfig::default().max_message_size(Some(65536))),
                false,
            );
            let result = tokio::select! {_=stop.changed()=>return Ok(()),r=tokio::time::timeout(Duration::from_secs(15),connect)=>r};
            let reason = match result {
                Ok(Ok((mut socket, _))) => {
                    // A connected websocket is not evidence of a live sensor; announce only on fresh HR.
                    let mut last_data = tokio::time::Instant::now();
                    let mut live = false;
                    loop {
                        tokio::select! {
                         _=stop.changed()=>{let _=tokio::time::timeout(Duration::from_secs(1),socket.close(None)).await;return Ok(());},
                         _=tokio::time::sleep_until(last_data+Duration::from_secs(30))=>break,
                         frame=socket.next()=>match frame {
                          Some(Ok(Message::Text(text)))=>{
                           if let Ok(m)=parse_message(&text,now_ms()) {
                            let source=m.source_timestamp.unwrap();
                            if source<=previous_source {continue;}
                            previous_source=source;last_data=tokio::time::Instant::now();delay=1;
                            if !live{output.send(SensorMessage::Connected("Pulsoid".into())).await?;live=true;}
                            output.send(SensorMessage::Measurement(m)).await?;
                           }
                          },
                          Some(Ok(Message::Ping(data)))=>{if socket.send(Message::Pong(data)).await.is_err(){break;}},
                          Some(Ok(Message::Close(_)))|Some(Err(_))|None=>break,
                          _=>{}
                         }
                        }
                    }
                    "Pulsoidの配信が途切れました".to_string()
                }
                Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response)))
                    if matches!(response.status().as_u16(), 401 | 403) =>
                {
                    return Err(anyhow::anyhow!(
                        "Pulsoid認証エラー: トークンとdata:heart_rate:read権限を確認してください"
                    ));
                }
                _ => "Pulsoidに接続できませんでした".to_string(),
            };
            output
                .send(SensorMessage::Disconnected(format!(
                    "{reason}（{delay}秒後に再接続）"
                )))
                .await?;
            tokio::select! {_=stop.changed()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(delay))=>{}}
            delay = (delay * 2).min(30);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_both_clocks_and_never_invents_rr() {
        let m = parse_message(
            r#"{"measured_at":1625310655000,"data":{"heart_rate":60}}"#,
            2000,
        )
        .unwrap();
        assert_eq!(m.timestamp, 2000);
        assert_eq!(m.source_timestamp, Some(1625310655000));
        assert!(m.rr_ms.is_empty());
        assert_eq!(m.heart_rate, 60);
    }
    #[test]
    fn malformed_and_zero_are_rejected() {
        for payload in [
            "{}",
            r#"{"measured_at":1,"data":{"heart_rate":0}}"#,
            r#"{"measured_at":0,"data":{"heart_rate":60}}"#,
        ] {
            assert!(parse_message(payload, 0).is_err());
        }
    }
}
