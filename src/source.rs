use crate::{
    compositor,
    model::{Asset, HEIGHT, RATE, SAMPLES_PER_TICK, WIDTH},
    output::{AUDIO_FORMAT, again},
};
use anyhow::{Context, Result, bail};
use ffmpeg_next as av;
use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

pub fn probe(path: &Path, id: String, name: String) -> Result<Asset> {
    let input = av::format::input(path)?;
    let video = input
        .streams()
        .best(av::media::Type::Video)
        .context("MP4 must have a video stream")?;
    let decoder = av::codec::context::Context::from_parameters(video.parameters())?
        .decoder()
        .video()?;
    if decoder.id() != av::codec::Id::H264 {
        bail!("first version accepts H.264 video in MP4");
    }
    if decoder.width() > 3840 || decoder.height() > 2160 {
        bail!("input resolution exceeds 3840x2160");
    }
    if let Some(audio) = input.streams().best(av::media::Type::Audio)
        && audio.parameters().id() != av::codec::Id::AAC
    {
        bail!("first version accepts AAC audio");
    }
    let duration_ms = (input.duration().max(0) / 1000) as u64;
    if duration_ms == 0 || duration_ms > 86_400_000 {
        bail!("input must have a known duration of at most 24 hours");
    }
    Ok(Asset {
        id,
        name,
        kind: "video".into(),
        duration_ms,
        width: decoder.width(),
        height: decoder.height(),
        path: path.to_path_buf(),
    })
}

enum Chunk {
    Video(f64, av::frame::Video),
    Audio(i64, [Vec<f32>; 2]),
    End,
    Error(String),
}

fn send(tx: &SyncSender<Chunk>, cancel: &AtomicBool, mut chunk: Chunk) -> Result<()> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("source cancelled");
        }
        match tx.try_send(chunk) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Disconnected(_)) => bail!("source released"),
            Err(TrySendError::Full(value)) => {
                chunk = value;
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn decode(path: &Path, tx: &SyncSender<Chunk>, cancel: &AtomicBool) -> Result<()> {
    let mut input = av::format::input(path)?;
    let stream = input
        .streams()
        .best(av::media::Type::Video)
        .context("missing video")?;
    let vi = stream.index();
    let vtb = f64::from(stream.time_base());
    let mut video = av::codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .video()?;
    // Normalize both tracks against the same container origin, preserving A/V offset.
    // SAFETY: input owns a live AVFormatContext for this entire worker.
    let origin = unsafe { (*input.as_ptr()).start_time };
    let origin = if origin == av::ffi::AV_NOPTS_VALUE {
        0.0
    } else {
        origin as f64 / 1_000_000.0
    };
    let factor = (WIDTH as f64 / video.width() as f64).min(HEIGHT as f64 / video.height() as f64);
    let w = ((video.width() as f64 * factor) as u32 / 2 * 2).max(2);
    let h = ((video.height() as f64 * factor) as u32 / 2 * 2).max(2);
    let mut scaler = av::software::scaling::Context::get(
        video.format(),
        video.width(),
        video.height(),
        av::format::Pixel::YUV420P,
        w,
        h,
        av::software::scaling::Flags::BILINEAR,
    )?;
    let mut audio = if let Some(stream) = input.streams().best(av::media::Type::Audio) {
        let decoder = av::codec::context::Context::from_parameters(stream.parameters())?
            .decoder()
            .audio()?;
        let resample = av::software::resampling::Context::get(
            decoder.format(),
            decoder.channel_layout(),
            decoder.rate(),
            AUDIO_FORMAT,
            av::ChannelLayout::STEREO,
            RATE as u32,
        )?;
        Some((
            stream.index(),
            f64::from(stream.time_base()),
            decoder,
            resample,
            None::<i64>,
        ))
    } else {
        None
    };
    let mut video_fallback = 0.0;
    let mut drain_video = |decoder: &mut av::decoder::Video| -> Result<()> {
        let mut frame = av::frame::Video::empty();
        loop {
            match decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    let t = frame
                        .timestamp()
                        .map_or(video_fallback, |pts| pts as f64 * vtb - origin);
                    video_fallback = t + 1.0 / 30.0;
                    let mut fitted = av::frame::Video::empty();
                    scaler.run(&frame, &mut fitted)?;
                    let mut full = compositor::canvas(16, 128, 128);
                    compositor::paste(
                        &mut full,
                        &fitted,
                        ((WIDTH - w) / 2 / 2 * 2) as usize,
                        ((HEIGHT - h) / 2 / 2 * 2) as usize,
                    );
                    send(tx, cancel, Chunk::Video(t, full))?;
                }
                Err(e) if again(e) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    };
    let drain_audio = |decoder: &mut av::decoder::Audio,
                       resample: &mut av::software::resampling::Context,
                       next: &mut Option<i64>,
                       tb: f64|
     -> Result<()> {
        let mut frame = av::frame::Audio::empty();
        loop {
            match decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    let at = *next.get_or_insert_with(|| {
                        ((frame.timestamp().unwrap_or(0) as f64 * tb - origin) * RATE as f64)
                            .round() as i64
                    });
                    // Explicit capacity permits 44.1kHz -> 48kHz expansion and delayed samples.
                    let capacity = (frame.samples() as u64 * RATE).div_ceil(decoder.rate() as u64)
                        as usize
                        + 256;
                    let mut out =
                        av::frame::Audio::new(AUDIO_FORMAT, capacity, av::ChannelLayout::STEREO);
                    resample.run(&frame, &mut out)?;
                    *next = Some(at + out.samples() as i64);
                    if out.samples() > 0 {
                        send(
                            tx,
                            cancel,
                            Chunk::Audio(
                                at,
                                [
                                    out.plane::<f32>(0)[..out.samples()].to_vec(),
                                    out.plane::<f32>(1)[..out.samples()].to_vec(),
                                ],
                            ),
                        )?;
                    }
                }
                Err(e) if again(e) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    };
    let mut packet = av::Packet::empty();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        match packet.read(&mut input) {
            Ok(()) => {}
            Err(av::Error::Eof) => break,
            Err(e) => return Err(e.into()),
        }
        if packet.stream() == vi {
            video.send_packet(&packet)?;
            drain_video(&mut video)?;
        }
        if let Some((index, tb, decoder, resample, next)) = &mut audio
            && packet.stream() == *index
        {
            decoder.send_packet(&packet)?;
            drain_audio(decoder, resample, next, *tb)?;
        }
    }
    video.send_eof()?;
    drain_video(&mut video)?;
    if let Some((_, tb, decoder, resample, next)) = &mut audio {
        decoder.send_eof()?;
        drain_audio(decoder, resample, next, *tb)?;
        loop {
            let mut out = av::frame::Audio::new(AUDIO_FORMAT, 4096, av::ChannelLayout::STEREO);
            let delay = resample.flush(&mut out)?;
            let at = next.unwrap_or(0);
            *next = Some(at + out.samples() as i64);
            if out.samples() > 0 {
                send(
                    tx,
                    cancel,
                    Chunk::Audio(
                        at,
                        [
                            out.plane::<f32>(0)[..out.samples()].to_vec(),
                            out.plane::<f32>(1)[..out.samples()].to_vec(),
                        ],
                    ),
                )?;
            }
            if delay.is_none() || out.samples() == 0 {
                break;
            }
        }
    }
    send(tx, cancel, Chunk::End)
}

pub struct Source {
    rx: Receiver<Chunk>,
    cancel: Arc<AtomicBool>,
    video: VecDeque<(f64, av::frame::Video)>,
    audio: VecDeque<(i64, [Vec<f32>; 2])>,
    last: Option<av::frame::Video>,
    pub error: Option<String>,
    pub ended: bool,
}

impl Source {
    pub fn spawn(asset: &Asset) -> Self {
        let (tx, rx) = mpsc::sync_channel(24);
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let path = asset.path.clone();
        thread::spawn(move || {
            if let Err(e) = decode(&path, &tx, &flag) {
                let _ = send(&tx, &flag, Chunk::Error(format!("{e:#}")));
            }
        });
        Self {
            rx,
            cancel,
            video: VecDeque::new(),
            audio: VecDeque::new(),
            last: None,
            error: None,
            ended: false,
        }
    }

    pub fn pump(&mut self) {
        // Limits include worker channel and render-side staging; no entire-file decode.
        while self.video.len() < 24 && self.audio.len() < 96 {
            match self.rx.try_recv() {
                Ok(Chunk::Video(at, frame)) => self.video.push_back((at, frame)),
                Ok(Chunk::Audio(at, samples)) => self.audio.push_back((at, samples)),
                Ok(Chunk::End) => self.ended = true,
                Ok(Chunk::Error(e)) => {
                    self.error = Some(e);
                    self.ended = true;
                }
                Err(_) => break,
            }
        }
    }

    pub fn ready(&self) -> bool {
        !self.video.is_empty()
    }

    pub fn render(
        &mut self,
        local_tick: u64,
        volume: f32,
    ) -> (Option<av::frame::Video>, [Vec<f32>; 2], bool) {
        self.pump();
        let time = local_tick as f64 / 30.0;
        while self
            .video
            .front()
            .is_some_and(|(at, _)| *at <= time + 0.001)
        {
            self.last = self.video.pop_front().map(|(_, frame)| frame);
        }
        let start = local_tick as i64 * SAMPLES_PER_TICK as i64;
        let end = start + SAMPLES_PER_TICK as i64;
        let mut samples = [vec![0.0; SAMPLES_PER_TICK], vec![0.0; SAMPLES_PER_TICK]];
        while let Some((at, block)) = self.audio.front() {
            let block_end = *at + block[0].len() as i64;
            if block_end <= start {
                self.audio.pop_front();
                continue;
            }
            if *at >= end {
                break;
            }
            let from = start.max(*at);
            let to = end.min(block_end);
            for ch in 0..2 {
                for n in from..to {
                    samples[ch][(n - start) as usize] =
                        (block[ch][(n - at) as usize] * volume).clamp(-1.0, 1.0);
                }
            }
            if block_end <= end {
                self.audio.pop_front();
            } else {
                break;
            }
        }
        let underrun = self.last.is_none() || (self.video.is_empty() && !self.ended);
        (self.last.clone(), samples, underrun)
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
