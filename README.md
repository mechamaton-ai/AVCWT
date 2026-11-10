# AVCWT (Rust Edition)

A fast, cross-platform desktop application written in **Rust** for synchronizing media playback across multiple users over the network.

## Features
* **Cross-platform** (Windows & Linux) single-binary architecture (`avcwt`).
* **Embedded `libmpv` video player** inside a native dark-mode `eframe` / `egui` GUI with floating translucent On-Screen Controls (OSC).
* **Built-in WebSocket Sync Server** — host a session directly from the GUI (`Settings` -> `Host Server`) or headlessly from the CLI (`avcwt --server`).
* **Synchronized Playback** — synchronizes `play`, `pause`, `seek`, `playlist`, `user_count`, and periodic `time_sync` drift correction across connected peers.
* **YouTube & Livestream Support** — integrates with `yt-dlp` and automatically detects live streams to disable time-seek jitter.
* **Track & Cache Management** — switch audio and subtitle tracks on the fly and configure the demuxer network cache size (`settings.ini`).

## Requirements
* **Rust 1.75+** (`cargo`, `rustc`)
* **`libmpv`** shared library installed on the system:
  * **Linux (Debian/Ubuntu):**
    ```bash
    sudo apt install libmpv-dev yt-dlp ffmpeg
    ```
  * **Windows:**
    Download the latest `mpv-dev-x86_64` archive from [shinchiro/mpv-winbuild-cmake](https://sourceforge.net/projects/mpv-player-windows/files/libmpv/) and place `mpv-2.dll` (and optionally `yt-dlp.exe`) in the same folder as `avcwt.exe`.

## Building & Running

### Launch GUI Client
```bash
cargo run --release
```

### Run Standalone Headless Sync Server
```bash
cargo run --release -- --server --host 0.0.0.0 --port 8765
```

### Run Automated Tests
```bash
cargo test
```
