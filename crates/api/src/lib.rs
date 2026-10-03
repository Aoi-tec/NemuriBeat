use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use physio_core::RealtimeCore;
use std::{collections::HashMap, path::PathBuf};
#[derive(Clone)]
pub struct ApiState {
    pub core: RealtimeCore,
    pub database: PathBuf,
}
type Response = Result<Json<serde_json::Value>, (StatusCode, String)>;
fn err(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
pub fn router(state: ApiState) -> Router {
    router_with_options(state, true, true)
}
pub fn router_with_options(state: ApiState, full_api: bool, overlay: bool) -> Router {
    let mut app = Router::new();
    if full_api {
        app = app
            .route("/api/v1/physiology/latest", get(latest))
            .route("/api/v1/physiology/history", get(history))
            .route("/api/v1/sleep/status", get(sleep_status))
            .route("/api/v1/sleep/sessions", get(sessions))
            .route("/api/v1/sleep/sessions/{id}", get(session))
            .route("/api/v1/sleep/days", get(days))
            .route("/api/v1/sleep/days/{date}", get(day))
            .route("/api/v1/settings", get(settings));
    }
    if overlay {
        app = app
            .route("/overlay/heart-rate", get(overlay_page))
            .route("/overlay/heart-rate.js", get(overlay_script))
            .route("/overlay/heart-rate.css", get(overlay_style))
            .route("/overlay/icon.png", get(overlay_icon))
            .route("/api/v1/overlay/status", get(overlay_status));
    }
    app.with_state(state)
}
async fn overlay_page() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Html(include_str!("../assets/heart-rate.html")),
    )
}
async fn overlay_script() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/heart-rate.js"),
    )
}
async fn overlay_style() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/heart-rate.css"),
    )
}
async fn overlay_icon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/png")],
        include_bytes!("../../../apps/desktop/assets/branding/nemuribeat-appicon-64.png")
            .as_slice(),
    )
}
async fn overlay_status(State(state): State<ApiState>) -> impl IntoResponse {
    let s = state.core.latest();
    let age = (shared_types::now_ms() - s.timestamp).max(0);
    let available = s.sensor_connected && s.heart_rate.is_some() && age <= 5000;
    let source = if s
        .sensor_id
        .as_deref()
        .is_some_and(|id| id.starts_with("HypeRate"))
    {
        "HypeRate"
    } else if s
        .sensor_id
        .as_deref()
        .is_some_and(|id| id.starts_with("DEMO"))
    {
        "DEMO"
    } else {
        "BLE"
    };
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(
            serde_json::json!({"available":available,"heart_rate":s.heart_rate.filter(|_|available),"mode":s.mode,"source":source,"demo":source=="DEMO","asleep":(available && s.sleep.tracking).then_some(s.sleep.sleep_like),"estimated":s.mode==shared_types::MeasurementMode::HrOnly,"updated_at":s.timestamp,"age_ms":age}),
        ),
    )
}
async fn latest(State(s): State<ApiState>) -> Response {
    Ok(Json(serde_json::to_value(s.core.latest()).map_err(err)?))
}
async fn history(State(s): State<ApiState>, Query(q): Query<HashMap<String, String>>) -> Response {
    let since = q
        .get("since")
        .map(|v| v.parse::<i64>())
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "since must be Unix milliseconds".into(),
            )
        })?
        .unwrap_or(shared_types::now_ms() - 86400000);
    let data = tokio::task::spawn_blocking(move || storage::history(&s.database, since, None))
        .await
        .map_err(err)?
        .map_err(err)?;
    Ok(Json(serde_json::to_value(data).map_err(err)?))
}
async fn sessions(State(s): State<ApiState>) -> Response {
    let data = tokio::task::spawn_blocking(move || storage::sessions(&s.database, 0))
        .await
        .map_err(err)?
        .map_err(err)?;
    Ok(Json(serde_json::to_value(data).map_err(err)?))
}
async fn session(State(s): State<ApiState>, Path(id): Path<String>) -> Response {
    let data = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let found = storage::sessions(&s.database, 0)?
            .into_iter()
            .find(|x| x.session_id == id);
        let minutes = storage::history(&s.database, 0, Some(&id))?;
        Ok((found, minutes))
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    if data.0.is_none() {
        return Err((StatusCode::NOT_FOUND, "Unknown session".into()));
    }
    Ok(Json(serde_json::json!({"session":data.0,"minutes":data.1})))
}
async fn settings(State(s): State<ApiState>) -> Response {
    let data = tokio::task::spawn_blocking(move || storage::settings(&s.database))
        .await
        .map_err(err)?
        .map_err(err)?;
    Ok(Json(serde_json::to_value(data).map_err(err)?))
}
async fn days(State(s): State<ApiState>) -> Response {
    let data = tokio::task::spawn_blocking(move || storage::sleep_days(&s.database))
        .await
        .map_err(err)?
        .map_err(err)?;
    Ok(Json(serde_json::to_value(data).map_err(err)?))
}
async fn day(State(s): State<ApiState>, Path(date): Path<String>) -> Response {
    let data = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let found = storage::sleep_days(&s.database)?
            .into_iter()
            .find(|d| d.sleep_day == date);
        let minutes = if let Some(d) = &found {
            storage::day_history(&s.database, d)?
        } else {
            vec![]
        };
        Ok((found, minutes))
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    if data.0.is_none() {
        return Err((StatusCode::NOT_FOUND, "Unknown sleep day".into()));
    }
    Ok(Json(serde_json::json!({"day":data.0,"minutes":data.1})))
}
pub async fn serve(state: ApiState, port: u16) -> anyhow::Result<()> {
    serve_with_options(state, port, true, true).await
}
pub async fn serve_with_options(
    state: ApiState,
    port: u16,
    full_api: bool,
    overlay: bool,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    axum::serve(listener, router_with_options(state, full_api, overlay)).await?;
    Ok(())
}

async fn sleep_status(State(state): State<ApiState>) -> Response {
    let s = state.core.latest();
    Ok(Json(
        serde_json::json!({"asleep":s.sleep.tracking.then_some(s.sleep.sleep_like),"tracking":s.sleep.tracking,"probability":s.sleep.tracking.then_some(s.sleep.probability),"mode":s.mode,"estimated":s.mode==shared_types::MeasurementMode::HrOnly,"measurement_available":s.rr_available&&s.mode==shared_types::MeasurementMode::HrRr&&s.sleep.tracking,"updated_at":s.features.timestamp,"source_timestamp":s.source_timestamp,"model":s.sleep_model,"hmd":s.hmd}),
    ))
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tower::ServiceExt;
    fn state() -> ApiState {
        ApiState {
            core: RealtimeCore::new(),
            database: "unused.sqlite".into(),
        }
    }
    #[tokio::test]
    async fn overlay_runs_without_exposing_full_api() {
        let app = router_with_options(state(), false, true);
        for path in [
            "/overlay/heart-rate",
            "/overlay/heart-rate.js",
            "/overlay/heart-rate.css",
            "/overlay/icon.png",
            "/api/v1/overlay/status",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    #[tokio::test]
    async fn stale_and_disconnected_pulse_is_null() {
        for age in [0, 6000] {
            let state = state();
            state.core.ingest(shared_types::Measurement {
                timestamp: shared_types::now_ms() - age,
                source_timestamp: None,
                sensor_id: "HypeRate".into(),
                heart_rate: 75,
                rr_ms: vec![],
                contact: None,
            });
            let app = router_with_options(state, false, true);
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/api/v1/overlay/status")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body = to_bytes(response.into_body(), 16384).await.unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(json["available"], age == 0);
            assert_eq!(
                json["heart_rate"],
                if age == 0 {
                    serde_json::json!(75)
                } else {
                    serde_json::Value::Null
                }
            );
            assert!(json.get("api_key").is_none());
            assert_eq!(json["source"], "HypeRate");
        }
        let state = state();
        state.core.disconnected();
        let app = router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/overlay/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
        assert!(json["heart_rate"].is_null());
        assert!(json["asleep"].is_null());
    }
    #[tokio::test]
    async fn disabling_overlay_removes_its_routes() {
        let response = router_with_options(state(), true, false)
            .oneshot(
                Request::builder()
                    .uri("/overlay/heart-rate")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
