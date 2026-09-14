//! Encoded-packet fan-out. No network calls run on the channel's render thread.
use anyhow::{Context, Result, bail};
use ffmpeg_next as av;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CString, c_void},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
pub struct PublishRequest {
    pub server_url: String,
    pub stream_key: String,
}

impl PublishRequest {
    pub fn validate(&self) -> Result<(), String> {
        let url = url::Url::parse(self.server_url.trim())
            .map_err(|_| "Enter a valid RTMP or RTMPS server URL")?;
        if !matches!(url.scheme(), "rtmp" | "rtmps")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "Use an rtmp:// or rtmps:// server URL without credentials, query or fragment"
                    .into(),
            );
        }
        if self.server_url.len() > 2048
            || self.stream_key.is_empty()
            || self.stream_key.len() > 512
            || self
                .stream_key
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '?' | '#' | '\\'))
        {
            return Err("Enter the stream key separately, without spaces or URL separators".into());
        }
        Ok(())
    }

    fn destination(&self) -> String {
        let url = url::Url::parse(self.server_url.trim()).expect("validated URL");
        format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap(),
            url.port().map(|p| format!(":{p}")).unwrap_or_default()
        )
    }

    fn url(&self) -> String {
        format!(
            "{}/{}",
            self.server_url.trim().trim_end_matches('/'),
            self.stream_key
        )
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PublishState {
    pub status: String,
    pub destination: Option<String>,
    pub message: String,
    pub packets_sent: u64,
    pub bytes_sent: u64,
}
impl Default for PublishState {
    fn default() -> Self {
        Self {
            status: "idle".into(),
            destination: None,
            message: "Not publishing".into(),
            packets_sent: 0,
            bytes_sent: 0,
        }
    }
}

pub struct Encoded {
    packet: av::Packet,
    time_base: av::Rational,
}
pub struct Publisher {
    tx: SyncSender<Encoded>,
    interrupt: Arc<Interrupt>,
    state: Arc<Mutex<PublishState>>,
    worker: Option<JoinHandle<()>>,
}

struct Interrupt {
    start: Instant,
    deadline_ms: AtomicU64,
    cancelled: AtomicBool,
}
impl Interrupt {
    fn arm(&self) {
        self.deadline_ms.store(
            self.start.elapsed().as_millis() as u64 + 5000,
            Ordering::Relaxed,
        );
    }
}
unsafe extern "C" fn interrupted(opaque: *mut c_void) -> i32 {
    // SAFETY: NativeMux holds the Arc backing opaque until after its native IO is closed.
    let guard = unsafe { &*(opaque as *const Interrupt) };
    i32::from(
        guard.cancelled.load(Ordering::Relaxed)
            || guard.start.elapsed().as_millis() as u64
                >= guard.deadline_ms.load(Ordering::Relaxed),
    )
}

/// Field order matters: native IO drops before its interrupt callback storage.
struct NativeMux {
    mux: av::format::context::Output,
    guard: Arc<Interrupt>,
}
impl NativeMux {
    fn open(
        request: &PublishRequest,
        parameters: [av::codec::Parameters; 2],
        guard: Arc<Interrupt>,
    ) -> Result<Self> {
        guard.arm();
        let target = CString::new(request.url())?;
        // SAFETY: FFmpeg allocates the context; Output::wrap assumes ownership exactly once.
        // Callback storage is Arc-backed and lives through open/write/close operations.
        let mut mux = unsafe {
            let mut ctx = ptr::null_mut();
            let result = av::ffi::avformat_alloc_output_context2(
                &mut ctx,
                ptr::null_mut(),
                c"flv".as_ptr(),
                ptr::null(),
            );
            if result < 0 || ctx.is_null() {
                bail!("Could not allocate RTMP muxer");
            }
            (*ctx).interrupt_callback = av::ffi::AVIOInterruptCB {
                callback: Some(interrupted),
                opaque: Arc::as_ptr(&guard) as *mut c_void,
            };
            let mut output = av::format::context::Output::wrap(ctx);
            let mut options = av::Dictionary::new();
            options.set("rw_timeout", "5000000");
            options.set("rtmp_live", "live");
            options.set("tls_verify", "1");
            let mut raw_options = options.disown();
            let result = av::ffi::avio_open2(
                &mut (*output.as_mut_ptr()).pb,
                target.as_ptr(),
                av::ffi::AVIO_FLAG_WRITE,
                &(*ctx).interrupt_callback,
                &mut raw_options,
            );
            let _ = av::Dictionary::own(raw_options);
            if result < 0 {
                return Err(av::Error::from(result).into());
            }
            output
        };
        for parameters in parameters {
            let mut stream = mux.add_stream(av::encoder::find(av::codec::Id::None))?;
            stream.set_parameters(parameters);
            stream.set_time_base((1, 1000));
            // Codec tags from other muxers are not valid FLV tags.
            unsafe {
                (*stream.parameters().as_mut_ptr()).codec_tag = 0;
            }
        }
        let mut options = av::Dictionary::new();
        options.set("flvflags", "no_duration_filesize");
        guard.arm();
        mux.write_header_with(options)?;
        Ok(Self { mux, guard })
    }

    fn write(&mut self, mut encoded: Encoded, origin: i64) -> Result<usize> {
        let index = encoded.packet.stream();
        // Convert from channel timestamps to a fresh connection-local clock.
        encoded.packet.rescale_ts(encoded.time_base, (1, 1000));
        if encoded.packet.dts().is_none_or(|dts| dts < origin) {
            return Ok(0);
        }
        encoded
            .packet
            .set_pts(encoded.packet.pts().map(|pts| pts - origin));
        encoded
            .packet
            .set_dts(encoded.packet.dts().map(|dts| dts - origin));
        encoded.packet.set_position(-1);
        let tb = self
            .mux
            .stream(index)
            .context("Missing RTMP stream")?
            .time_base();
        encoded.packet.rescale_ts((1, 1000), tb);
        let size = encoded.packet.size();
        self.guard.arm();
        encoded.packet.write_interleaved(&mut self.mux)?;
        Ok(size)
    }
}

impl Publisher {
    pub fn start(
        request: PublishRequest,
        parameters: [av::codec::Parameters; 2],
    ) -> Result<Self, String> {
        request.validate()?;
        let (tx, rx) = mpsc::sync_channel(256);
        let state = Arc::new(Mutex::new(PublishState {
            status: "connecting".into(),
            destination: Some(request.destination()),
            message: "Connecting to streaming server".into(),
            packets_sent: 0,
            bytes_sent: 0,
        }));
        let interrupt = Arc::new(Interrupt {
            start: Instant::now(),
            deadline_ms: AtomicU64::new(5000),
            cancelled: AtomicBool::new(false),
        });
        let status = state.clone();
        let guard = interrupt.clone();
        let worker = thread::Builder::new()
            .name("rtmp-publisher".into())
            .spawn(move || {
                let result = publish(request, parameters, rx, guard.clone(), status.clone());
                let mut s = status.lock().unwrap();
                if s.status == "failed" {
                    return;
                } // Preserve a queue-overflow diagnosis.
                if guard.cancelled.load(Ordering::Relaxed) {
                    s.status = "idle".into();
                    s.message = "Publishing stopped; local channel continues".into();
                } else if let Err(error) = result {
                    s.status = "failed".into();
                    // Errors are numeric/native descriptions, never target URLs or keys.
                    s.message = format!(
                        "Publish failed: {error}. Check the server, key and network; then retry."
                    );
                } else {
                    s.status = "idle".into();
                    s.message = "Publishing stopped".into();
                }
            })
            .map_err(|_| "Could not start publishing worker")?;
        Ok(Self {
            tx,
            interrupt,
            state,
            worker: Some(worker),
        })
    }

    pub fn send(&self, packet: &av::Packet, time_base: av::Rational) {
        if self.interrupt.cancelled.load(Ordering::Relaxed) {
            return;
        }
        match self.tx.try_send(Encoded {
            packet: packet.clone(),
            time_base,
        }) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.interrupt.cancelled.store(true, Ordering::Relaxed);
                let mut state = self.state.lock().unwrap();
                state.status = "failed".into();
                state.message="Publishing stopped because the network could not keep up. HLS continues; retry when the connection recovers.".into();
            }
        }
    }
    pub fn state(&self) -> PublishState {
        self.state.lock().unwrap().clone()
    }
    pub fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(|w| w.is_finished())
    }
    pub fn stop(&self) {
        self.interrupt.cancelled.store(true, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap();
        if matches!(
            state.status.as_str(),
            "connecting" | "waiting_keyframe" | "publishing"
        ) {
            state.status = "stopping".into();
            state.message = "Disconnecting publisher".into();
        }
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn publish(
    request: PublishRequest,
    parameters: [av::codec::Parameters; 2],
    rx: Receiver<Encoded>,
    guard: Arc<Interrupt>,
    state: Arc<Mutex<PublishState>>,
) -> Result<()> {
    let mut output = NativeMux::open(&request, parameters, guard.clone())?;
    drop(request); // Target/key never enter snapshots or persistence.
    {
        let mut s = state.lock().unwrap();
        s.status = "waiting_keyframe".into();
        s.message = "Connected; waiting for the next video keyframe".into();
    }
    let mut origin = None;
    while !guard.cancelled.load(Ordering::Relaxed) {
        let encoded = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        if origin.is_none() {
            if encoded.packet.stream() != 0 || !encoded.packet.is_key() {
                continue;
            }
            let dts = encoded
                .packet
                .dts()
                .context("Video packet has no timestamp")?;
            origin = Some(av::Rescale::rescale(&dts, encoded.time_base, (1, 1000)));
        }
        let size = output.write(encoded, origin.unwrap())?;
        if size > 0 {
            let mut s = state.lock().unwrap();
            s.status = "publishing".into();
            s.message = "Sending program video and audio".into();
            s.packets_sent += 1;
            s.bytes_sent += size as u64;
        }
    }
    // Interrupt remains armed on stop: disconnect immediately, rather than drain stale frames.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_targets_without_echoing_credentials() {
        for server in [
            "file:///tmp/key",
            "https://youtube.com/live2",
            "rtmp://user:secret@host/live2",
        ] {
            assert!(
                PublishRequest {
                    server_url: server.into(),
                    stream_key: "secret".into()
                }
                .validate()
                .is_err()
            );
        }
        let request = PublishRequest {
            server_url: "rtmps://a.rtmps.youtube.com:443/live2".into(),
            stream_key: "private-key".into(),
        };
        assert!(request.validate().is_ok());
        assert!(!request.destination().contains("private-key"));
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;
    #[test]
    fn loopback_publishes_decodable_audio_video_and_restarts_without_interrupting_hls() {
        av::init().unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let server = format!("rtmp://127.0.0.1:{port}/live");
        let target = format!("{server}/test-key");
        let receiver = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(12);
            let mut options = av::Dictionary::new();
            options.set("listen", "1");
            options.set("timeout", "10");
            options.set("analyzeduration", "500000");
            let mut input = av::format::input_with_interrupt_and_dictionary(
                &target,
                move || Instant::now() > deadline,
                options,
            )
            .unwrap();
            let mut video =
                av::codec::context::Context::from_parameters(input.stream(0).unwrap().parameters())
                    .unwrap()
                    .decoder()
                    .video()
                    .unwrap();
            let mut audio =
                av::codec::context::Context::from_parameters(input.stream(1).unwrap().parameters())
                    .unwrap()
                    .decoder()
                    .audio()
                    .unwrap();
            let mut counts = [0, 0];
            let mut last = [None, None];
            for (stream, packet) in input.packets() {
                let i = stream.index();
                let dts = packet.dts().unwrap();
                assert!(dts >= 0 && last[i].is_none_or(|old| dts > old));
                last[i] = Some(dts);
                if i == 0 {
                    video.send_packet(&packet).unwrap();
                    while video.receive_frame(&mut av::frame::Video::empty()).is_ok() {
                        counts[0] += 1;
                    }
                } else {
                    audio.send_packet(&packet).unwrap();
                    while audio.receive_frame(&mut av::frame::Audio::empty()).is_ok() {
                        counts[1] += 1;
                    }
                }
                if counts[0] > 60 && counts[1] > 60 {
                    break;
                }
            }
            counts
        });
        thread::sleep(Duration::from_millis(200));
        let dir = tempfile::tempdir().unwrap();
        let mut output = crate::output::Output::new(&dir.path().join("media.m3u8"), true).unwrap();
        let request = || PublishRequest {
            server_url: server.clone(),
            stream_key: "test-key".into(),
        };
        output.start_publish(request()).unwrap();
        assert!(output.start_publish(request()).is_err());
        for tick in 0..180 {
            let mut frame = crate::compositor::slate(tick);
            output
                .write(&mut frame, tick, &[vec![0.1; 1600], vec![0.1; 1600]])
                .unwrap();
            thread::sleep(Duration::from_millis(30));
        }
        let counts = receiver.join().unwrap();
        assert!(counts[0] > 60 && counts[1] > 60, "{counts:?}");
        output.stop_publish();
        let deadline = Instant::now() + Duration::from_secs(6);
        while !output.publisher_finished() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(20));
        }
        // A fresh connection to the now closed port must fail without preventing HLS writes.
        output.start_publish(request()).unwrap();
        for tick in 180..240 {
            output
                .write(
                    &mut crate::compositor::slate(tick),
                    tick,
                    &[vec![0.0; 1600], vec![0.0; 1600]],
                )
                .unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(output.publish_state().status, "failed");
        assert!(
            !serde_json::to_string(&output.publish_state())
                .unwrap()
                .contains("test-key")
        );
        output.finish().unwrap();
        assert!(
            std::fs::read_to_string(dir.path().join("media.m3u8"))
                .unwrap()
                .contains("#EXTINF")
        );
    }
}
