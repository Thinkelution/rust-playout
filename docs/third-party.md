# Third-party components

Rust Playout uses Rust and JavaScript dependencies tracked in Cargo.lock and
web/package-lock.json in the tagged source release. Those dependencies retain
their respective licenses. The web control room includes React, Hls.js and
Lucide icons; its compiled code is embedded in the executable.

Media processing dynamically links FFmpeg libraries installed separately by the
user. FFmpeg and enabled codecs such as libx264 retain their own licensing terms.
This archive does not redistribute FFmpeg or its native dependency libraries.
See https://ffmpeg.org/legal.html and the installed distribution's license notices.

This notice does not grant additional rights in third-party components.

The FFmpeg build used to link this alpha reports "GPL version 3 or later".
The repository owner must choose compatible project licensing before the binary
release is published. Excluding native libraries from the archive does not remove
this licensing consideration.
