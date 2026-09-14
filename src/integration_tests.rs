use crate::{
    engine::{self, Action, Command, Handle},
    model::*,
};
use ffmpeg_next as av;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

struct Running {
    handle: Handle,
    worker: Option<thread::JoinHandle<anyhow::Result<()>>>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.handle.shutdown.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("worker panic")
                .expect("engine failure");
        }
    }
}
fn control(handle: &Handle, action: Action) -> serde_json::Value {
    let (reply, wait) = tokio::sync::oneshot::channel();
    handle
        .commands
        .send(Command {
            id: uuid::Uuid::new_v4().to_string(),
            action,
            reply,
        })
        .unwrap();
    wait.blocking_recv().unwrap().unwrap()
}
fn until(handle: &Handle, predicate: impl Fn(&State) -> bool) -> State {
    let start = Instant::now();
    loop {
        let state = handle.state.lock().unwrap().clone();
        assert!(
            start.elapsed() < Duration::from_secs(12),
            "timeout: {}",
            serde_json::to_string(&state).unwrap()
        );
        if predicate(&state) {
            return state;
        }
        thread::sleep(Duration::from_millis(30));
    }
}

#[test]
fn native_channel_survives_live_edits_and_encodes_banner_audio_and_captions() {
    av::init().unwrap();
    av::log::set_level(av::log::Level::Error);
    let dir = tempfile::tempdir().unwrap();
    crate::demo::generate(dir.path()).unwrap();
    let mut assets = BTreeMap::new();
    for n in 1..=3 {
        let id = format!("demo-{n}");
        let asset = crate::source::probe(
            &dir.path().join(format!("media/{id}.mp4")),
            id.clone(),
            id.clone(),
        )
        .unwrap();
        assets.insert(id, asset);
    }
    let (commands, rx) = mpsc::sync_channel(128);
    let (events, _) = tokio::sync::broadcast::channel(16);
    let handle = Handle {
        state: Arc::new(Mutex::new(State::new("test".into(), assets))),
        commands,
        events,
        shutdown: Arc::new(AtomicBool::new(false)),
    };
    let worker_handle = handle.clone();
    let root = dir.path().to_path_buf();
    let running = Running {
        handle: handle.clone(),
        worker: Some(thread::spawn(move || engine::run(worker_handle, rx, root))),
    };
    until(&handle, |s| s.status == "live");
    let a = control(
        &handle,
        Action::Enqueue {
            asset_id: "demo-1".into(),
            after_id: None,
        },
    )["result"]["item_id"]
        .as_str()
        .unwrap()
        .to_string();
    until(&handle, |s| {
        s.current.as_ref().is_some_and(|c| c.item.id == a)
    });
    control(
        &handle,
        Action::Captions(
            a.clone(),
            vec![Cue {
                start_ms: 1000,
                end_ms: 6000,
                text: "Caption across live source switch".into(),
            }],
        ),
    );
    let b = control(
        &handle,
        Action::Enqueue {
            asset_id: "demo-2".into(),
            after_id: None,
        },
    )["result"]["item_id"]
        .as_str()
        .unwrap()
        .to_string();
    let c = control(
        &handle,
        Action::Enqueue {
            asset_id: "demo-3".into(),
            after_id: Some(a.clone()),
        },
    )["result"]["item_id"]
        .as_str()
        .unwrap()
        .to_string();
    let state = until(&handle, |s| s.queue.len() == 2);
    assert_eq!(
        state.queue[0].id, c,
        "insert after current must precede existing next item"
    );
    control(&handle, Action::Reorder(vec![b.clone(), c.clone()]));
    control(&handle, Action::Remove(b));
    let banner = control(
        &handle,
        Action::Banner(
            BannerRequest {
                duration_ms: 2000,
                title: "TEST AD".into(),
                asset_id: None,
            },
            None,
        ),
    );
    let banner_start = banner["program_ms"].as_u64().unwrap();
    until(&handle, |s| s.program_ms >= banner_start + 2500);
    assert!(handle.state.lock().unwrap().banner.is_none());
    let take = control(&handle, Action::Take(Some(c.clone())));
    let taken = until(&handle, |s| {
        s.current.as_ref().is_some_and(|i| i.item.id == c)
    });
    let event = taken
        .events
        .iter()
        .find(|e| {
            e.kind == "source_taken" && e.command_id.as_deref() == take["command_id"].as_str()
        })
        .unwrap();
    let switch_ms = event.program_ms;
    until(&handle, |s| s.program_ms >= 8000);
    drop(running);
    let root = dir.path().join("hls/test");
    let playlist = std::fs::read_to_string(root.join("media.m3u8")).unwrap();
    assert!(!playlist.contains("#EXT-X-DISCONTINUITY"));
    assert!(playlist.contains("#EXT-X-ENDLIST"));
    let mut input = av::format::input(&root.join("media.m3u8")).unwrap();
    let vs = input.streams().best(av::media::Type::Video).unwrap();
    let vi = vs.index();
    let tb = f64::from(vs.time_base());
    let mut video = av::codec::context::Context::from_parameters(vs.parameters())
        .unwrap()
        .decoder()
        .video()
        .unwrap();
    let audio_stream = input.streams().best(av::media::Type::Audio).unwrap();
    let ai = audio_stream.index();
    let mut audio = av::codec::context::Context::from_parameters(audio_stream.parameters())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    let mut video_samples = Vec::new();
    let mut audio_energy = 0.0_f64;
    let mut audio_samples = 0;
    for (stream, packet) in input.packets() {
        if stream.index() == vi {
            video.send_packet(&packet).unwrap();
            let mut frame = av::frame::Video::empty();
            while video.receive_frame(&mut frame).is_ok() {
                let t = frame.timestamp().unwrap() as f64 * tb * 1000.0;
                let y = frame.data(0)[700 * frame.stride(0) + 1250];
                video_samples.push((t, y));
            }
        } else if stream.index() == ai {
            audio.send_packet(&packet).unwrap();
            let mut frame = av::frame::Audio::empty();
            while audio.receive_frame(&mut frame).is_ok() {
                for sample in frame.plane::<f32>(0) {
                    audio_energy += (*sample as f64).powi(2);
                    audio_samples += 1;
                }
            }
        }
    }
    assert!(
        video_samples.len() >= 235,
        "missing video frames: {}",
        video_samples.len()
    );
    let offset = video_samples[0].0;
    assert!(
        (offset - 1024.0 / 48000.0 * 1000.0).abs() < 1.0,
        "unexpected mux origin {offset}"
    );
    assert!(
        video_samples
            .windows(2)
            .all(|pair| ((pair[1].0 - pair[0].0) - 1000.0 / 30.0).abs() < 0.1),
        "timestamps must remain continuous across takes"
    );
    let during = video_samples
        .iter()
        .find(|(t, _)| *t - offset > banner_start as f64 + 600.0)
        .unwrap()
        .1;
    assert!(
        (during as i16 - 66).abs() < 5,
        "banner pixels missing: {during}"
    );
    let after = video_samples
        .iter()
        .find(|(t, _)| *t - offset > switch_ms as f64 + 700.0)
        .unwrap()
        .1;
    assert!(
        (after as i16 - 126).abs() < 5,
        "new source pixels missing after restoration: {after}"
    );
    assert!(
        audio_samples > 300_000 && audio_energy / audio_samples as f64 > 0.00005,
        "source audio missing"
    );
    let vtt = std::fs::read_to_string(root.join("caption-1.vtt")).unwrap();
    assert!(vtt.contains("Caption across live source switch"));
    assert!(vtt.contains("MPEGTS:1920"));
}
