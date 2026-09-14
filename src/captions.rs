use anyhow::Result;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

pub fn atomic_write(path: &Path, text: &str) -> Result<()> {
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

fn timestamp(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

#[derive(Clone)]
struct Span {
    start: u64,
    end: u64,
    text: String,
}

pub struct Captions {
    root: PathBuf,
    spans: VecDeque<Span>,
    active: Option<Span>,
    sequence: u64,
    transport_offset: u64,
}

impl Captions {
    pub fn new(root: &Path, transport_offset: u64) -> Result<Self> {
        atomic_write(
            &root.join("master.m3u8"),
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"captions\",NAME=\"English\",LANGUAGE=\"en\",AUTOSELECT=YES,DEFAULT=YES,URI=\"captions.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=3200000,RESOLUTION=1280x720,SUBTITLES=\"captions\"\nmedia.m3u8\n",
        )?;
        Ok(Self {
            root: root.to_path_buf(),
            spans: VecDeque::new(),
            active: None,
            sequence: 0,
            transport_offset,
        })
    }

    pub fn tick(&mut self, ms: u64, text: String) -> Result<()> {
        if self.active.as_ref().map(|s| s.text.as_str()).unwrap_or("") != text {
            if let Some(mut old) = self.active.take() {
                old.end = ms;
                self.spans.push_back(old);
            }
            if !text.is_empty() {
                self.active = Some(Span {
                    start: ms,
                    end: ms,
                    text,
                });
            }
        }
        if let Some(active) = &mut self.active {
            active.end = ms;
        }
        while ms >= (self.sequence + 1) * 2000 {
            self.publish()?;
            self.sequence += 1;
        }
        Ok(())
    }

    fn publish(&mut self) -> Result<()> {
        let start = self.sequence * 2000;
        let end = start + 2000;
        let mut vtt = format!(
            "WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:{}\n\n",
            self.transport_offset
        );
        for span in self.spans.iter().chain(self.active.iter()) {
            if span.start < end && span.end > start {
                let safe = span
                    .text
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                vtt.push_str(&format!(
                    "{} --> {}\n{}\n\n",
                    timestamp(span.start.max(start)),
                    timestamp(span.end.min(end)),
                    safe
                ));
            }
        }
        atomic_write(
            &self.root.join(format!("caption-{}.vtt", self.sequence)),
            &vtt,
        )?;
        let first = self.sequence.saturating_sub(5);
        let mut playlist = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n"
        );
        for i in first..=self.sequence {
            playlist.push_str(&format!("#EXTINF:2.000,\ncaption-{i}.vtt\n"));
        }
        atomic_write(&self.root.join("captions.m3u8"), &playlist)?;
        if self.sequence >= 9 {
            let _ =
                std::fs::remove_file(self.root.join(format!("caption-{}.vtt", self.sequence - 9)));
        }
        while self.spans.front().is_some_and(|s| s.end <= start) {
            self.spans.pop_front();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cues_split_at_segments_and_do_not_rewrite_published_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = Captions::new(dir.path(), 1920).unwrap();
        c.tick(1000, "A < B".into()).unwrap();
        c.tick(2000, "A < B".into()).unwrap();
        let first = std::fs::read_to_string(dir.path().join("caption-0.vtt")).unwrap();
        assert!(first.contains("00:00:01.000 --> 00:00:02.000"));
        assert!(first.contains("A &lt; B"));
        c.tick(2500, "".into()).unwrap();
        c.tick(4000, "".into()).unwrap();
        assert_eq!(
            first,
            std::fs::read_to_string(dir.path().join("caption-0.vtt")).unwrap()
        );
        assert!(
            std::fs::read_to_string(dir.path().join("caption-1.vtt"))
                .unwrap()
                .contains("00:00:02.000 --> 00:00:02.500")
        );
    }
}
