//! Public-client OAuth: device codes and bearer tokens never enter settings or logs.
use anyhow::{bail, Result};
use async_trait::async_trait;
use sensor_core::{Device, SensorAdapter, SensorMessage};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
pub const CLIENT_ID: &str = "5131ba31-97fa-4587-b9d8-e155640f343e";
const AUTH_URL: &str = "https://pulsoid.net/oauth2/device_authorization";
const TOKEN_URL: &str = "https://pulsoid.net/oauth2/token";
#[derive(Clone)]
pub struct Authorization {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: i64,
}
#[derive(Deserialize)]
struct DeviceCode {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
    token_type: String,
}
#[derive(Deserialize)]
struct OAuthError {
    error: String,
}
#[derive(Debug, PartialEq)]
enum Poll {
    Pending,
    SlowDown,
    Expired,
    Denied,
}
fn poll_error(error: &str) -> Poll {
    match error {
        "authorization_pending" => Poll::Pending,
        "slow_down" => Poll::SlowDown,
        "expired_token" => Poll::Expired,
        _ => Poll::Denied,
    }
}
pub struct DeviceOAuthAdapter {
    pub updates: mpsc::Sender<Option<Authorization>>,
}
impl DeviceOAuthAdapter {
    async fn authorize(&self) -> Result<String> {
        self.authorize_at(AUTH_URL, TOKEN_URL).await
    }
    async fn authorize_at(&self, auth_url: &str, token_url: &str) -> Result<String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let response = client
            .post(auth_url)
            .form(&[("client_id", CLIENT_ID), ("scope", "data:heart_rate:read")])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Pulsoid認証サーバーに接続できません"))?;
        if !response.status().is_success() {
            bail!("Pulsoid認証を開始できません（HTTP {}）。クライアントIDとDevice Flow設定を確認してください", response.status().as_u16());
        }
        let code: DeviceCode = response
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("Pulsoid認証応答が不正です"))?;
        let uri = url::Url::parse(&code.verification_uri_complete)?;
        anyhow::ensure!(
            uri.scheme() == "https"
                && uri.host_str() == Some("pulsoid.net")
                && uri.username().is_empty()
                && uri.password().is_none()
                && uri.port_or_known_default() == Some(443),
            "Pulsoid認証URLが不正です"
        );
        anyhow::ensure!(
            !code.device_code.is_empty()
                && code.expires_in > 0
                && code.expires_in <= 3600
                && code.interval <= 60,
            "Pulsoid認証応答が不正です"
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
        self.updates
            .send(Some(Authorization {
                user_code: code.user_code,
                verification_uri: code.verification_uri_complete,
                expires_at: shared_types::now_ms() + code.expires_in as i64 * 1000,
            }))
            .await?;
        let mut interval = code.interval.max(1);
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            if tokio::time::Instant::now() >= deadline {
                bail!("Pulsoid認証の有効期限が切れました。再度認証してください");
            }
            let response = match client
                .post(token_url)
                .form(&[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", &code.device_code),
                    ("client_id", CLIENT_ID),
                ])
                .send()
                .await
            {
                Ok(r) => r,
                Err(_) => {
                    interval = (interval + 5).min(60);
                    continue;
                }
            };
            if response.status().is_success() {
                let token: Token = response
                    .json()
                    .await
                    .map_err(|_| anyhow::anyhow!("Pulsoidトークン応答が不正です"))?;
                anyhow::ensure!(
                    token.token_type.eq_ignore_ascii_case("bearer")
                        && !token.access_token.is_empty(),
                    "Pulsoidトークン応答が不正です"
                );
                return Ok(token.access_token);
            }
            if response.status().is_server_error() || response.status().as_u16() == 429 {
                interval = (interval + 5).min(60);
                continue;
            }
            let error: OAuthError = response
                .json()
                .await
                .map_err(|_| anyhow::anyhow!("Pulsoid認証に失敗しました"))?;
            match poll_error(&error.error) {
                Poll::Pending => {}
                Poll::SlowDown => interval = (interval + 5).min(60),
                Poll::Expired => bail!("Pulsoid認証の有効期限が切れました"),
                Poll::Denied => bail!("Pulsoid認証が拒否されました。再度認証してください"),
            }
        }
    }
}
#[async_trait]
impl SensorAdapter for DeviceOAuthAdapter {
    async fn scan(&self) -> Result<Vec<Device>> {
        Ok(vec![])
    }
    async fn run(
        &self,
        id: String,
        output: mpsc::Sender<SensorMessage>,
        mut stop: watch::Receiver<bool>,
    ) -> Result<()> {
        if *stop.borrow() {
            return Ok(());
        }
        let result = tokio::select! { _=stop.changed()=>Ok(None), result=self.authorize()=>result.map(Some) };
        let _ = self.updates.send(None).await;
        if let Some(token) = result? {
            super::PulsoidAdapter::new(token)?
                .run(id, output, stop)
                .await?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn polling_errors() {
        assert_eq!(poll_error("authorization_pending"), Poll::Pending);
        assert_eq!(poll_error("slow_down"), Poll::SlowDown);
        assert_eq!(poll_error("expired_token"), Poll::Expired);
        assert_eq!(poll_error("invalid_grant"), Poll::Denied);
        assert_eq!(poll_error("unexpected"), Poll::Denied);
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn server(
        responses: Vec<(u16, String)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let mut requests = vec![];
            for (status, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![];
                loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                        let len = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        if body.len() >= len {
                            break;
                        }
                    }
                }
                requests.push(String::from_utf8(request).unwrap());
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        (url, task)
    }
    fn code(expires: u64) -> String {
        format!(
            r#"{{"device_code":"private-device","user_code":"user-code","verification_uri_complete":"https://pulsoid.net/oauth2/device_authorization_consent?user_code=user-code","expires_in":{expires},"interval":1}}"#
        )
    }
    #[tokio::test]
    async fn sends_client_id_scope_and_device_grant_then_gets_token() {
        let (url, server) = server(vec![
            (200, code(60)),
            (400, r#"{"error":"authorization_pending"}"#.into()),
            (
                200,
                r#"{"access_token":"private-token","token_type":"bearer"}"#.into(),
            ),
        ])
        .await;
        let (updates, mut rx) = mpsc::channel(4);
        let adapter = DeviceOAuthAdapter { updates };
        assert_eq!(
            adapter.authorize_at(&url, &url).await.unwrap(),
            "private-token"
        );
        let auth = rx.recv().await.unwrap().unwrap();
        assert_eq!(auth.user_code, "user-code");
        assert!(auth.verification_uri.starts_with("https://pulsoid.net/"));
        let requests = server.await.unwrap();
        assert!(requests[0].contains(CLIENT_ID));
        assert!(requests[0].contains("scope=data%3Aheart_rate%3Aread"));
        assert!(requests[1]
            .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
        assert!(requests[1].contains("device_code=private-device"));
        assert!(requests[1].contains(CLIENT_ID));
    }
    #[tokio::test]
    async fn rejected_and_expired_consent_never_return_a_token() {
        for error in ["invalid_grant", "expired_token"] {
            let (url, server) = server(vec![
                (200, code(60)),
                (400, format!(r#"{{"error":"{error}"}}"#)),
            ])
            .await;
            let (updates, _rx) = mpsc::channel(4);
            assert!(DeviceOAuthAdapter { updates }
                .authorize_at(&url, &url)
                .await
                .is_err());
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn local_deadline_stops_polling() {
        let (url, server) = server(vec![(200, code(1))]).await;
        let (updates, _rx) = mpsc::channel(4);
        let error = DeviceOAuthAdapter { updates }
            .authorize_at(&url, &url)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("有効期限"));
        assert_eq!(server.await.unwrap().len(), 1);
    }
}
