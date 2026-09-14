# Continuous playout

The Rust service owns the channel. A browser is a client, not the playback clock.
No production code spawns `ffmpeg` or `ffprobe`. `ffmpeg-next` links directly to
libavformat, libavcodec, libswscale, libswresample and libavutil.

## First executable slice

- One local channel: 1280×720, 30 fps, H.264, stereo 48 kHz AAC.
- Independent, bounded decoder workers for current and prepared-next MP4 sources.
- One monotonic program timeline across cuts, empty schedules and failed inputs.
- Persistent encoders and HLS muxer; two-second segments served over HTTP.
- HTTP commands with acknowledgments and WebSocket snapshots/events.
- Browser media upload, playlist controls, live status and delayed HLS preview.
- Timed L-band composition in the actual encoded program, with countdown and
  engine-owned expiry. PNG artwork and a text/color fallback.
- Caption cues mapped from clip time to program time and published as WebVTT HLS.

## Important boundaries

Only inputs the engine can access are playable. Uploaded media stays in ignored
`data/`. Output profiles stay fixed during a channel session. The engine decodes
and re-encodes rather than attempting arbitrary compressed-packet splicing.
The first release cuts between sources; transitions and RTMP are follow-up work.
HLS preview trails live engine state. Published segments/captions are immutable.
An empty channel renders a slate and silence. A failed source must not end output.
Queues are bounded: overload must be reported, not converted into unbounded RAM.

## Control semantics

Commands carry stable IDs. Applied events include program time. Editing the
upcoming queue does not mutate the currently playing item. A take waits for a
prepared source; it does not restart an encoder. Banner time is evaluated on the
program clock, never by browser timers. Shutdown drains encoders and closes HLS.

## Verification gates

1. Build and link native libraries; no CLI dependency in runtime.
2. Produce and decode HLS video/audio through the library API.
3. Insert, reorder, remove and take while timestamps remain continuous.
4. Verify banner pixels during its interval and restoration after expiry.
5. Verify caption timing across a source switch.
6. Exercise web controls, error responses, media uploads and reconnects.

## Challenge

Intended for the GPT-6 Astra Challenge. The supplied screenshot is context;
official eligibility, deadline/timezone and submission requirements have not
yet been verified. No submission or public deployment is implied by local work.
