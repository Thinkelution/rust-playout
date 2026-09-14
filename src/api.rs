use crate::{
    engine::{Action, Command, Handle},
    model::{Asset, BannerRequest, Cue},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State as Extract, WebSocketUpgrade, ws::Message},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};
use tokio::io::AsyncWriteExt;
use tower_http::services::ServeDir;

#[derive(Clone)]
pub struct App {
    pub engine: Handle,
    pub root: PathBuf,
}
type ApiError = (StatusCode, Json<serde_json::Value>);
fn error(status: StatusCode, text: impl ToString) -> ApiError {
    (status, Json(serde_json::json!({"error":text.to_string()})))
}
fn bad(text: impl ToString) -> ApiError {
    error(StatusCode::BAD_REQUEST, text)
}

async fn command(app: &App, action: Action) -> Result<Json<serde_json::Value>, ApiError> {
    let (reply, wait) = tokio::sync::oneshot::channel();
    let id = uuid::Uuid::new_v4().to_string();
    app.engine
        .commands
        .try_send(Command {
            id: id.clone(),
            action,
            reply,
        })
        .map_err(|_| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "engine command queue is full or unavailable",
            )
        })?;
    match tokio::time::timeout(Duration::from_secs(5), wait).await {
        Ok(Ok(Ok(value))) => Ok(Json(value)),
        Ok(Ok(Err(e))) => Err(bad(e)),
        Ok(Err(_)) => Err(error(StatusCode::SERVICE_UNAVAILABLE, "engine stopped")),
        Err(_) => Err(error(
            StatusCode::GATEWAY_TIMEOUT,
            format!("command {id} outcome is uncertain; inspect events before retrying"),
        )),
    }
}

pub fn router(app: App) -> Router {
    let root = app.root.clone();
    Router::new()
        .route("/api/state", get(state))
        .route("/api/publish", post(start_publish).delete(stop_publish))
        .route("/api/events", get(events))
        .route("/api/assets", post(upload))
        .route("/api/queue", post(enqueue))
        .route("/api/queue/order", put(reorder))
        .route("/api/queue/{id}", delete(remove))
        .route("/api/queue/{id}/captions", put(captions))
        .route("/api/take", post(take))
        .route("/api/clear", post(clear))
        .route("/api/volume", put(volume))
        .route("/api/banner", post(banner).delete(remove_banner))
        .nest_service("/hls", ServeDir::new(root.join("hls")))
        .nest_service("/media", ServeDir::new(root.join("media")))
        .fallback_service(ServeDir::new("web/dist"))
        .layer(DefaultBodyLimit::max(256 * 1024 * 1024))
        .layer(axum::middleware::from_fn(same_origin))
        .with_state(app)
}

async fn same_origin(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let hostname = host.split(':').next().unwrap_or("");
    if !matches!(hostname, "127.0.0.1" | "localhost") {
        return error(
            StatusCode::FORBIDDEN,
            "local control requires a loopback host",
        )
        .into_response();
    }
    if let Some(origin) = request.headers().get("origin") {
        let host = request
            .headers()
            .get("host")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let allowed = origin
            .to_str()
            .is_ok_and(|o| o == format!("http://{host}") || o == format!("https://{host}"));
        if !allowed {
            return error(StatusCode::FORBIDDEN, "cross-origin control is disabled")
                .into_response();
        }
    }
    let hls = request.uri().path().starts_with("/hls/");
    let mut response = next.run(request).await;
    if hls {
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
    }
    response
}

async fn state(Extract(app): Extract<App>) -> Json<crate::model::State> {
    Json(app.engine.state.lock().unwrap().clone())
}
async fn events(Extract(app): Extract<App>, ws: WebSocketUpgrade) -> impl IntoResponse {
    let mut updates = app.engine.events.subscribe();
    ws.on_upgrade(move |mut socket| async move {
        let mut heartbeat = tokio::time::interval(Duration::from_millis(500));
        let initial = app.engine.state.lock().unwrap().clone();
        if socket.send(Message::Text(serde_json::to_string(&initial).unwrap().into())).await.is_err() { return; }
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if app.engine.shutdown.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                }
                state = updates.recv() => {
                    let state = match state { Ok(s) => s, Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => app.engine.state.lock().unwrap().clone(), Err(_) => break };
                    if socket.send(Message::Text(serde_json::to_string(&state).unwrap().into())).await.is_err() { break; }
                }
                incoming = socket.recv() => { if matches!(incoming, None | Some(Err(_)) | Some(Ok(Message::Close(_)))) { break; } }
            }
        }
    })
}

#[derive(Deserialize)]
struct Enqueue {
    asset_id: String,
    after_id: Option<String>,
}
async fn enqueue(
    Extract(app): Extract<App>,
    Json(v): Json<Enqueue>,
) -> Result<Json<serde_json::Value>, ApiError> {
    command(
        &app,
        Action::Enqueue {
            asset_id: v.asset_id,
            after_id: v.after_id,
        },
    )
    .await
}
#[derive(Deserialize)]
struct Order {
    ids: Vec<String>,
}
async fn reorder(
    Extract(app): Extract<App>,
    Json(v): Json<Order>,
) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::Reorder(v.ids)).await
}
async fn remove(
    Extract(app): Extract<App>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::Remove(id)).await
}
#[derive(Deserialize)]
struct Take {
    item_id: Option<String>,
}
async fn take(
    Extract(app): Extract<App>,
    Json(v): Json<Take>,
) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::Take(v.item_id)).await
}
async fn clear(Extract(app): Extract<App>) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::Clear).await
}
#[derive(Deserialize)]
struct Volume {
    value: f32,
}
async fn volume(
    Extract(app): Extract<App>,
    Json(v): Json<Volume>,
) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::Volume(v.value)).await
}
async fn remove_banner(Extract(app): Extract<App>) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::RemoveBanner).await
}
async fn captions(
    Extract(app): Extract<App>,
    Path(id): Path<String>,
    Json(cues): Json<Vec<Cue>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::Captions(id, cues)).await
}
async fn banner(
    Extract(app): Extract<App>,
    Json(v): Json<BannerRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    v.validate().map_err(bad)?;
    let path = if let Some(id) = &v.asset_id {
        let state = app.engine.state.lock().unwrap();
        let asset = state
            .assets
            .get(id)
            .filter(|a| a.kind == "image")
            .ok_or_else(|| bad("unknown banner artwork"))?;
        Some(asset.path.clone())
    } else {
        None
    };
    let artwork = if let Some(path) = path {
        Some(
            tokio::task::spawn_blocking(move || crate::compositor::artwork(&path))
                .await
                .map_err(bad)?
                .map_err(bad)?,
        )
    } else {
        None
    };
    command(&app, Action::Banner(v, artwork)).await
}

pub fn save_asset(root: &std::path::Path, asset: &Asset) -> anyhow::Result<()> {
    crate::captions::atomic_write(
        &root.join("media").join(format!("{}.json", asset.id)),
        &serde_json::to_string_pretty(asset)?,
    )
}

async fn upload(
    Extract(app): Extract<App>,
    mut multipart: Multipart,
) -> Result<Json<Asset>, ApiError> {
    let mut field = multipart
        .next_field()
        .await
        .map_err(bad)?
        .ok_or_else(|| bad("upload a media file"))?;
    let name = field
        .file_name()
        .unwrap_or("upload")
        .chars()
        .take(180)
        .collect::<String>();
    let extension = std::path::Path::new(&name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["mp4", "png", "jpg", "jpeg"].contains(&extension.as_str()) {
        return Err(bad("upload an MP4 video or PNG/JPEG banner"));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let stored_extension = if extension == "mp4" { "mp4" } else { "img" };
    let path = app
        .root
        .join("media")
        .join(format!("{id}.{stored_extension}"));
    let result = async {
        let mut file = tokio::fs::File::create(&path).await.map_err(bad)?;
        let mut size = 0;
        while let Some(chunk) = field.chunk().await.map_err(bad)? {
            size += chunk.len();
            if size > 250 * 1024 * 1024 {
                return Err(bad("file limit is 250 MB"));
            }
            file.write_all(&chunk).await.map_err(bad)?;
        }
        file.flush().await.map_err(bad)?;
        drop(file);
        let local_path = path.clone();
        let asset = tokio::task::spawn_blocking(move || -> anyhow::Result<Asset> {
            if stored_extension == "mp4" {
                crate::source::probe(&local_path, id, name)
            } else {
                let reader = image::ImageReader::open(&local_path)?.with_guessed_format()?;
                let (width, height) = reader.into_dimensions()?;
                anyhow::ensure!(
                    width <= 4096 && height <= 4096,
                    "artwork limit is 4096x4096"
                );
                crate::compositor::artwork(&local_path)?;
                Ok(Asset {
                    id,
                    name,
                    kind: "image".into(),
                    duration_ms: 0,
                    width,
                    height,
                    path: local_path,
                })
            }
        })
        .await
        .map_err(bad)?
        .map_err(bad)?;
        save_asset(&app.root, &asset).map_err(bad)?;
        let _ = command(&app, Action::Register(asset.clone())).await?;
        Ok(Json(asset))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_file(path.with_extension("json")).await;
    }
    result
}

async fn start_publish(
    Extract(app): Extract<App>,
    Json(request): Json<crate::publish::PublishRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::StartPublish(request)).await
}
async fn stop_publish(Extract(app): Extract<App>) -> Result<Json<serde_json::Value>, ApiError> {
    command(&app, Action::StopPublish).await
}
