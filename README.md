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
