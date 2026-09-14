# Rust Playout 0.0.1-alpha — macOS Apple Silicon

This archive contains the native arm64 executable with the web control room
embedded. No Rust, Node.js or source checkout is needed to run it.

## Requirements

- Apple Silicon Mac. This release was tested on macOS 26.5.1 only.
- Homebrew in its standard `/opt/homebrew` location.
- FFmpeg **9** libraries installed with `brew install ffmpeg`.
  Required ABI versions: libavcodec/libavformat 63, libavutil 61,
  libswscale 10 and libswresample 7, including the libx264 encoder.
  A future FFmpeg major upgrade can require a new Rust Playout binary.
- A browser and disk space for uploaded media and HLS output.

Native media libraries are not bundled. This is not a fully standalone or
universal macOS application. Intel Mac, Windows and Linux binaries are not
provided in this release.

## Homebrew installation

On macOS 26 (Tahoe) or newer with Apple Silicon and Homebrew at `/opt/homebrew`:

```sh
brew install thinkelution/tap/rust-playout
rust-playout --demo
```

This uses our [Homebrew tap](https://github.com/Thinkelution/homebrew-tap) and
installs FFmpeg as a dependency. Open http://127.0.0.1:8787. No background process
starts during installation. Update with `brew update` followed by
`brew upgrade thinkelution/tap/rust-playout`; remove with `brew uninstall rust-playout`.
Your data is retained when uninstalling.

## Manual archive installation

1. Download the macOS arm64 `.tar.gz` and `SHA256SUMS` from the same release.
2. In the download directory, verify the archive and extract it:

   ```sh
   shasum -a 256 -c SHA256SUMS
   tar -xzf rust-playout-0.0.1-alpha-macos-arm64.tar.gz
   cd rust-playout-0.0.1-alpha-macos-arm64
   brew install ffmpeg
   ./rust-playout --version
   ./rust-playout --demo
   ```

3. Open http://127.0.0.1:8787 in your browser. Queue the demo clips to try the
   rundown and timed ad controls. Stop/start controls are at the top of the page.
4. Press Ctrl-C in the terminal to close the application. The channel's Stop
   button stops output while keeping the control service running.

The binary has a local ad-hoc signature, not an Apple Developer ID signature or
notarization. macOS may block a downloaded executable. If you trust this release,
use macOS System Settings → Privacy & Security to allow it after the first launch
attempt. Do not disable Gatekeeper globally.

Data is stored in `data/` under the directory from which you launch the binary.
Keep that directory to retain uploaded media. To choose a stable data location:

```sh
PLAYOUT_DATA="$HOME/Library/Application Support/Rust Playout" ./rust-playout
```

`PLAYOUT_PORT` changes the default port, 8787. The service listens only on
localhost and has no account system; do not expose it to the internet.

## Alpha boundaries

One channel, 720p30 output, local H.264/AAC MP4 sources, hard cuts and CPU rendering.
RTMP/RTMPS includes composed video/audio and ads; WebVTT captions remain HLS-only.
Real YouTube ingestion has not yet been verified. The stream key must be entered
again after stopping publishing. Upcoming rundown/captions are session state;
media metadata persists. Old HLS sessions are retained on disk.

The existing profile is H.264 target 2.5 Mbps, AAC stereo 48 kHz at 128 kbps.
This is an early alpha, not a production broadcast system.
