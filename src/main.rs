mod api;
mod captions;
mod compositor;
mod demo;
mod engine;
#[cfg(test)]
mod integration_tests;
mod model;
mod output;
mod publish;
mod source;
mod web_assets;
use anyhow::Result;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rust_playout=info,tower_http=info".into()),
        )
        .init();
    if std::env::args().any(|v| v == "--version" || v == "-V") {
        println!("rust-playout {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    ffmpeg_next::init()?;
    ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
    let root = PathBuf::from(std::env::var("PLAYOUT_DATA").unwrap_or_else(|_| "data".into()));
    std::fs::create_dir_all(root.join("media"))?;
    let root = std::fs::canonicalize(root)?;
    if std::env::args().any(|v| v == "--demo" || v == "--make-demo") {
        demo::generate(&root)?;
    }
    if std::env::args().any(|v| v == "--make-demo") {
        return Ok(());
    }
    let mut assets = BTreeMap::new();
    for entry in std::fs::read_dir(root.join("media"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|v| v == "json")
            && let Ok(mut asset) =
                serde_json::from_str::<model::Asset>(&std::fs::read_to_string(&path)?)
        {
            asset.path = path.with_extension(if asset.kind == "video" { "mp4" } else { "img" });
            if asset.path.exists() {
                assets.insert(asset.id.clone(), asset);
            }
        }
    }
    let session = uuid::Uuid::new_v4().to_string();
    let (commands, receiver) = mpsc::sync_channel(128);
    let (events, _) = tokio::sync::broadcast::channel(16);
    let handle = engine::Handle {
        state: Arc::new(Mutex::new(model::State::new(session, assets))),
        commands,
        events,
        shutdown: Arc::new(AtomicBool::new(false)),
    };
    let worker_handle = handle.clone();
    let worker_root = root.clone();
    let worker = std::thread::Builder::new()
        .name("playout-render".into())
        .spawn(move || {
            if let Err(error) = engine::run(worker_handle.clone(), receiver, worker_root) {
                tracing::error!("Channel failed: {error:#}");
                let mut state = worker_handle.state.lock().unwrap();
                state.status = "failed".into();
                state.event("channel_failed", format!("{error:#}"), None);
                let _ = worker_handle.events.send(state.clone());
            }
        })?;
    let port: u16 = std::env::var("PLAYOUT_PORT")
        .unwrap_or_else(|_| "8787".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    tracing::info!("Control room: http://127.0.0.1:{port} (native FFmpeg libraries; no CLI)");
    let shutdown = handle.shutdown.clone();
    axum::serve(
        listener,
        api::router(api::App {
            engine: handle.clone(),
            root,
        }),
    )
    .with_graceful_shutdown(async move {
        let _ = tokio::signal::ctrl_c().await;
        shutdown.store(true, Ordering::Relaxed);
    })
    .await?;
    handle.shutdown.store(true, Ordering::Relaxed);
    tokio::task::spawn_blocking(move || worker.join())
        .await?
        .map_err(|_| anyhow::anyhow!("render thread panicked"))?;
    Ok(())
}
