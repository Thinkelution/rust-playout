use crate::model::{FPS, HEIGHT, RATE, WIDTH};
use anyhow::{Context, Result, bail};
use ffmpeg_next as av;
use std::{collections::VecDeque, path::Path};

pub const AUDIO_FORMAT: av::format::Sample =
    av::format::Sample::F32(av::format::sample::Type::Planar);

pub fn again(error: av::Error) -> bool {
    matches!(
        error,
        av::Error::Eof
            | av::Error::Other {
                errno: av::error::EAGAIN
            }
    )
}

/// Owned by the single render thread for the entire channel session.
pub struct Output {
    mux: av::format::context::Output,
    video: av::encoder::Video,
    audio: av::encoder::Audio,
    fifo: [VecDeque<f32>; 2],
    audio_pts: i64,
    closed: bool,
}

impl Output {
    pub fn new(path: &Path, hls: bool) -> Result<Self> {
        let mut mux = if hls {
            av::format::output_as(path, "hls")?
        } else {
            av::format::output(path)?
        };
        let global = mux
            .format()
            .flags()
            .contains(av::format::Flags::GLOBAL_HEADER);
        let vc = av::encoder::find_by_name("libx264")
            .context("FFmpeg libraries must include libx264")?;
        let mut video = av::codec::context::Context::new_with_codec(vc)
            .encoder()
            .video()?;
        video.set_width(WIDTH);
        video.set_height(HEIGHT);
        video.set_format(av::format::Pixel::YUV420P);
        video.set_time_base((1, FPS as i32));
        video.set_frame_rate(Some((FPS as i32, 1)));
        video.set_gop((FPS * 2) as u32);
        video.set_max_b_frames(0);
        video.set_bit_rate(2_500_000);
        if global {
            video.set_flags(av::codec::Flags::GLOBAL_HEADER);
        }
        let mut options = av::Dictionary::new();
        options.set("preset", "veryfast");
        options.set("tune", "zerolatency");
        options.set("sc_threshold", "0");
        options.set("keyint_min", "60");
        let video = video.open_with(options)?;
        {
            let mut stream = mux.add_stream(vc)?;
            stream.set_time_base((1, FPS as i32));
            stream.set_parameters(&video);
        }
        let ac = av::encoder::find(av::codec::Id::AAC).context("AAC encoder missing")?;
        let mut audio = av::codec::context::Context::new_with_codec(ac)
            .encoder()
            .audio()?;
        audio.set_rate(RATE as i32);
        audio.set_channel_layout(av::ChannelLayout::STEREO);
        audio.set_format(AUDIO_FORMAT);
        audio.set_bit_rate(128_000);
        audio.set_time_base((1, RATE as i32));
        if global {
            audio.set_flags(av::codec::Flags::GLOBAL_HEADER);
        }
        let audio = audio.open_as(ac)?;
        {
            let mut stream = mux.add_stream(ac)?;
            stream.set_time_base((1, RATE as i32));
            stream.set_parameters(&audio);
        }
        let mut options = av::Dictionary::new();
        if hls {
            options.set("avoid_negative_ts", "make_zero");
            options.set("hls_time", "2");
            options.set("hls_list_size", "6");
            options.set("hls_delete_threshold", "3");
            options.set(
                "hls_flags",
                "delete_segments+independent_segments+temp_file",
            );
            let segments = path
                .parent()
                .context("missing output directory")?
                .join("segment-%06d.ts");
            options.set(
                "hls_segment_filename",
                segments.to_str().context("non-UTF8 output path")?,
            );
        }
        mux.write_header_with(options)?;
        Ok(Self {
            mux,
            video,
            audio,
            fifo: Default::default(),
            audio_pts: 0,
            closed: false,
        })
    }

    pub fn write(
        &mut self,
        picture: &mut av::frame::Video,
        tick: u64,
        samples: &[Vec<f32>; 2],
    ) -> Result<()> {
        picture.set_pts(Some(tick as i64));
        picture.set_kind(av::picture::Type::None);
        self.video.send_frame(picture)?;
        self.drain_video()?;
        for (fifo, input) in self.fifo.iter_mut().zip(samples) {
            fifo.extend(input);
        }
        let size = self.audio.frame_size() as usize;
        if size == 0 {
            bail!("AAC encoder returned zero frame size");
        }
        while self.fifo[0].len() >= size {
            self.write_audio(size)?;
        }
        Ok(())
    }

    fn write_audio(&mut self, size: usize) -> Result<()> {
        let mut frame = av::frame::Audio::new(AUDIO_FORMAT, size, av::ChannelLayout::STEREO);
        frame.set_rate(RATE as u32);
        frame.set_pts(Some(self.audio_pts));
        for ch in 0..2 {
            for sample in &mut frame.plane_mut::<f32>(ch)[..size] {
                *sample = self.fifo[ch].pop_front().unwrap_or(0.0);
            }
        }
        self.audio_pts += size as i64;
        self.audio.send_frame(&frame)?;
        self.drain_audio()
    }

    fn drain_video(&mut self) -> Result<()> {
        let tb = self
            .mux
            .stream(0)
            .context("missing video stream")?
            .time_base();
        let mut packet = av::Packet::empty();
        loop {
            match self.video.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(0);
                    packet.set_duration(1);
                    packet.rescale_ts((1, FPS as i32), tb);
                    packet.write_interleaved(&mut self.mux)?;
                }
                Err(e) if again(e) => break,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn drain_audio(&mut self) -> Result<()> {
        let tb = self
            .mux
            .stream(1)
            .context("missing audio stream")?
            .time_base();
        let mut packet = av::Packet::empty();
        loop {
            match self.audio.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(1);
                    packet.rescale_ts((1, RATE as i32), tb);
                    packet.write_interleaved(&mut self.mux)?;
                }
                Err(e) if again(e) => break,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        if !self.fifo[0].is_empty() {
            self.write_audio(self.audio.frame_size() as usize)?;
        }
        self.video.send_eof()?;
        self.drain_video()?;
        self.audio.send_eof()?;
        self.drain_audio()?;
        self.mux.write_trailer()?;
        Ok(())
    }

    pub fn caption_offset(&self) -> u64 {
        // SAFETY: this encoder owns a live context. make_zero shifts AAC priming
        // below zero and video together; WebVTT must use the same transport origin.
        let padding = unsafe { (*self.audio.as_ptr()).initial_padding.max(0) } as u64;
        padding * 90_000 / RATE
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Err(e) = self.finish() {
            tracing::warn!("closing output: {e:#}");
        }
    }
}
