# Rust Playout

An API-controlled live playout engine with a web control room. Change upcoming
media and insert timed L-band ads while one HLS channel keeps running.

Built in Rust using **FFmpeg libraries directly**, not the `ffmpeg` executable.
See [architecture](docs/architecture.md) for scope and verification gates.

## Development prerequisites

- Rust toolchain, Node.js 22+, pkg-config and Clang/libclang.
- FFmpeg 9 development libraries with an H.264 encoder (`libx264`) and AAC.
- macOS: `brew install rust ffmpeg pkg-config` plus Xcode command-line tools.

This is an early implementation, not a production broadcast system.

## Run the control room

```sh
cd web
npm ci
npm run build
cd ..
cargo run --release -- --demo
```

Open **http://127.0.0.1:8787**. The service starts a live standby slate. `--demo`
creates three 15-second H.264/AAC clips using the native libraries on first run.
Click **Queue all media**, change the upcoming order, and **Take banner** to put
an L-shaped ad and countdown into the actual encoded output. Clip playback and
banner expiry continue if you close the browser.

The HLS monitor is delayed by segment generation and player buffering. The
program clock and applied events report engine time. Source preview is separate.

For development, run `npm run dev` inside `web/` with the engine running. Vite
proxies `/api`, `/hls` and `/media` to the service. Use the production build above
for the same-origin control checks; the Vite proxy preserves the browser Host.

Environment: `PLAYOUT_PORT` (default `8787`), `PLAYOUT_DATA` (default `data`),
`RUST_LOG` for logging. Run the binary from the repository root to serve `web/dist`.
Ctrl-C stops the server and drains the encoders.

## Implemented

- H.264 MP4 sources with AAC audio (video-only files get silence), up to 4K input.
- Normalized 720p30 H.264 + 48 kHz stereo AAC, continuously encoded to HLS.
- Upload media, insert after a particular item, reorder, remove and take next.
- Bounded independent decoder workers and prepared-next source.
- Timed PNG/JPEG or text/color L-band, countdown, replace/remove and auto restore.
- Clip-relative caption cues published as segmented WebVTT with transport timing.
- HTTP command acknowledgments, correlated take events and WebSocket snapshots.
- Media metadata persists; each engine restart creates a fresh channel session.

API details and examples: [control API](docs/api.md).

## Verification

```sh
cargo test --release
cargo clippy --all-targets -- -D warnings
cargo build --release && python3 tests/api_smoke.py
cd web && npm ci && npm run build
```

The integration test generates real MP4 sources, starts the engine, changes the
live rundown, takes a new source and inserts a timed banner. It then decodes the
HLS output through libavformat/libavcodec and checks video timestamp continuity,
banner/restoration pixels, non-silent source audio and caption transport mapping.
It does not require an FFmpeg executable, browser, or network service.

## Current boundaries

One channel, local MP4 inputs, one fixed output profile and hard cuts. CPU
composition, one active L-band (80% program scale, bottom/right artwork), one
English subtitle rendition. Banner text/countdown uses an ASCII bitmap font.
The service starts on localhost without accounts; remote hosting needs a proper
authentication layer. Upload limit: 250 MB per file. Artwork limit: 4096×4096.

Rundown and captions are session state and are not restored after restart.
Finished HLS sessions remain under `data/hls/`; retention across restarts is not
automated. A filesystem/output failure marks the channel failed; automatic output
recovery, RTMP, network/live inputs, crossfades, GPU acceleration, broadcast
caption formats, multi-user edit conflicts and extended soak testing are future
milestones. Sustained overload is reported, not guaranteed to meet real-time.

This project links third-party native libraries. Packaging/distribution must
account for the licenses of the actual FFmpeg build and enabled encoders.
