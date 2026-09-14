use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

pub const FPS: u64 = 30;
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
pub const RATE: u64 = 48_000;
pub const SAMPLES_PER_TICK: usize = (RATE / FPS) as usize;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    #[serde(skip)]
    pub path: std::path::PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

impl Cue {
    pub fn validate(&self) -> Result<(), String> {
        if self.end_ms <= self.start_ms || self.end_ms > 86_400_000 {
            return Err("caption requires 0 <= start < end <= 24 hours".into());
        }
        if self.text.trim().is_empty() || self.text.len() > 1000 || self.text.contains("-->") {
            return Err("caption must contain 1–1000 bytes and no timing arrow".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: String,
    pub asset_id: String,
    #[serde(default)]
    pub captions: Vec<Cue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BannerRequest {
    pub duration_ms: u64,
    pub title: String,
    #[serde(default)]
    pub asset_id: Option<String>,
}

impl BannerRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !(1000..=600_000).contains(&self.duration_ms) {
            return Err("banner duration must be 1–600 seconds".into());
        }
        if self.title.len() > 60 || !self.title.is_ascii() {
            return Err("banner title must be at most 60 ASCII characters".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BannerState {
    pub title: String,
    pub asset_id: Option<String>,
    pub expires_at_ms: u64,
    pub remaining_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Playing {
    pub item: QueueItem,
    pub position_ms: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub sequence: u64,
    pub program_ms: u64,
    pub kind: String,
    pub message: String,
    pub command_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct State {
    pub session_id: String,
    pub program_ms: u64,
    pub status: String,
    pub output_url: String,
    pub assets: BTreeMap<String, Asset>,
    pub queue: Vec<QueueItem>,
    pub current: Option<Playing>,
    pub preparing: Option<String>,
    pub next_ready: bool,
    pub banner: Option<BannerState>,
    pub volume: f32,
    pub publish: crate::publish::PublishState,
    pub late_frames: u64,
    pub underrun_frames: u64,
    pub events: VecDeque<Event>,
}

impl State {
    pub fn new(session_id: String, assets: BTreeMap<String, Asset>) -> Self {
        Self {
            output_url: format!("/hls/{session_id}/master.m3u8"),
            session_id,
            program_ms: 0,
            status: "starting".into(),
            assets,
            queue: vec![],
            current: None,
            preparing: None,
            next_ready: false,
            banner: None,
            volume: 1.0,
            publish: Default::default(),
            late_frames: 0,
            underrun_frames: 0,
            events: VecDeque::new(),
        }
    }

    pub fn event(&mut self, kind: &str, message: impl Into<String>, command_id: Option<String>) {
        let sequence = self.events.back().map_or(1, |e| e.sequence + 1);
        self.events.push_back(Event {
            sequence,
            program_ms: self.program_ms,
            kind: kind.into(),
            message: message.into(),
            command_id,
        });
        if self.events.len() > 100 {
            self.events.pop_front();
        }
    }
}

pub fn validate_order(items: &[QueueItem], ids: &[String]) -> Result<(), String> {
    let expected: std::collections::BTreeSet<_> = items.iter().map(|i| &i.id).collect();
    let given: std::collections::BTreeSet<_> = ids.iter().collect();
    if expected != given || ids.len() != items.len() {
        Err("order must contain each upcoming item exactly once".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicate_reorders_and_invalid_timers() {
        let items = vec![
            QueueItem {
                id: "a".into(),
                asset_id: "x".into(),
                captions: vec![],
            },
            QueueItem {
                id: "b".into(),
                asset_id: "x".into(),
                captions: vec![],
            },
        ];
        assert!(validate_order(&items, &["b".into(), "a".into()]).is_ok());
        assert!(validate_order(&items, &["a".into(), "a".into()]).is_err());
        assert!(
            BannerRequest {
                duration_ms: 0,
                title: "Ad".into(),
                asset_id: None
            }
            .validate()
            .is_err()
        );
        assert!(
            Cue {
                start_ms: 100,
                end_ms: 50,
                text: "Bad".into()
            }
            .validate()
            .is_err()
        );
    }
}
