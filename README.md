# SyncPlay Clone

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
* `libmpv` library installed on the system (e.g. `sudo apt install libmpv-dev` on Debian/Ubuntu, or via `mpv` on Windows).

## Running
To start the application:
```bash
pip install -r requirements.txt
python client.py
```
