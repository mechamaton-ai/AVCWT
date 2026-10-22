import sys
import os
import locale
from PyQt6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout, 
                             QHBoxLayout, QPushButton, QListWidget, QFileDialog, 
                             QLabel, QLineEdit, QInputDialog, QSlider, QListWidgetItem,
                             QComboBox, QSpinBox)
from PyQt6.QtCore import Qt, QThread, pyqtSignal, QObject, QTimer
from PyQt6.QtGui import QPalette, QColor, QKeySequence, QShortcut

# Add the script directory to the PATH to find mpv-*.dll on Windows
os.environ["PATH"] = os.path.dirname(os.path.abspath(__file__)) + os.pathsep + os.environ["PATH"]
import mpv

import threading
import asyncio
import websockets
import json
from server import run_server

class MpvSignals(QObject):
    time_pos_changed = pyqtSignal(float)
    duration_changed = pyqtSignal(float)
    pause_changed = pyqtSignal(bool)
    track_list_changed = pyqtSignal(list)

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

        # Left Panel (Video Player Area & Controls)
        left_panel = QWidget()
        left_layout = QVBoxLayout(left_panel)
        main_layout.addWidget(left_panel, stretch=3)

        self.video_container = QWidget()
        self.video_container.setStyleSheet("background-color: black;")
        left_layout.addWidget(self.video_container, stretch=1)
        
        # Player Controls Bar
        control_layout = QHBoxLayout()
        self.btn_play_pause = QPushButton("Play")
        self.btn_play_pause.setFixedWidth(60)
        self.lbl_time = QLabel("00:00 / 00:00")
        
        self.slider = QSlider(Qt.Orientation.Horizontal)
        self.slider.setRange(0, 1000)
        self.slider.setEnabled(False)
        
        self.volume_slider = QSlider(Qt.Orientation.Horizontal)
        self.volume_slider.setRange(0, 100)
        self.volume_slider.setValue(100)
        self.volume_slider.setFixedWidth(100)

        self.cb_audio = QComboBox()
        self.cb_audio.setFixedWidth(100)
        self.cb_subtitles = QComboBox()
        self.cb_subtitles.setFixedWidth(100)
        
        self.btn_toggle_sidebar = QPushButton("☰")
        self.btn_toggle_sidebar.setFixedWidth(30)
        
        self.btn_fullscreen = QPushButton("⛶")
        self.btn_fullscreen.setFixedWidth(30)
        
        # We need the control bar widget to be hideable
        self.control_widget = QWidget()
        self.control_widget.setLayout(control_layout)
        left_layout.addWidget(self.control_widget)
        
        control_layout.addWidget(self.btn_play_pause)
        control_layout.addWidget(self.slider)
        control_layout.addWidget(self.lbl_time)
        control_layout.addWidget(self.volume_slider)
        control_layout.addWidget(QLabel("Audio:"))
        control_layout.addWidget(self.cb_audio)
        control_layout.addWidget(QLabel("Subs:"))
        control_layout.addWidget(self.cb_subtitles)
        control_layout.addWidget(self.btn_toggle_sidebar)
        control_layout.addWidget(self.btn_fullscreen)
        
        # Initialize MPV
        locale.setlocale(locale.LC_NUMERIC, 'C') # required by mpv
        self.player = mpv.MPV(wid=str(int(self.video_container.winId())),
                              input_default_bindings=True,
                              input_vo_keyboard=True,
                              osc=True,
                              cache="yes")

        # Right Panel (Controls & Playlist)
        self.right_panel = QWidget()
        right_layout = QVBoxLayout(self.right_panel)
        main_layout.addWidget(self.right_panel, stretch=1)

        # --- Playlist / URL Management ---
        self.file_list = QListWidget()
        right_layout.addWidget(QLabel("Playlist"))
        right_layout.addWidget(self.file_list)

        btn_layout = QHBoxLayout()
        self.btn_add_url = QPushButton("Add URL")
        self.btn_remove_file = QPushButton("Remove")
        btn_layout.addWidget(self.btn_add_url)
        btn_layout.addWidget(self.btn_remove_file)
        right_layout.addLayout(btn_layout)
        
        self.btn_add_url.clicked.connect(self.add_url)
        self.btn_remove_file.clicked.connect(self.remove_file)
        self.file_list.itemDoubleClicked.connect(self.play_file)

        # --- Settings & Network Controls ---
        right_layout.addSpacing(20)
        
        # Cache Setting
        cache_layout = QHBoxLayout()
        cache_layout.addWidget(QLabel("Network Cache (MB):"))
        self.spin_cache = QSpinBox()
        self.spin_cache.setRange(10, 2000)
        self.spin_cache.setValue(150) # Default to 150MB
        cache_layout.addWidget(self.spin_cache)
        right_layout.addLayout(cache_layout)
        
        right_layout.addSpacing(10)
        
        # Network Connection
        net_header_layout = QHBoxLayout()
        net_header_layout.addWidget(QLabel("Network Connection"))
        self.lbl_viewers = QLabel("Viewers: 0")
        self.lbl_viewers.setAlignment(Qt.AlignmentFlag.AlignRight)
        net_header_layout.addWidget(self.lbl_viewers)
        right_layout.addLayout(net_header_layout)
        
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
        
        # Control bar signals
        self.btn_play_pause.clicked.connect(self.toggle_playback)
        self.slider.sliderMoved.connect(self.seek_video)
        self.volume_slider.valueChanged.connect(self.change_volume)
        self.btn_toggle_sidebar.clicked.connect(self.toggle_sidebar)
        self.btn_fullscreen.clicked.connect(self.toggle_fullscreen)
        self.cb_audio.currentIndexChanged.connect(self.change_audio_track)
        self.cb_subtitles.currentIndexChanged.connect(self.change_subtitle_track)
        self.spin_cache.valueChanged.connect(self.update_cache_size)
        
        # Mpv Thread Signals to GUI
        self.mpv_signals = MpvSignals()
        self.mpv_signals.time_pos_changed.connect(self.update_ui_time)
        self.mpv_signals.duration_changed.connect(self.update_ui_duration)
        self.mpv_signals.pause_changed.connect(self.update_ui_pause)
        self.mpv_signals.track_list_changed.connect(self.update_ui_tracks)
        self._current_duration = 0.0

        # Bind MPV events
        self.player.observe_property('pause', self.on_mpv_pause)
        self.player.observe_property('time-pos', self.on_mpv_time_pos)
        self.player.observe_property('duration', self.on_mpv_duration)
        self.player.observe_property('track-list', self.on_mpv_track_list)
        
        # Global Shortcuts
        self.shortcut_space = QShortcut(QKeySequence("Space"), self)
        self.shortcut_space.activated.connect(self.toggle_playback)

    def update_cache_size(self, value_mb):
        bytes_val = value_mb * 1024 * 1024
        # Apply to player properties (python-mpv accesses these directly)
        try:
            self.player.cache = "yes"
            self.player.demuxer_max_bytes = bytes_val
            self.player.demuxer_max_back_bytes = bytes_val
        except AttributeError:
            pass # Failsafe if player is not fully initialized

    def toggle_sidebar(self):
        self.right_panel.setVisible(not self.right_panel.isVisible())

    def toggle_fullscreen(self):
        if self.isFullScreen():
            self.showNormal()
            self.control_widget.setVisible(True)
            self.right_panel.setVisible(True)
        else:
            self.showFullScreen()
            self.control_widget.setVisible(False)
            self.right_panel.setVisible(False)

    def change_volume(self, value):
        self.player.volume = value

    def toggle_playback(self):
        if self.player.pause is not None:
            self.player.pause = not self.player.pause

    def seek_video(self, value):
        if self._current_duration > 0:
            new_pos = (value / 1000.0) * self._current_duration
            self.player.time_pos = new_pos

    def format_time(self, seconds):
        if seconds is None:
            return "00:00"
        m, s = divmod(int(seconds), 60)
        h, m = divmod(m, 60)
        if h > 0:
            return f"{h:02d}:{m:02d}:{s:02d}"
        return f"{m:02d}:{s:02d}"

    def update_ui_time(self, time_pos):
        if self._current_duration > 0:
            self.slider.blockSignals(True)
            self.slider.setValue(int((time_pos / self._current_duration) * 1000))
            self.slider.blockSignals(False)
        self.lbl_time.setText(f"{self.format_time(time_pos)} / {self.format_time(self._current_duration)}")

    def update_ui_duration(self, duration):
        self._current_duration = duration
        if duration > 0:
            self.slider.setEnabled(True)
        else:
            self.slider.setEnabled(False)

    def update_ui_pause(self, is_paused):
        self.btn_play_pause.setText("Play" if is_paused else "Pause")

    def change_audio_track(self, index):
        track_id = self.cb_audio.itemData(index)
        if track_id is None:
            self.player.aid = "no"
        else:
            self.player.aid = track_id

    def change_subtitle_track(self, index):
        track_id = self.cb_subtitles.itemData(index)
        if track_id is None:
            self.player.sid = "no"
        else:
            self.player.sid = track_id

    def update_ui_tracks(self, tracks):
        # We need to temporarily block signals so updating the combo boxes 
        # doesn't trigger a switch command.
        self.cb_audio.blockSignals(True)
        self.cb_subtitles.blockSignals(True)
        
        self.cb_audio.clear()
        self.cb_subtitles.clear()
        
        self.cb_audio.addItem("None", None)
        self.cb_subtitles.addItem("None", None)
        
        audio_idx = 0
        sub_idx = 0
        
        for track in tracks:
            # Type is 'audio', 'sub', or 'video'
            if track.get('type') == 'audio':
                title = track.get('title') or track.get('lang') or f"Audio {track.get('id')}"
                self.cb_audio.addItem(title, track.get('id'))
                if track.get('selected'):
                    audio_idx = self.cb_audio.count() - 1
            elif track.get('type') == 'sub':
                title = track.get('title') or track.get('lang') or f"Sub {track.get('id')}"
                self.cb_subtitles.addItem(title, track.get('id'))
                if track.get('selected'):
                    sub_idx = self.cb_subtitles.count() - 1
                    
        self.cb_audio.setCurrentIndex(audio_idx)
        self.cb_subtitles.setCurrentIndex(sub_idx)
        
        self.cb_audio.blockSignals(False)
        self.cb_subtitles.blockSignals(False)

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
        
        if action == 'user_count':
            self.lbl_viewers.setText(f"Viewers: {data.get('count', 0)}")
            
        elif action == 'pause':
            self.ignore_next_pause = True
            self.player.pause = data.get('state', True)
            
        elif action == 'seek':
            self.ignore_next_seek = True
            position = data.get('position', 0)
            if self.player.time_pos is not None and abs(self.player.time_pos - position) > 1.0:
                self.player.time_pos = position

    def on_mpv_pause(self, _name, value):
        if value is not None:
            self.mpv_signals.pause_changed.emit(value)
            
        if self.ignore_next_pause:
            self.ignore_next_pause = False
            return
        
        if value is not None:
            self.ws_thread.send_message({
                'action': 'pause',
                'state': value
            })

    def on_mpv_duration(self, _name, value):
        if value is not None:
            self.mpv_signals.duration_changed.emit(value)

    def on_mpv_track_list(self, _name, value):
        if value is not None:
            self.mpv_signals.track_list_changed.emit(value)

    def on_mpv_time_pos(self, _name, value):
        if value is not None:
            self.mpv_signals.time_pos_changed.emit(value)
            
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
            # Wait 1s for the server to spin up and bind the port before connecting
            QTimer.singleShot(1000, self.connect_to_server)

    def add_url(self):
        url, ok = QInputDialog.getText(self, "Add URL", "Enter Media URL:")
        if ok and url:
            item = QListWidgetItem(url)
            self.file_list.addItem(item)
            self.file_list.setCurrentItem(item)
            self.play_file(item)

    def remove_file(self):
        selected = self.file_list.selectedItems()
        if not selected:
            return
        for item in selected:
            self.file_list.takeItem(self.file_list.row(item))
            self.player.command('stop')

    def play_file(self, item):
        self.player.pause = True
        self.player.play(item.text())

    def keyPressEvent(self, event):
        if event.key() == Qt.Key.Key_Escape and self.isFullScreen():
            self.toggle_fullscreen()
        else:
            super().keyPressEvent(event)

if __name__ == "__main__":
    app = QApplication(sys.argv)
    
    # Enable Dark Mode
    app.setStyle("Fusion")
    palette = QPalette()
    palette.setColor(QPalette.ColorRole.Window, QColor(53, 53, 53))
    palette.setColor(QPalette.ColorRole.WindowText, Qt.GlobalColor.white)
    palette.setColor(QPalette.ColorRole.Base, QColor(25, 25, 25))
    palette.setColor(QPalette.ColorRole.AlternateBase, QColor(53, 53, 53))
    palette.setColor(QPalette.ColorRole.ToolTipBase, Qt.GlobalColor.black)
    palette.setColor(QPalette.ColorRole.ToolTipText, Qt.GlobalColor.white)
    palette.setColor(QPalette.ColorRole.Text, Qt.GlobalColor.white)
    palette.setColor(QPalette.ColorRole.Button, QColor(53, 53, 53))
    palette.setColor(QPalette.ColorRole.ButtonText, Qt.GlobalColor.white)
    palette.setColor(QPalette.ColorRole.BrightText, Qt.GlobalColor.red)
    palette.setColor(QPalette.ColorRole.Link, QColor(42, 130, 218))
    palette.setColor(QPalette.ColorRole.Highlight, QColor(42, 130, 218))
    palette.setColor(QPalette.ColorRole.HighlightedText, Qt.GlobalColor.black)
    app.setPalette(palette)
    
    window = SyncPlayClient()
    window.show()
    sys.exit(app.exec())
