# Local control API

Base URL: `http://127.0.0.1:8787`. No FFmpeg commands are generated.
The service is loopback-only. Browser Origin must match Host (including port).
There is no remote authentication or multi-operator conflict protocol yet.

## State and events

`GET /api/state` returns the current snapshot. `WS /api/events` sends an initial
snapshot and updates five times per second. Each includes the latest 100 events,
their increasing `sequence`, `program_ms`, `kind` and optional `command_id`.
Reconnect clients use the snapshot rather than assuming every event was received.

Successful commands return:

```json
{"command_id":"…","program_ms":12333,"result":{}}
```

An enqueue result includes `item_id`. Take returns `pending: true`; observe the
correlated `source_taken`, `source_failed` or `command_cancelled` event for its
outcome. A take targets a stable queue item; changing the head before application
cancels the request. Ordinary edits are applied on the render thread.
Snapshots can trail command acknowledgments by 200 ms.

Validation errors return 400 with `{"error":"…"}`. Queue overload or an unavailable
engine returns 503. If a command times out, its outcome is uncertain: inspect
events using its command ID before retrying. Automatic retry/idempotency is not
implemented. Clients should serialize dependent edits and use returned item IDs.

## Media

`POST /api/assets`: multipart form containing one `file` field. Accepts MP4 with
H.264/AAC, or PNG/JPEG artwork. Video-only clips render silence. Returns asset
metadata. Files are stored under generated IDs, independent of uploaded names.

## Schedule

| Method | Path | JSON body |
|---|---|---|
| POST | `/api/queue` | `{"asset_id":"…","after_id":"…"}` |
| PUT | `/api/queue/order` | `{"ids":["item-2","item-1"]}` |
| DELETE | `/api/queue/{item_id}` | — |
| POST | `/api/take` | `{"item_id":"…"}` or `{}` for next |
| POST | `/api/clear` | — |
| PUT | `/api/volume` | `{"value":0.75}` (0–2) |

Omit `after_id` to append. The currently playing item's ID inserts at the head of
the upcoming queue. Reorder must name every upcoming item exactly once. Removing
an upcoming item does not affect current playback. Clear releases current and
upcoming sources but keeps output running with slate/silence. An idle channel
automatically takes the first prepared item.

## Timed L-band

`POST /api/banner`:

```json
{"title":"Your sponsor","duration_ms":30000,"asset_id":null}
```

Duration: 1–600 seconds. Optional `asset_id` refers to uploaded artwork. Design
artwork at 1280×720; the program covers the top-left 1024×576, leaving a bottom
144-pixel strip and right 256-pixel strip. Text and countdown render on top.
Title limit: 60 ASCII characters. A new banner replaces the current one.
`DELETE /api/banner` removes it. Expiry uses the program clock and emits
`banner_expired`, even with no browser connected. Viewer presentation follows
HLS latency. These controls change encoded pixels; they are not browser overlays.

## Captions

`PUT /api/queue/{item_id}/captions`:

```json
[{"start_ms":1000,"end_ms":6000,"text":"Caption relative to this clip"}]
```

For upcoming clips, replace all cues. For the current clip, supplied cues must
start in the future; already-started cues remain and future cues are replaced.
The engine maps cue time to the channel timeline. Captions end when their source
is taken off air. Up to 1000 cues per item, 1000 bytes per cue, end after start.
The web form edits a single future cue; the API supports arrays.

## Output

Read `output_url` from state. It points at a session-specific HLS master playlist
with H.264/AAC video/audio and an English WebVTT rendition. Each session has its
own directory, preserving output URL identity through live edits. Segment target
duration is two seconds and playlist window is six segments. Network errors,
player buffering and machine load can increase visible delay.
