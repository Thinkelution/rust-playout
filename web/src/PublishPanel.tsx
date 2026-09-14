import { useState } from "react";
import { Radio } from "lucide-react";
export type PublishState = {
  status: string;
  destination: string | null;
  message: string;
  packets_sent: number;
  bytes_sent: number;
};
export function PublishPanel({
  state,
  ready,
}: {
  state?: PublishState;
  ready: boolean;
}) {
  const [server, setServer] = useState("rtmps://a.rtmps.youtube.com:443/live2");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const active =
    !!state &&
    ["connecting", "waiting_keyframe", "publishing", "stopping"].includes(
      state.status,
    );
  async function control(stop: boolean) {
    setBusy(true);
    setError("");
    try {
      const response = await fetch("/api/publish", {
        method: stop ? "DELETE" : "POST",
        headers: { "Content-Type": "application/json" },
        ...(stop
          ? {}
          : { body: JSON.stringify({ server_url: server, stream_key: key }) }),
      });
      const result = await response.json();
      if (!response.ok)
        throw new Error(result.error || "Publisher request failed");
      if (!stop) setKey("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Publisher request failed");
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel publish-panel">
      <div className="panel-heading">
        <div>
          <Radio size={16} />
          <h2>YouTube / RTMP output</h2>
        </div>
        <span className="muted small">
          {state?.status.replaceAll("_", " ").toUpperCase() ?? "IDLE"}
        </span>
      </div>
      <form
        className="publish-form"
        onSubmit={(e) => {
          e.preventDefault();
          void control(false);
        }}
      >
        <label>
          Streaming server
          <input
            aria-label="Streaming server"
            value={server}
            disabled={active || busy}
            onChange={(e) => setServer(e.target.value)}
            required
            spellCheck={false}
          />
        </label>
        <label>
          Stream key
          <input
            aria-label="Stream key"
            type="password"
            value={key}
            disabled={active || busy}
            onChange={(e) => setKey(e.target.value)}
            required
            autoComplete="off"
            placeholder="Paste your YouTube stream key"
          />
        </label>
        <p className="muted small">
          Copy the server and key from YouTube Live Control Room. Starting sends
          the current program, including ads. YouTube may go live automatically
          if auto-start is enabled.
        </p>
        {active ? (
          <button
            className="button secondary"
            type="button"
            disabled={busy || !ready || state?.status === "stopping"}
            onClick={() => void control(true)}
          >
            Stop publishing
          </button>
        ) : (
          <button
            className="button primary"
            disabled={busy || !ready || !key.trim()}
          >
            {busy ? "Starting…" : "Start publishing"}
          </button>
        )}
        <p role="status">
          {state?.message ?? "Not publishing"}
          {state?.bytes_sent
            ? ` · ${(state.bytes_sent / 1_000_000).toFixed(1)} MB sent`
            : ""}
        </p>
        {error && <p role="alert">{error}</p>}
        <p className="muted small">
          720p · 30 fps · H.264 + AAC. HLS continues independently. Captions
          currently appear in HLS only. Keys are cleared after starting and are
          not saved. Confirm reception and live status in YouTube.
        </p>
      </form>
    </section>
  );
}
