import sys
import os
import locale
from PyQt6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout, 
                             QHBoxLayout, QPushButton, QListWidget, QFileDialog, 
                             QLabel, QLineEdit)
from PyQt6.QtCore import Qt, QThread, pyqtSignal

# Add the script directory to the PATH to find mpv-*.dll on Windows
os.environ["PATH"] = os.path.dirname(os.path.abspath(__file__)) + os.pathsep + os.environ["PATH"]
import mpv

import threading
import asyncio
import websockets
import json
from server import run_server

class WebSocketThread(QThread):
    message_received = pyqtSignal(dict)
    connection_status = pyqtSignal(bool)

    def __init__(self):
        super().__init__()
        self.uri = ""
        self.loop = None
        self.ws = None
        self._is_running = False

    def connect_to(self, uri):
        self.uri = uri
        self.start()

    def run(self):
        self.loop = asyncio.new_event_loop()
        asyncio.set_event_loop(self.loop)
        self.loop.run_until_complete(self._connect_and_listen())

    async def _connect_and_listen(self):
        try:
            async with websockets.connect(self.uri) as websocket:
                self.ws = websocket
                self._is_running = True
                self.connection_status.emit(True)
                
                while self._is_running:
                    try:
                        message = await asyncio.wait_for(websocket.recv(), timeout=1.0)
                        data = json.loads(message)
                        self.message_received.emit(data)
                    except asyncio.TimeoutError:
                        continue
                    except websockets.exceptions.ConnectionClosed:
                        break
        except Exception as e:
            print(f"Connection failed: {e}")
        finally:
            self._is_running = False
            self.ws = None
            self.connection_status.emit(False)

    def send_message(self, data):
        if self.ws and self._is_running and self.loop:
            asyncio.run_coroutine_threadsafe(
                self.ws.send(json.dumps(data)), self.loop
            )

    def stop(self):
        self._is_running = False
        if self.loop:
            self.loop.stop()

class SyncPlayClient(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("SyncPlay Clone")
        self.resize(800, 600)

        # Main Widget and Layout
        main_widget = QWidget()
        self.setCentralWidget(main_widget)
        main_layout = QHBoxLayout(main_widget)

        # Left Panel (Video Player Area)
        self.video_container = QWidget()
        self.video_container.setStyleSheet("background-color: black;")
        main_layout.addWidget(self.video_container, stretch=3)
        
        # Initialize MPV
        locale.setlocale(locale.LC_NUMERIC, 'C') # required by mpv
        self.player = mpv.MPV(wid=str(int(self.video_container.winId())),
                              input_default_bindings=True,
                              input_vo_keyboard=True,
                              osc=True)

        # Right Panel (Controls & Playlist)
        right_panel = QWidget()
        right_layout = QVBoxLayout(right_panel)
        main_layout.addWidget(right_panel, stretch=1)

        # --- Playlist / File Management ---
        self.file_list = QListWidget()
        right_layout.addWidget(QLabel("Playlist"))
        right_layout.addWidget(self.file_list)

        btn_layout = QHBoxLayout()
        self.btn_add_file = QPushButton("Add File")
        self.btn_remove_file = QPushButton("Remove File")
        btn_layout.addWidget(self.btn_add_file)
        btn_layout.addWidget(self.btn_remove_file)
        right_layout.addLayout(btn_layout)
        
        self.btn_add_file.clicked.connect(self.add_file)
        self.btn_remove_file.clicked.connect(self.remove_file)
        self.file_list.itemDoubleClicked.connect(self.play_file)

        # --- Network Controls ---
        right_layout.addSpacing(20)
        right_layout.addWidget(QLabel("Network Connection"))
        
        self.ip_input = QLineEdit()
        self.ip_input.setPlaceholderText("ws://localhost:8765")
        self.ip_input.setText("ws://localhost:8765")
        right_layout.addWidget(self.ip_input)

        net_btn_layout = QHBoxLayout()
        self.btn_connect = QPushButton("Connect")
        self.btn_host = QPushButton("Host Server")
        net_btn_layout.addWidget(self.btn_connect)
        net_btn_layout.addWidget(self.btn_host)
        right_layout.addLayout(net_btn_layout)
        
        self.btn_host.clicked.connect(self.host_server)
        self.btn_connect.clicked.connect(self.connect_to_server)
        
        self.server_thread = None
        self.ws_thread = WebSocketThread()
        self.ws_thread.message_received.connect(self.handle_network_message)
        self.ws_thread.connection_status.connect(self.handle_connection_status)
        
        self.ignore_next_seek = False
        self.ignore_next_pause = False
        
        # Bind MPV events
        self.player.observe_property('pause', self.on_mpv_pause)
        self.player.observe_property('time-pos', self.on_mpv_time_pos)

    def connect_to_server(self):
        uri = self.ip_input.text()
        if uri:
            self.btn_connect.setEnabled(False)
            self.btn_connect.setText("Connecting...")
            self.ws_thread.connect_to(uri)

    def handle_connection_status(self, connected):
        if connected:
            self.btn_connect.setText("Connected")
            self.btn_connect.setStyleSheet("background-color: green; color: white;")
        else:
            self.btn_connect.setEnabled(True)
            self.btn_connect.setText("Connect")
            self.btn_connect.setStyleSheet("")

    def handle_network_message(self, data):
        action = data.get('action')
        
        if action == 'pause':
            self.ignore_next_pause = True
            self.player.pause = data.get('state', True)
            
        elif action == 'seek':
            self.ignore_next_seek = True
            position = data.get('position', 0)
            if self.player.time_pos is not None and abs(self.player.time_pos - position) > 1.0:
                self.player.time_pos = position

    def on_mpv_pause(self, _name, value):
        if self.ignore_next_pause:
            self.ignore_next_pause = False
            return
        
        if value is not None:
            self.ws_thread.send_message({
                'action': 'pause',
                'state': value
            })

    def on_mpv_time_pos(self, _name, value):
        if self.ignore_next_seek:
            # We don't reset ignore_next_seek here immediately because time-pos
            # updates constantly. We'll let a small tolerance handle it, or 
            # let the user's manual seek trigger this. 
            pass
            
        # This is a bit tricky: time-pos updates continuously during playback.
        # To avoid flooding the network, we'd normally only send explicit seek events.
        # For simplicity in this clone, we'll only broadcast if there's a big jump.
        if not hasattr(self, '_last_time_pos'):
            self._last_time_pos = value
            
        if value is not None and self._last_time_pos is not None:
            # If difference is more than 1.5 seconds, consider it a seek
            if abs(value - self._last_time_pos) > 1.5:
                if self.ignore_next_seek:
                    self.ignore_next_seek = False
                else:
                    self.ws_thread.send_message({
                        'action': 'seek',
                        'position': value
                    })
        self._last_time_pos = value

    def host_server(self):
        if self.server_thread is None:
            self.server_thread = threading.Thread(target=run_server, daemon=True)
            self.server_thread.start()
            self.btn_host.setEnabled(False)
            self.btn_host.setText("Hosting...")
            self.ip_input.setText("ws://localhost:8765")
            self.connect_to_server()

    def add_file(self):
        file_path, _ = QFileDialog.getOpenFileName(self, "Open Video File", "", "Video Files (*.mp4 *.mkv *.avi *.webm);;All Files (*)")
        if file_path:
            self.file_list.addItem(file_path)

    def remove_file(self):
        selected = self.file_list.selectedItems()
        if not selected:
            return
        for item in selected:
            self.file_list.takeItem(self.file_list.row(item))

    def play_file(self, item):
        self.player.play(item.text())


if __name__ == "__main__":
    app = QApplication(sys.argv)
    window = SyncPlayClient()
    window.show()
    sys.exit(app.exec())
