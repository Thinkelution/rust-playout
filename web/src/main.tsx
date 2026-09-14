import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import Hls from "hls.js";
import {
  Radio,
  ArrowUpRight,
  Plus,
  Upload,
  Film,
  ArrowUp,
  ArrowDown,
  X,
  Play,
  SkipForward,
  Layers,
  Clock3,
  Captions,
  Volume2,
  Code2,
  ExternalLink,
  Circle,
  Check,
  AlertCircle,
} from "lucide-react";
import "./style.css";
import { PublishPanel, type PublishState } from "./PublishPanel";

type Asset = {
  id: string;
  name: string;
  kind: string;
  duration_ms: number;
  width: number;
  height: number;
};
type Cue = { start_ms: number; end_ms: number; text: string };
type Item = { id: string; asset_id: string; captions: Cue[] };
type Channel = {
  session_id: string;
  program_ms: number;
  status: string;
  output_url: string;
  assets: Record<string, Asset>;
  queue: Item[];
  current: { item: Item; position_ms: number; duration_ms: number } | null;
  preparing: string | null;
  next_ready: boolean;
  banner: { title: string; remaining_ms: number; expires_at_ms: number } | null;
  volume: number;
  publish: PublishState;
  late_frames: number;
  underrun_frames: number;
  events: {
    sequence: number;
    program_ms: number;
    kind: string;
    message: string;
    command_id: string | null;
  }[];
};
const time = (ms: number) => {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 3600)
    .toString()
    .padStart(
      2,
      "0",
    )}:${Math.floor(s / 60) % 60 < 10 ? "0" : ""}${Math.floor(s / 60) % 60}:${(s % 60).toString().padStart(2, "0")}`;
};
const shortTime = (ms: number) => time(ms).slice(3);

function Player({ url }: { url: string }) {
  const ref = useRef<HTMLVideoElement>(null);
  const [status, setStatus] = useState("Connecting to HLS…");
  useEffect(() => {
    const video = ref.current!;
    let hls: Hls | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    setStatus("Waiting for the first segments…");
    const connect = () => {
      if (disposed) return;
      if (Hls.isSupported()) {
        hls?.destroy();
        hls = new Hls({ liveSyncDurationCount: 2, maxBufferLength: 12 });
        hls.loadSource(url);
        hls.attachMedia(video);
        hls.on(Hls.Events.MANIFEST_PARSED, () => {
          void video.play().catch(() => setStatus("Press play to preview"));
        });
        hls.on(Hls.Events.ERROR, (_, data) => {
          if (data.fatal) {
            setStatus("Reconnecting to HLS…");
            timer = setTimeout(connect, 2000);
          }
        });
      } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
        video.src = url;
        void video.play().catch(() => setStatus("Press play to preview"));
      } else setStatus("HLS playback is not supported in this browser");
    };
    connect();
    return () => {
      disposed = true;
      clearTimeout(timer);
      hls?.destroy();
    };
  }, [url]);
  return (
    <div className="player">
      <video
        ref={ref}
        controls
        autoPlay
        muted
        playsInline
        onPlaying={() => setStatus("")}
        onWaiting={() => setStatus("Buffering…")}
        onError={() => setStatus("Preview unavailable; check channel status")}
      />
      {status && (
        <div className="player-message">
          <Radio size={24} />
          {status}
        </div>
      )}
    </div>
  );
}

function App() {
  const [channel, setChannel] = useState<Channel | null>(null),
    [connected, setConnected] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [uploading, setUploading] = useState(false),
    [busy, setBusy] = useState(false),
    [tab, setTab] = useState<"video" | "image">("video");
  const [title, setTitle] = useState("A little space. A big impression."),
    [duration, setDuration] = useState(30),
    [art, setArt] = useState("");
  const [captionItem, setCaptionItem] = useState(""),
    [cueText, setCueText] = useState(""),
    [cueStart, setCueStart] = useState(2),
    [cueEnd, setCueEnd] = useState(8);
  const [preview, setPreview] = useState<Asset | null>(null),
    [apiOpen, setApiOpen] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const [volumeDraft, setVolumeDraft] = useState(1);
  useEffect(() => {
    setVolumeDraft(channel?.volume ?? 1);
  }, [channel?.volume]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(""), 4500);
    return () => clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    let socket: WebSocket;
    let reconnect: ReturnType<typeof setTimeout>;
    let disposed = false;
    const connect = () => {
      socket = new WebSocket(
        `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/events`,
      );
      socket.onopen = () => setConnected(true);
      socket.onmessage = (e) => {
        try {
          setChannel(JSON.parse(e.data));
        } catch {
          setError("Received an invalid channel update");
        }
      };
      socket.onclose = () => {
        setConnected(false);
        if (!disposed) reconnect = setTimeout(connect, 1500);
      };
      socket.onerror = () => socket.close();
    };
    connect();
    return () => {
      disposed = true;
      clearTimeout(reconnect);
      socket?.close();
    };
  }, []);
  async function request(path: string, method = "POST", body?: unknown) {
    const response = await fetch(`/api${path}`, {
      method,
      headers: body === undefined ? {} : { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await response.text();
    let value;
    try {
      value = JSON.parse(text);
    } catch {
      throw new Error(text || `Request failed (${response.status})`);
    }
    if (!response.ok)
      throw new Error(value.error || `Request failed (${response.status})`);
    return value;
  }
  async function act(
    path: string,
    method = "POST",
    body?: unknown,
    message = "Control applied",
  ) {
    setBusy(true);
    setError("");
    try {
      const result = await request(path, method, body);
      setNotice(
        result.result?.pending ? "Take accepted — preparing source" : message,
      );
      return result;
    } catch (e) {
      setError(String(e));
      return null;
    } finally {
      setBusy(false);
    }
  }
  async function upload(files: FileList | null) {
    if (!files?.length) return;
    setUploading(true);
    setError("");
    try {
      for (const file of Array.from(files)) {
        const body = new FormData();
        body.append("file", file);
        const response = await fetch("/api/assets", { method: "POST", body });
        const text = await response.text();
        let value;
        try {
          value = JSON.parse(text);
        } catch {
          throw new Error(text);
        }
        if (!response.ok) throw new Error(value.error || "Upload failed");
      }
      setNotice("Media uploaded and ready");
    } catch (e) {
      setError(String(e));
    } finally {
      setUploading(false);
      if (fileInput.current) fileInput.current.value = "";
    }
  }
  const assets = Object.values(channel?.assets ?? {}),
    videos = assets.filter((a) => a.kind === "video"),
    images = assets.filter((a) => a.kind === "image");
  const current = channel?.current;
  const currentAsset = current
    ? channel?.assets[current.item.asset_id]
    : undefined;
  const items = [...(current ? [current.item] : []), ...(channel?.queue ?? [])];
  const ready = connected && channel?.status === "live";
  async function move(index: number, delta: number) {
    const ids = channel!.queue.map((i) => i.id);
    [ids[index], ids[index + delta]] = [ids[index + delta], ids[index]];
    await act("/queue/order", "PUT", { ids }, "Rundown updated");
  }
  async function demo() {
    setBusy(true);
    setError("");
    try {
      for (const asset of videos) {
        await request("/queue", "POST", { asset_id: asset.id });
      }
      setNotice("Media added to the rundown");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      <header className="topbar">
        <a className="brand" href="/">
          <span className="brand-mark">
            <Radio size={22} />
          </span>
          <b>
            rust<span>/</span>playout
          </b>
          <span className="alpha">PREVIEW 0.1</span>
        </a>
        <nav>
          <span className="nav-active">Control room</span>
          <button onClick={() => setApiOpen(true)}>
            <Code2 size={15} /> API reference
          </button>
        </nav>
        <span className={`connection ${connected ? "" : "offline"}`}>
          <i />
          {connected ? "Engine connected" : "Reconnecting"}
        </span>
      </header>
      <main>
        <section className="page-heading">
          <div>
            <div className="eyebrow">WORKSPACE / LOCAL CHANNEL</div>
            <h1>
              Channel 01 <span className="channel-tag">HLS</span>
            </h1>
            <p>A continuous signal. A rundown you can change.</p>
          </div>
          <div className="heading-actions">
            <div className="program-clock">
              <span>PROGRAM CLOCK</span>
              <strong>{time(channel?.program_ms ?? 0)}</strong>
            </div>
            <a
              className="button secondary"
              href={channel?.output_url}
              target="_blank"
              rel="noreferrer"
            >
              <ExternalLink size={15} /> HLS manifest
            </a>
          </div>
        </section>
        {error && (
          <div className="alert" role="alert">
            <AlertCircle size={18} />
            <span>{error}</span>
            <button aria-label="Dismiss error" onClick={() => setError("")}>
              <X size={16} />
            </button>
          </div>
        )}
        <div className="workbench">
          <section className="panel monitor">
            <div className="panel-heading">
              <div>
                <span className={`status-dot ${ready ? "live" : ""}`} />
                <h2>Program monitor</h2>
                <span className="badge">{channel?.status ?? "CONNECTING"}</span>
              </div>
              <span className="muted small">1280 × 720 · 30 fps</span>
            </div>
            {channel ? (
              <Player url={channel.output_url} />
            ) : (
              <div className="player connecting">
                <Radio size={32} />
                <p>Connecting to your channel</p>
              </div>
            )}
            <div className="monitor-info">
              <div>
                <span className="eyebrow">ON AIR NOW</span>
                <strong>{currentAsset?.name ?? "Standby slate"}</strong>
              </div>
              <div className="remaining">
                <span className="eyebrow">REMAINING</span>
                <strong>
                  {current
                    ? shortTime(
                        Math.max(0, current.duration_ms - current.position_ms),
                      )
                    : "—"}
                </strong>
              </div>
            </div>
            <div className="progress">
              <span
                style={{
                  width: current
                    ? `${Math.min(100, (current.position_ms / current.duration_ms) * 100)}%`
                    : "0%",
                }}
              />
            </div>
            <div className="monitor-footer">
              <span>
                <Clock3 size={13} /> HLS preview trails the program clock
              </span>
              <label>
                <Volume2 size={15} />
                <input
                  aria-label="Program volume"
                  type="range"
                  min="0"
                  max="2"
                  step="0.05"
                  value={volumeDraft}
                  onChange={(event) =>
                    setVolumeDraft(Number(event.target.value))
                  }
                  disabled={!ready}
                  onPointerUp={(e) =>
                    void act(
                      "/volume",
                      "PUT",
                      { value: Number(e.currentTarget.value) },
                      "Program volume updated",
                    )
                  }
                  onKeyUp={(e) =>
                    void act(
                      "/volume",
                      "PUT",
                      { value: Number(e.currentTarget.value) },
                      "Program volume updated",
                    )
                  }
                />
                <span>{Math.round((channel?.volume ?? 1) * 100)}%</span>
              </label>
            </div>
          </section>
          <section className="panel rundown">
            <div className="panel-heading">
              <div>
                <Layers size={17} />
                <h2>Live rundown</h2>
                <span className="count">{channel?.queue.length ?? 0}</span>
              </div>
              <span className="muted small">UPCOMING</span>
            </div>
            <div className="rundown-list">
              {channel?.queue.length ? (
                channel.queue.map((item, index) => {
                  const asset = channel.assets[item.asset_id];
                  return (
                    <article
                      className={`rundown-item ${index === 0 ? "next" : ""}`}
                      key={item.id}
                    >
                      <div className="rundown-index">
                        {String(index + 1).padStart(2, "0")}
                      </div>
                      <div className="rundown-body">
                        <div className="row-label">
                          {index === 0 ? (
                            <span className="ready-label">
                              {channel.next_ready
                                ? "● READY NEXT"
                                : "◌ PREPARING"}
                            </span>
                          ) : (
                            <span>QUEUED</span>
                          )}
                          <span>{shortTime(asset.duration_ms)}</span>
                        </div>
                        <strong>{asset.name}</strong>
                        <div className="item-actions">
                          <button
                            aria-label={`Preview ${asset.name}`}
                            onClick={() => setPreview(asset)}
                          >
                            <Play size={12} />
                            Preview
                          </button>
                          <button
                            disabled={!ready || busy}
                            onClick={() =>
                              void act("/take", "POST", { item_id: item.id })
                            }
                          >
                            Take now
                            <ArrowUpRight size={12} />
                          </button>
                          <div className="spacer" />
                          <button
                            aria-label={`Move ${asset.name} up`}
                            disabled={index === 0 || busy}
                            onClick={() => void move(index, -1)}
                          >
                            <ArrowUp size={13} />
                          </button>
                          <button
                            aria-label={`Move ${asset.name} down`}
                            disabled={
                              index === channel.queue.length - 1 || busy
                            }
                            onClick={() => void move(index, 1)}
                          >
                            <ArrowDown size={13} />
                          </button>
                          <button
                            aria-label={`Remove ${asset.name}`}
                            disabled={busy}
                            onClick={() =>
                              void act(
                                `/queue/${item.id}`,
                                "DELETE",
                                undefined,
                                "Removed from rundown",
                              )
                            }
                          >
                            <X size={13} />
                          </button>
                        </div>
                      </div>
                    </article>
                  );
                })
              ) : (
                <div className="empty">
                  <Layers size={28} />
                  <h3>Your next moment starts here</h3>
                  <p>
                    Add a clip from the media library.
                    <br />
                    The channel stays live between clips.
                  </p>
                  <button
                    className="button secondary"
                    disabled={!videos.length || busy || !ready}
                    onClick={() => void demo()}
                  >
                    <Plus size={14} />
                    Queue all media
                  </button>
                </div>
              )}
            </div>
            <div className="rundown-footer">
              <button
                className="button primary"
                disabled={!ready || busy || !channel?.queue.length}
                onClick={() => void act("/take", "POST", {})}
              >
                <SkipForward size={16} /> Take next
              </button>
              <button
                className="quiet"
                disabled={
                  !ready || busy || (!current && !channel?.queue.length)
                }
                onClick={() =>
                  void act(
                    "/clear",
                    "POST",
                    undefined,
                    "Rundown cleared; standby slate is on air",
                  )
                }
              >
                Clear to slate
              </button>
            </div>
          </section>
        </div>
        <div className="lower-grid">
          <section className="panel library">
            <div className="panel-heading">
              <div>
                <Film size={17} />
                <h2>Media library</h2>
                <span className="count">{assets.length}</span>
              </div>
              <button
                className="button secondary compact"
                disabled={uploading || !ready}
                onClick={() => fileInput.current?.click()}
              >
                <Upload size={14} />
                {uploading ? "Uploading…" : "Upload media"}
              </button>
              <input
                ref={fileInput}
                className="hidden"
                type="file"
                accept=".mp4,.png,.jpg,.jpeg"
                multiple
                onChange={(e) => void upload(e.target.files)}
              />
            </div>
            <div className="library-tabs">
              <button
                className={tab === "video" ? "selected" : ""}
                onClick={() => setTab("video")}
              >
                Video clips <span>{videos.length}</span>
              </button>
              <button
                className={tab === "image" ? "selected" : ""}
                onClick={() => setTab("image")}
              >
                Ad artwork <span>{images.length}</span>
              </button>
              <span className="muted small">
                {tab === "video"
                  ? "H.264 / AAC · MP4"
                  : "PNG / JPEG · 1280 × 720 recommended"}
              </span>
            </div>
            <div className="asset-grid">
              {(tab === "video" ? videos : images).map((asset, index) => (
                <article className="asset" key={asset.id}>
                  <button
                    className={`asset-thumb tint-${index % 3}`}
                    onClick={() => setPreview(asset)}
                    aria-label={`Preview ${asset.name}`}
                  >
                    {asset.kind === "image" ? (
                      <img src={`/media/${asset.id}.img`} alt={asset.name} />
                    ) : (
                      <>
                        <Film size={26} />
                        <span className="asset-number">
                          {String(index + 1).padStart(2, "0")}
                        </span>
                        <span className="asset-duration">
                          {shortTime(asset.duration_ms)}
                        </span>
                      </>
                    )}
                  </button>
                  <div className="asset-description">
                    <strong title={asset.name}>{asset.name}</strong>
                    <span>
                      {asset.width} × {asset.height} ·{" "}
                      {asset.kind === "video" ? "H.264" : "ARTWORK"}
                    </span>
                  </div>
                  <button
                    className="add-asset"
                    disabled={!ready || busy}
                    onClick={() =>
                      asset.kind === "video"
                        ? void act(
                            "/queue",
                            "POST",
                            { asset_id: asset.id },
                            "Added to rundown",
                          )
                        : (setArt(asset.id),
                          setNotice("Artwork selected for the next banner"))
                    }
                  >
                    <Plus size={14} />
                    {asset.kind === "video" ? "Add to rundown" : "Use artwork"}
                  </button>
                </article>
              ))}
              {!(tab === "video" ? videos : images).length && (
                <div className="empty library-empty">
                  <Upload size={25} />
                  <h3>
                    {tab === "video"
                      ? "Bring your own footage"
                      : "Make room for your brand"}
                  </h3>
                  <p>
                    {tab === "video"
                      ? "Upload an H.264 / AAC MP4 to get started."
                      : "Upload artwork for the bottom and right-side ad area."}
                  </p>
                </div>
              )}
            </div>
          </section>
          <section className="panel ad-panel">
            <div className="panel-heading">
              <div>
                <span className="l-icon" />
                <h2>L-band insertion</h2>
              </div>
              <span
                className={channel?.banner ? "badge active" : "muted small"}
              >
                {channel?.banner ? "ON AIR" : "READY WHEN YOU ARE"}
              </span>
            </div>
            <div className="ad-content">
              <div className="ad-preview">
                <div className="mini-program">
                  <Radio size={23} />
                  <span>PROGRAM CONTINUES</span>
                </div>
                <span className="mini-side">
                  YOUR
                  <br />
                  BRAND
                </span>
                <span className="mini-bottom">
                  {title || "Your message here"}
                </span>
              </div>
              <div className="ad-settings">
                <label>
                  Banner message
                  <input
                    value={title}
                    maxLength={60}
                    onChange={(e) => setTitle(e.target.value)}
                  />
                </label>
                <div className="input-row">
                  <label>
                    Duration <span className="muted">seconds</span>
                    <input
                      type="number"
                      value={duration}
                      min={1}
                      max={600}
                      onChange={(e) => setDuration(Number(e.target.value))}
                    />
                  </label>
                  <label>
                    Artwork
                    <select
                      value={art}
                      onChange={(e) => setArt(e.target.value)}
                    >
                      <option value="">Default teal</option>
                      {images.map((a) => (
                        <option key={a.id} value={a.id}>
                          {a.name}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              </div>
            </div>
            <div className="ad-footer">
              <div className="muted small">
                <Clock3 size={13} />
                {channel?.banner
                  ? `${Math.ceil(channel.banner.remaining_ms / 1000)}s remaining · auto restore`
                  : "Engine-timed removal. Program audio continues."}
              </div>
              <div className="ad-buttons">
                {channel?.banner && (
                  <button
                    className="button secondary"
                    disabled={busy}
                    onClick={() =>
                      void act("/banner", "DELETE", undefined, "Banner removed")
                    }
                  >
                    Remove now
                  </button>
                )}
                <button
                  className="button primary"
                  disabled={!ready || busy}
                  onClick={() =>
                    void act(
                      "/banner",
                      "POST",
                      {
                        duration_ms: duration * 1000,
                        title,
                        asset_id: art || null,
                      },
                      "Banner is on air",
                    )
                  }
                >
                  <Play size={14} />
                  {channel?.banner ? "Replace banner" : "Take banner"}
                </button>
              </div>
            </div>
          </section>
        </div>
        <div className="bottom-grid">
          <section className="panel caption-panel">
            <div className="panel-heading">
              <div>
                <Captions size={18} />
                <h2>Caption cue</h2>
              </div>
              <span className="muted small">WEBVTT · ENGLISH</span>
            </div>
            <form
              onSubmit={async (e) => {
                e.preventDefault();
                const result = await act(
                  `/queue/${captionItem}/captions`,
                  "PUT",
                  [
                    {
                      start_ms: cueStart * 1000,
                      end_ms: cueEnd * 1000,
                      text: cueText,
                    },
                  ],
                  "Caption cue saved",
                );
                if (result) setCueText("");
              }}
            >
              <select
                aria-label="Caption target"
                value={captionItem}
                onChange={(e) => setCaptionItem(e.target.value)}
                required
              >
                <option value="">Choose an on-air or upcoming clip</option>
                {items.map((i) => (
                  <option key={i.id} value={i.id}>
                    {channel?.assets[i.asset_id]?.name}
                    {i.id === current?.item.id ? " (on air)" : ""}
                  </option>
                ))}
              </select>
              <div className="cue-row">
                <label>
                  Start (s)
                  <input
                    type="number"
                    min={0}
                    step="0.1"
                    value={cueStart}
                    onChange={(e) => setCueStart(Number(e.target.value))}
                  />
                </label>
                <label>
                  End (s)
                  <input
                    type="number"
                    min={0}
                    step="0.1"
                    value={cueEnd}
                    onChange={(e) => setCueEnd(Number(e.target.value))}
                  />
                </label>
                <label className="cue-text">
                  Caption text
                  <input
                    required
                    maxLength={1000}
                    value={cueText}
                    placeholder="What should viewers read?"
                    onChange={(e) => setCueText(e.target.value)}
                  />
                </label>
                <button
                  className="button secondary"
                  disabled={
                    !ready ||
                    busy ||
                    !captionItem ||
                    !items.some((i) => i.id === captionItem)
                  }
                >
                  <Plus size={14} />
                  Save cue
                </button>
              </div>
              <p className="muted small">
                Times are relative to the clip. Saving replaces its future cues;
                transmitted captions stay unchanged.
              </p>
            </form>
          </section>
          <PublishPanel state={channel?.publish} ready={ready} />
          <section className="panel events">
            <div className="panel-heading">
              <div>
                <Circle size={13} />
                <h2>Channel activity</h2>
              </div>
              <span className="muted small">LIVE EVENTS</span>
            </div>
            <div className="event-list">
              {channel?.events
                .slice(-5)
                .reverse()
                .map((e) => (
                  <div className="event" key={e.sequence}>
                    <span
                      className={
                        e.kind.includes("failed")
                          ? "event-dot bad"
                          : "event-dot"
                      }
                    />
                    <span>{e.message}</span>
                    <time>{shortTime(e.program_ms)}</time>
                  </div>
                ))}
            </div>
          </section>
        </div>
        <footer className="page-footer">
          <span>
            <Radio size={13} /> RUST PLAYOUT{" "}
            <span className="muted">/ Native media engine</span>
          </span>
          <span>
            {channel?.late_frames ?? 0} late frames <span>·</span>{" "}
            {channel?.underrun_frames ?? 0} source underruns <span>·</span>{" "}
            Local session
          </span>
        </footer>
      </main>
      {notice && (
        <div className="toast" role="status">
          <Check size={16} />
          {notice}
          <button
            aria-label="Dismiss notification"
            onClick={() => setNotice("")}
          >
            <X size={14} />
          </button>
        </div>
      )}
      {preview && (
        <div className="modal-backdrop" onClick={() => setPreview(null)}>
          <section className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="panel-heading">
              <h2>{preview.name}</h2>
              <button
                aria-label="Close preview"
                onClick={() => setPreview(null)}
              >
                <X />
              </button>
            </div>
            {preview.kind === "video" ? (
              <video
                src={`/media/${preview.id}.mp4`}
                controls
                autoPlay
                muted
                playsInline
              />
            ) : (
              <img src={`/media/${preview.id}.img`} alt={preview.name} />
            )}
            <p>Source preview · independent of the live program</p>
          </section>
        </div>
      )}
      {apiOpen && (
        <div className="modal-backdrop" onClick={() => setApiOpen(false)}>
          <section
            className="modal api-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="panel-heading">
              <h2>Control API</h2>
              <button
                aria-label="Close API reference"
                onClick={() => setApiOpen(false)}
              >
                <X />
              </button>
            </div>
            <p>
              HTTP commands control the engine. WebSocket snapshots carry
              applied events and program time.
            </p>
            <pre>{`GET    /api/state\nWS     /api/events\nPOST   /api/assets             multipart file\nPOST   /api/queue              { asset_id, after_id? }\nPUT    /api/queue/order        { ids: [...] }\nDELETE /api/queue/:id\nPOST   /api/take               { item_id? }\nPOST   /api/banner             { title, duration_ms, asset_id? }\nDELETE /api/banner\nPUT    /api/volume             { value: 0..2 }\nPUT    /api/queue/:id/captions  [{ start_ms, end_ms, text }]\nPOST   /api/clear\nPOST   /api/publish            { server_url, stream_key }\nDELETE /api/publish`}</pre>
            <p>
              Bound to localhost. The engine continues when this tab closes.
            </p>
          </section>
        </div>
      )}
    </>
  );
}
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
