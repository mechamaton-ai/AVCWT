# AVCWT (Rust Edition)

A fast, cross-platform desktop application written in **Rust** for synchronizing media playback across multiple users over the network.

## Features
* **Cross-platform** (Windows & Linux) single-binary architecture (`AVCWT` / `AVCWT.exe`).
* **Native Windows `.exe` Support** — compiled with `windows_subsystem = "windows"` so double-clicking `AVCWT.exe` in Windows Explorer opens the GUI directly with no console window, and automatically fetches `mpv-2.dll` and `yt-dlp.exe` on first launch if they are not already alongside `AVCWT.exe`.
* **Embedded `libmpv` video player** inside a native dark-mode `eframe` / `egui` GUI with floating translucent On-Screen Controls (OSC).
* **Built-in WebSocket Sync Server** — host a session directly from the GUI (`Settings` -> `Host Server`) or headlessly from the CLI (`AVCWT --server`).
* **Synchronized Playback** — synchronizes `play`, `pause`, `seek`, `playlist`, `user_count`, and periodic `time_sync` drift correction across connected peers.
* **YouTube & Livestream Support** — integrates with `yt-dlp` and automatically detects live streams to disable time-seek jitter.
* **Track & Cache Management** — switch audio and subtitle tracks on the fly and configure the demuxer network cache size (`settings.ini`).

## Building & Running

### Windows (`AVCWT.exe`)
Build the standalone Windows GUI executable:
```powershell
cargo build --release
```
This produces `target\release\AVCWT.exe`. You can **double-click `AVCWT.exe`** directly in File Explorer to launch the app (it will automatically download `mpv-2.dll` and `yt-dlp.exe` next to itself on first run if they are not already in the folder).

### Linux
Install system dependencies and build:
```bash
sudo apt install libmpv-dev yt-dlp ffmpeg
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
