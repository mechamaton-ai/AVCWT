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

## Running (Easy Method)
We provide scripts to easily set up and launch the application without typing terminal commands.
* **Windows:** Double-click `launch.bat`.
* **Linux/macOS:** Run `./launch.sh`.

*Note for Windows users:* If you see a blue "Windows protected your PC" (SmartScreen) warning when running `launch.bat` or the built `.exe`:
1. Click **More info**.
2. Click **Run anyway**.
Alternatively, you can right-click the file, select **Properties**, check the **Unblock** box at the bottom, and click **Apply**.

## Running (Manual Method)
To start the application manually via terminal:
```bash
pip install -r requirements.txt
python client.py
```

## Building a Standalone Executable
If you want to share the application with friends without them needing to install Python, you can build a standalone executable:
* **Windows:** Double-click `build.bat`. The `.exe` will be generated in the `dist` folder.
* **Linux/macOS:** Run `./build.sh`. The executable will be generated in the `dist` folder.
