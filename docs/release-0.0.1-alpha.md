# Rust Playout 0.0.1 alpha

First downloadable alpha: a Rust playout engine with an embedded web control room.

## Homebrew (Apple Silicon, macOS 26+)

```sh
brew install thinkelution/tap/rust-playout
rust-playout --demo
```

Homebrew installs the matching release archive and FFmpeg dependency. Requires
Homebrew at `/opt/homebrew`. See the [tap](https://github.com/Thinkelution/homebrew-tap).

## Download and run

Download `rust-playout-0.0.1-alpha-macos-arm64.tar.gz` and `SHA256SUMS`.
**Apple Silicon macOS only; tested on macOS 26.5.1. Requires Homebrew FFmpeg 9
libraries at `/opt/homebrew`.** No Rust, Node.js or source checkout is required.
The binary uses an ad-hoc signature and is not notarized.

See [installation instructions](https://github.com/Thinkelution/rust-playout/blob/v0.0.1-alpha/docs/install-macos.md).

```sh
shasum -a 256 -c SHA256SUMS
tar -xzf rust-playout-0.0.1-alpha-macos-arm64.tar.gz
cd rust-playout-0.0.1-alpha-macos-arm64
brew install ffmpeg
./rust-playout --demo
```

Open http://127.0.0.1:8787.

## Included

- Continuous 720p30 H.264/AAC HLS output, through native libraries rather than a CLI process.
- Live rundown editing, prepared-next playback and hard cuts.
- Channel start/stop, with a fresh output session on restart.
- Timed L-shaped image/text ads, countdown and automatic removal.
- WebVTT captions in HLS.
- RTMP/RTMPS publisher with independent failure handling and masked stream-key entry.
- Demo clips and an embedded browser UI.

![Control room](https://raw.githubusercontent.com/Thinkelution/rust-playout/v0.0.1-alpha/docs/images/control-room.jpg)

## Verification and boundaries

Native tests, API smoke tests, frontend build and Clippy passed. The extracted
release archive was launched outside the source checkout and verified to serve
its embedded UI, generate demos and produce HLS. RTMP was tested with a local
receiver; real YouTube ingestion and successful RTMPS ingestion remain unverified.

One channel, local MP4 inputs, CPU composition, fixed 720p30 profile. No pause,
live inputs, automatic publisher reconnect or YouTube caption forwarding. The
service has no authentication and must remain local. Old HLS sessions need manual
retention management. Native media libraries are not included in the download.

## Open-source license

Rust Playout is licensed under **GPL-3.0-or-later**, without warranty. The binary
archive includes LICENSE and third-party notices. Corresponding project source,
including build scripts and dependency lockfiles, is provided as
`rust-playout-0.0.1-alpha-source.tar.gz` and in the `v0.0.1-alpha` tag.
FFmpeg libraries are installed separately by the user.
