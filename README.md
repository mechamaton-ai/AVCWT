# AVCWT

A Python-based desktop application for synchronizing media playback across multiple users over the network. 

## Features
* Cross-platform (Windows & Linux).
* Embedded MPV player inside a modern PyQt6 GUI.
* Built-in server functionality - host a session directly from the client.
* Synchronizes `play`, `pause`, and `seek` events.
* Manage a local list of video files to play.

## Requirements
* Python 3.8+
* PyQt6
* python-mpv
* websockets
* `libmpv` library installed on the system:
  * **Linux (Debian/Ubuntu):** `sudo apt install libmpv-dev`
  * **Windows:** Download the latest `mpv-dev` library from [here](https://sourceforge.net/projects/mpv-player-windows/files/libmpv/) (e.g., `mpv-dev-x86_64-....7z`). Extract the archive and copy the `mpv-2.dll` (or `mpv-1.dll`) file directly into the same folder as `client.py`.

## Running
To start the application:
```bash
pip install -r requirements.txt
python client.py
```

## Building Standalone Executables
To create a standalone executable that doesn't require Python, run `build.bat` on Windows or `./build.sh` on Linux/macOS.
