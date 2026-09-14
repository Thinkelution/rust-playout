"""Exercise the real HTTP service in an isolated temporary data directory.

Run after cargo build --release: python3 tests/api_smoke.py
No third-party Python packages or ffmpeg executable are required.
"""
import json
import struct
import zlib
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def run():
    with tempfile.TemporaryDirectory(prefix="playout-api-") as temp:
        with socket.socket() as port_socket:
            port_socket.bind(("127.0.0.1", 0))
            port = port_socket.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        env = dict(os.environ, PLAYOUT_PORT=str(port), PLAYOUT_DATA=temp)
        log = open(Path(temp) / "service.log", "w+")
        process = subprocess.Popen([str(ROOT / "target/release/rust-playout"), "--demo"], cwd=temp, env=env, stdout=log, stderr=log)

        def request(path, method="GET", body=None, headers=None, expected=200):
            data = json.dumps(body).encode() if body is not None else None
            req = urllib.request.Request(base + path, data=data, method=method, headers={"Content-Type": "application/json", **(headers or {})})
            try:
                response = urllib.request.urlopen(req, timeout=8)
            except urllib.error.HTTPError as error:
                response = error
            assert response.code == expected, (path, response.code, response.read())
            raw = response.read()
            return json.loads(raw) if raw.startswith(b"{") else raw

        def until(predicate):
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                assert process.poll() is None, "service exited"
                try:
                    state = request("/api/state")
                    if predicate(state):
                        return state
                except urllib.error.URLError:
                    pass
                time.sleep(0.05)
            raise AssertionError("condition did not become true")

        def upload(filename, content, expected=200):
            boundary = "playout-smoke-boundary"
            body = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{filename}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode()
                    + content + f'\r\n--{boundary}--\r\n'.encode())
            req = urllib.request.Request(base + "/api/assets", data=body, headers={"Content-Type": f"multipart/form-data; boundary={boundary}"})
            try:
                response = urllib.request.urlopen(req, timeout=8)
            except urllib.error.HTTPError as error:
                response = error
            assert response.code == expected, (response.code, response.read())
            return json.loads(response.read())

        try:
            initial = until(lambda s: s["status"] == "live")
            html = request("/")
            assert b"<div id=\"root\">" in html
            import re
            script = re.search(rb'src="([^"]+\.js)"', html).group(1).decode()
            assert b"Start channel" in request(script)
            request("/missing-file.js", expected=404)
            assert len(initial["assets"]) == 3
            request("/api/publish", "POST", {"server_url": "file:///tmp/output", "stream_key": "secret-smoke-key"}, expected=400)
            # A server that accepts TCP but never completes RTMP must not stall the channel.
            with socket.socket() as stalled:
                stalled.bind(("127.0.0.1", 0))
                stalled.listen(1)
                stalled.settimeout(5)
                request("/api/publish", "POST", {"server_url": f"rtmp://127.0.0.1:{stalled.getsockname()[1]}/live", "stream_key": "secret-smoke-key"})
                connection, _ = stalled.accept()
                with connection:
                    running = until(lambda s: s["program_ms"] > initial["program_ms"] + 1000)
                    assert running["publish"]["status"] == "connecting"
                    assert "secret-smoke-key" not in json.dumps(running)
                    request("/api/volume", "PUT", {"value": 1})
                    request("/api/publish", "DELETE")
                    until(lambda s: s["publish"]["status"] == "idle")
            log.flush()
            assert "secret-smoke-key" not in Path(temp, "service.log").read_text()

            request("/api/queue", "POST", {"asset_id": "missing"}, expected=400)
            request("/api/volume", "PUT", {"value": -1}, expected=400)
            request("/api/banner", "POST", {"title": "Bad", "duration_ms": 0}, expected=400)
            request("/api/clear", "POST", headers={"Origin": "https://untrusted.invalid"}, expected=403)
            request("/api/state", headers={"Host": "untrusted.invalid"}, expected=403)
            uploaded = upload("operator-test.mp4", (Path(temp) / "media/demo-1.mp4").read_bytes())
            assert uploaded["kind"] == "video" and uploaded["width"] == 1280
            upload("broken.mp4", b"not a media container", expected=400)
            def png_chunk(kind, value):
                return struct.pack(">I", len(value)) + kind + value + struct.pack(">I", zlib.crc32(kind + value))
            png = (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 2, 0, 0, 0))
                   + png_chunk(b"IDAT", zlib.compress((b"\x00" + bytes([30, 90, 65]) * 64) * 64)) + png_chunk(b"IEND", b""))
            artwork = upload("sponsor.png", png)
            assert artwork["kind"] == "image"
            a = request("/api/queue", "POST", {"asset_id": "demo-1"})["result"]["item_id"]
            until(lambda s: s["current"] and s["current"]["item"]["id"] == a)
            b = request("/api/queue", "POST", {"asset_id": "demo-2"})["result"]["item_id"]
            c = request("/api/queue", "POST", {"asset_id": "demo-3", "after_id": a})["result"]["item_id"]
            until(lambda s: [i["id"] for i in s["queue"]] == [c, b])
            request("/api/queue/order", "PUT", {"ids": [b, b]}, expected=400)
            request("/api/queue/order", "PUT", {"ids": [b, c]})
            request(f"/api/queue/{c}/captions", "PUT", [{"start_ms": 2000, "end_ms": 8000, "text": "API test caption"}])
            request(f"/api/queue/{b}", "DELETE")
            taken = request("/api/take", "POST", {"item_id": c})
            assert taken["result"]["pending"]
            until(lambda s: any(e["kind"] == "source_taken" and e["command_id"] == taken["command_id"] for e in s["events"]))
            request("/api/banner", "POST", {"title": "HTTP TEST", "duration_ms": 1000, "asset_id": artwork["id"]})
            until(lambda s: s["banner"] is not None)
            ended = until(lambda s: s["banner"] is None)
            assert any(e["kind"] == "banner_expired" for e in ended["events"])
            assert ended["session_id"] == initial["session_id"]
            until(lambda s: s["program_ms"] >= 4500)
            manifest = request(initial["output_url"])
            assert b"SUBTITLES" in manifest and b"media.m3u8" in manifest
            media = request(initial["output_url"].replace("master.m3u8", "media.m3u8"))
            assert b"#EXTINF:2.000000" in media
            request("/api/clear", "POST")
            until(lambda s: s["current"] is None and not s["queue"])
            request("/api/queue", "POST", {"asset_id": "demo-1"})
            until(lambda s: s["current"] is not None)
            queued = request("/api/queue", "POST", {"asset_id": "demo-2"})
            before_stop = request("/api/state")
            request("/api/channel/stop", "POST")
            stopped = until(lambda s: s["status"] == "stopped")
            assert stopped["current"] is None and stopped["banner"] is None
            assert len(stopped["queue"]) == 1
            time.sleep(0.4)
            assert request("/api/state")["program_ms"] == stopped["program_ms"]
            for name in ["media.m3u8", "captions.m3u8"]:
                assert b"#EXT-X-ENDLIST" in request(before_stop["output_url"].replace("master.m3u8", name))
            request("/api/take", "POST", {}, expected=400)
            request("/api/publish", "POST", {"server_url": "rtmp://127.0.0.1:1/live", "stream_key": "test"}, expected=400)
            request("/api/channel/stop", "POST")
            request("/api/channel/start", "POST")
            restarted = until(lambda s: s["status"] == "live" and s["current"] is not None)
            assert restarted["session_id"] != stopped["session_id"]
            assert restarted["program_ms"] < stopped["program_ms"]
            assert restarted["current"]["item"]["asset_id"] == "demo-2"
            assert restarted["publish"]["status"] == "idle"
            request("/api/channel/start", "POST")
            assert request("/api/state")["session_id"] == restarted["session_id"]
            until(lambda s: s["program_ms"] > 2500)
            assert b"#EXTINF" in request(restarted["output_url"].replace("master.m3u8", "media.m3u8"))
            with socket.socket() as stalled:
                stalled.bind(("127.0.0.1", 0))
                stalled.listen(1)
                stalled.settimeout(5)
                request("/api/publish", "POST", {"server_url": f"rtmp://127.0.0.1:{stalled.getsockname()[1]}/live", "stream_key": "test"})
                connection, _ = stalled.accept()
                with connection:
                    request("/api/channel/stop", "POST")
                    stopped = until(lambda s: s["status"] == "stopped")
                    assert stopped["publish"]["status"] == "idle"
            print("PASS: live API, MP4/PNG uploads, invalid media rejection, host/origin checks, reorder, captions, take events, timed artwork banner, HLS, clear-to-slate, channel stop/start and retained rundown")
        except Exception:
            log.flush()
            log.seek(0)
            print(log.read()[-5000:])
            raise
        finally:
            process.send_signal(signal.SIGINT)
            try:
                process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                raise AssertionError("graceful shutdown timed out")
            log.close()
        assert process.returncode == 0, f"service exit: {process.returncode}"


if __name__ == "__main__":
    run()
