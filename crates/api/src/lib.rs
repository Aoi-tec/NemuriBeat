use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
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
    Router::new()
        .route("/api/v1/physiology/latest", get(latest))
        .route("/api/v1/physiology/history", get(history))
        .route("/api/v1/sleep/status", get(sleep_status))
        .route("/api/v1/sleep/sessions", get(sessions))
        .route("/api/v1/sleep/sessions/{id}", get(session))
        .route("/api/v1/settings", get(settings))
        .with_state(state)
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
pub async fn serve(state: ApiState, port: u16) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}

async fn sleep_status(State(state): State<ApiState>) -> Response {
    let s = state.core.latest();
    Ok(Json(
        serde_json::json!({"asleep":s.sleep.tracking.then_some(s.sleep.sleep_like),"tracking":s.sleep.tracking,"probability":s.sleep.tracking.then_some(s.sleep.probability),"mode":s.mode,"estimated":s.mode==shared_types::MeasurementMode::HrOnly,"measurement_available":s.rr_available&&s.mode==shared_types::MeasurementMode::HrRr&&s.sleep.tracking,"updated_at":s.features.timestamp,"source_timestamp":s.source_timestamp}),
    ))
}
