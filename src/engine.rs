use crate::{
    captions::Captions,
    compositor::{self, Compositor},
    model::*,
    output::Output,
    source::Source,
};
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender},
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, oneshot};

pub enum Action {
    StartChannel,
    StopChannel,
    Register(Asset),
    Enqueue {
        asset_id: String,
        after_id: Option<String>,
    },
    Remove(String),
    Reorder(Vec<String>),
    Take(Option<String>),
    Clear,
    Volume(f32),
    StartPublish(crate::publish::PublishRequest),
    StopPublish,
    Banner(BannerRequest, Option<ffmpeg_next::frame::Video>),
    RemoveBanner,
    Captions(String, Vec<Cue>),
}

pub struct Command {
    pub id: String,
    pub action: Action,
    pub reply: oneshot::Sender<Result<serde_json::Value, String>>,
}

#[derive(Clone)]
pub struct Handle {
    pub state: Arc<Mutex<State>>,
    pub commands: SyncSender<Command>,
    pub events: broadcast::Sender<State>,
    pub shutdown: Arc<AtomicBool>,
}

struct Active {
    item: QueueItem,
    source: Source,
    start: u64,
    duration_ms: u64,
}
struct Prepared {
    item: QueueItem,
    source: Source,
}

pub fn run(handle: Handle, commands: Receiver<Command>, root: PathBuf) -> Result<()> {
    let mut state = handle.state.lock().unwrap().clone();
    let output_dir = root.join("hls").join(&state.session_id);
    std::fs::create_dir_all(&output_dir)?;
    let output = Output::new(&output_dir.join("media.m3u8"), true)?;
    let mut captions = Captions::new(&output_dir, output.caption_offset())?;
    let mut output = Some(output);
    let mut compositor = Compositor::new()?;
    let mut current: Option<Active> = None;
    let mut prepared: Option<Prepared> = None;
    let mut take: Option<(String, String)> = None;
    let mut banner_art = None;
    let mut tick = 0_u64;
    let mut start = Instant::now();
    state.status = "live".into();
    state.event("channel_started", "Continuous HLS output is running", None);
    while !handle.shutdown.load(Ordering::Relaxed) {
        state.program_ms = tick * 1000 / FPS;
        // Command work is bounded per render tick; network threads never touch codecs.
        for command in commands.try_iter().take(16) {
            let id = command.id.clone();
            let result: Result<serde_json::Value, String> = (|| {
                if state.status != "live"
                    && matches!(
                        &command.action,
                        Action::Take(_) | Action::Banner(..) | Action::StartPublish(_)
                    )
                {
                    return Err("Start the channel first".into());
                }
                match command.action {
                    Action::StartChannel => {
                        if state.status == "stopping" {
                            return Err("Channel is still stopping".into());
                        }
                        if state.status == "stopped" {
                            let session = uuid::Uuid::new_v4().to_string();
                            let dir = root.join("hls").join(&session);
                            let opened = (|| -> Result<_> {
                                std::fs::create_dir_all(&dir)?;
                                let next = Output::new(&dir.join("media.m3u8"), true)?;
                                let next_captions = Captions::new(&dir, next.caption_offset())?;
                                Ok((next, next_captions))
                            })()
                            .map_err(|e| e.to_string())?;
                            output = Some(opened.0);
                            captions = opened.1;
                            state.session_id = session;
                            state.output_url = format!("/hls/{}/master.m3u8", state.session_id);
                            tick = 0;
                            state.program_ms = 0;
                            state.late_frames = 0;
                            state.underrun_frames = 0;
                            state.publish = Default::default();
                            start = Instant::now();
                            state.status = "live".into();
                            state.event(
                                "channel_started",
                                "Channel started with a fresh output session",
                                Some(id.clone()),
                            );
                        }
                    }
                    Action::StopChannel => {
                        if state.status == "live" {
                            let active_output = output.as_mut().unwrap();
                            active_output.stop_publish();
                            active_output.finish().map_err(|e| e.to_string())?;
                            captions
                                .finish(state.program_ms)
                                .map_err(|e| e.to_string())?;
                            current = None;
                            prepared = None;
                            if let Some((pending, _)) = take.take() {
                                state.event(
                                    "command_cancelled",
                                    "Channel stopped before take",
                                    Some(pending),
                                );
                            }
                            banner_art = None;
                            state.current = None;
                            state.preparing = None;
                            state.next_ready = false;
                            state.banner = None;
                            state.status = "stopping".into();
                            state.event(
                                "channel_stopping",
                                "Closing channel output and publisher",
                                Some(id.clone()),
                            );
                        }
                    }
                    Action::Register(asset) => {
                        state.assets.insert(asset.id.clone(), asset);
                    }
                    Action::Enqueue { asset_id, after_id } => {
                        if state
                            .assets
                            .get(&asset_id)
                            .is_none_or(|a| a.kind != "video")
                        {
                            return Err("unknown video asset".into());
                        }
                        let position = match after_id {
                            None => state.queue.len(),
                            Some(ref after)
                                if current.as_ref().is_some_and(|c| &c.item.id == after) =>
                            {
                                0
                            }
                            Some(after) => {
                                state
                                    .queue
                                    .iter()
                                    .position(|i| i.id == after)
                                    .ok_or("after_id is not in the schedule")?
                                    + 1
                            }
                        };
                        if state.queue.len() >= 500 {
                            return Err("queue limit is 500 items".into());
                        }
                        let item = QueueItem {
                            id: uuid::Uuid::new_v4().to_string(),
                            asset_id,
                            captions: vec![],
                        };
                        let item_id = item.id.clone();
                        state.queue.insert(position, item);
                        return Ok(serde_json::json!({"item_id":item_id}));
                    }
                    Action::Remove(item_id) => {
                        let index = state
                            .queue
                            .iter()
                            .position(|i| i.id == item_id)
                            .ok_or("item is not upcoming")?;
                        state.queue.remove(index);
                    }
                    Action::Reorder(ids) => {
                        validate_order(&state.queue, &ids)?;
                        state.queue = ids
                            .iter()
                            .map(|id| state.queue.iter().find(|i| &i.id == id).unwrap().clone())
                            .collect();
                    }
                    Action::Take(item_id) => {
                        let target = match item_id {
                            Some(id) => state.queue.iter().find(|i| i.id == id),
                            None => state.queue.first(),
                        }
                        .ok_or("no upcoming item to take")?
                        .id
                        .clone();
                        // Pin the requested item to the head; later queue edits can cancel it.
                        let position = state.queue.iter().position(|i| i.id == target).unwrap();
                        let item = state.queue.remove(position);
                        state.queue.insert(0, item);
                        if let Some((previous, _)) = take.take() {
                            state.event(
                                "command_cancelled",
                                "Take superseded by a newer request",
                                Some(previous),
                            );
                        }
                        take = Some((id.clone(), target.clone()));
                        return Ok(serde_json::json!({"pending":true,"item_id":target}));
                    }
                    Action::Clear => {
                        current = None;
                        prepared = None;
                        state.queue.clear();
                        if let Some((pending, _)) = take.take() {
                            state.event(
                                "command_cancelled",
                                "Rundown cleared before take",
                                Some(pending),
                            );
                        }
                    }
                    Action::StartPublish(request) => {
                        output.as_mut().unwrap().start_publish(request)?;
                    }
                    Action::StopPublish => {
                        if let Some(output) = &output {
                            output.stop_publish();
                        }
                    }
                    Action::Volume(v) => {
                        if !v.is_finite() || !(0.0..=2.0).contains(&v) {
                            return Err("volume must be between 0 and 2".into());
                        }
                        state.volume = v;
                    }
                    Action::Banner(request, art) => {
                        request.validate()?;
                        state.banner = Some(BannerState {
                            title: request.title,
                            asset_id: request.asset_id,
                            expires_at_ms: state.program_ms + request.duration_ms,
                            remaining_ms: request.duration_ms,
                        });
                        banner_art = art;
                    }
                    Action::RemoveBanner => {
                        state.banner = None;
                        banner_art = None;
                    }
                    Action::Captions(item_id, cues) => {
                        if cues.len() > 1000 {
                            return Err("maximum 1000 cues per item".into());
                        }
                        for cue in &cues {
                            cue.validate()?;
                        }
                        if let Some(item) = state.queue.iter_mut().find(|i| i.id == item_id) {
                            item.captions = cues;
                            if let Some(p) = &mut prepared
                                && p.item.id == item_id
                            {
                                p.item = item.clone();
                            }
                        } else if let Some(active) = &mut current {
                            if active.item.id != item_id {
                                return Err("unknown queue item".into());
                            }
                            let local_ms = (tick - active.start) * 1000 / FPS;
                            if cues.iter().any(|c| c.start_ms <= local_ms) {
                                return Err("live caption updates must start in the future".into());
                            }
                            active.item.captions.retain(|c| c.start_ms <= local_ms);
                            active.item.captions.extend(cues);
                        } else {
                            return Err("unknown queue item".into());
                        }
                    }
                }
                Ok(serde_json::json!({}))
            })();
            if let Ok(value) = &result {
                let pending = value
                    .get("pending")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                state.event(
                    if pending {
                        "command_accepted"
                    } else {
                        "command_applied"
                    },
                    if pending {
                        "Take accepted; preparing source"
                    } else {
                        "Control applied"
                    },
                    Some(id.clone()),
                );
            }
            let _ = command.reply.send(result.map(|value| serde_json::json!({"command_id":id,"program_ms":state.program_ms,"result":value})));
        }
        if state.status != "live" {
            if let Some(active_output) = &output {
                state.publish = active_output.publish_state();
                if active_output.publisher_finished() {
                    output = None;
                    state.status = "stopped".into();
                    state.event(
                        "channel_stopped",
                        "Channel stopped; upcoming rundown retained",
                        None,
                    );
                }
            }
            *handle.state.lock().unwrap() = state.clone();
            let _ = handle.events.send(state.clone());
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if current
            .as_ref()
            .is_some_and(|c| (tick - c.start) * 1000 / FPS >= c.duration_ms)
        {
            state.event("source_ended", "Source completed; output continues", None);
            current = None;
        }
        let wanted = state.queue.first().map(|i| i.id.clone());
        if take
            .as_ref()
            .is_some_and(|(_, target)| wanted.as_ref() != Some(target))
        {
            let (id, _) = take.take().unwrap();
            state.event(
                "command_cancelled",
                "Rundown edit cancelled the pending take",
                Some(id),
            );
        }
        if prepared.as_ref().map(|p| p.item.id.clone()) != wanted {
            prepared = state.queue.first().and_then(|item| {
                state.assets.get(&item.asset_id).map(|asset| Prepared {
                    item: item.clone(),
                    source: Source::spawn(asset),
                })
            });
        }
        if let Some(p) = &mut prepared {
            p.source.pump();
        }
        if let Some(error) = prepared.as_ref().and_then(|p| p.source.error.clone()) {
            state.event("source_failed", error, take.take().map(|(id, _)| id));
            state.queue.remove(0);
            prepared = None;
        }
        if (current.is_none() || take.is_some())
            && prepared.as_ref().is_some_and(|p| p.source.ready())
        {
            let p = prepared.take().unwrap();
            let asset = &state.assets[&p.item.asset_id];
            let duration_ms = asset.duration_ms;
            let name = asset.name.clone();
            state.queue.retain(|i| i.id != p.item.id);
            current = Some(Active {
                item: p.item,
                source: p.source,
                start: tick,
                duration_ms,
            });
            state.event(
                "source_taken",
                format!("On air: {name}"),
                take.take().map(|(id, _)| id),
            );
        }
        if state.queue.is_empty() {
            take = None;
        }
        if state
            .banner
            .as_ref()
            .is_some_and(|b| state.program_ms >= b.expires_at_ms)
        {
            state.banner = None;
            banner_art = None;
            state.event("banner_expired", "Program restored to full frame", None);
        }
        let mut caption_text = String::new();
        let (mut picture, audio) = if let Some(c) = &mut current {
            let local_tick = tick - c.start;
            let local_ms = local_tick * 1000 / FPS;
            let (frame, samples, underrun) = c.source.render(local_tick, state.volume);
            if underrun {
                state.underrun_frames += 1;
            }
            if let Some(error) = c.source.error.take() {
                state.event("source_failed", error, None);
            }
            caption_text = c
                .item
                .captions
                .iter()
                .filter(|cue| cue.start_ms <= local_ms && local_ms < cue.end_ms)
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (frame.unwrap_or_else(|| compositor::slate(tick)), samples)
        } else {
            (
                compositor::slate(tick),
                [vec![0.0; SAMPLES_PER_TICK], vec![0.0; SAMPLES_PER_TICK]],
            )
        };
        if let Some(banner) = &mut state.banner {
            banner.remaining_ms = banner.expires_at_ms.saturating_sub(state.program_ms);
            picture = compositor.banner(
                &picture,
                banner_art.as_ref(),
                &banner.title,
                banner.remaining_ms,
            )?;
        }
        let active_output = output.as_mut().unwrap();
        active_output.write(&mut picture, tick, &audio)?;
        let publish = active_output.publish_state();
        if publish.status != state.publish.status {
            state.event("publish_status", publish.message.clone(), None);
        }
        state.publish = publish;
        captions.tick(state.program_ms, caption_text)?;
        state.current = current.as_ref().map(|c| Playing {
            item: c.item.clone(),
            position_ms: (tick - c.start) * 1000 / FPS,
            duration_ms: c.duration_ms,
        });
        state.preparing = prepared.as_ref().map(|p| p.item.id.clone());
        state.next_ready = prepared.as_ref().is_some_and(|p| p.source.ready());
        if tick.is_multiple_of(6) {
            *handle.state.lock().unwrap() = state.clone();
            let _ = handle.events.send(state.clone());
        }
        tick += 1;
        let deadline = start + Duration::from_secs_f64(tick as f64 / FPS as f64);
        if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        } else {
            state.late_frames += 1;
        }
    }
    if let Some(output) = &mut output {
        output.stop_publish();
        output.finish()?;
    }
    state.status = "stopped".into();
    *handle.state.lock().unwrap() = state;
    Ok(())
}
