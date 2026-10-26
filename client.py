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

class MpvContainer(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setAttribute(Qt.WidgetAttribute.WA_DontCreateNativeAncestors)
        self.setAttribute(Qt.WidgetAttribute.WA_NativeWindow)
        self.setMouseTracking(True)
        self.player = None

    def set_player(self, player):
        self.player = player

    def mouseMoveEvent(self, event):
        if self.player:
            self.player.command('mouse', int(event.position().x()), int(event.position().y()))
        super().mouseMoveEvent(event)

    def mousePressEvent(self, event):
        if self.player:
            button_map = {Qt.MouseButton.LeftButton: 0, Qt.MouseButton.RightButton: 2, Qt.MouseButton.MiddleButton: 1}
            btn = button_map.get(event.button())
            if btn is not None:
                self.player.command('mouse', int(event.position().x()), int(event.position().y()), btn, 'single')
        super().mousePressEvent(event)

    def mouseReleaseEvent(self, event):
        super().mouseReleaseEvent(event)

    def wheelEvent(self, event):
        if self.player:
            delta = event.angleDelta().y()
            if delta > 0:
                self.player.command('keydown', 'WHEEL_UP')
                self.player.command('keyup', 'WHEEL_UP')
            elif delta < 0:
                self.player.command('keydown', 'WHEEL_DOWN')
                self.player.command('keyup', 'WHEEL_DOWN')
        super().wheelEvent(event)

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
        self.setWindowTitle("AVCWT")
        self.resize(800, 600)

        # Main Widget and Layout
        main_widget = QWidget()
        self.setCentralWidget(main_widget)
        main_layout = QHBoxLayout(main_widget)

        # Left Panel (Video Player Area & Controls)
        left_panel = QWidget()
        left_layout = QVBoxLayout(left_panel)
        main_layout.addWidget(left_panel, stretch=3)

        self.video_container = MpvContainer()
        self.video_container.setStyleSheet("background-color: black;")
        left_layout.addWidget(self.video_container, stretch=1)
        
        # Initialize MPV
        locale.setlocale(locale.LC_NUMERIC, 'C') # required by mpv

        mpv_args = {
            'wid': str(int(self.video_container.winId())),
            'profile': 'pseudo-gui',
            'script_opts': 'osc-seekbarstyle=bar,osc-deadzonesize=0,osc-minmousemove=3',
            'input_default_bindings': True,
            'input_vo_keyboard': True,
            'osc': True,
            'hwdec': 'auto',
            'cache': 'yes'
        }
        if sys.platform.startswith('linux'):
            mpv_args['vo'] = 'x11'

        self.player = mpv.MPV(**mpv_args)
        self.video_container.set_player(self.player)

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
        
        # Signals
        self.spin_cache.valueChanged.connect(self.update_cache_size)
        
        # Bind MPV events
        self.player.observe_property('pause', self.on_mpv_pause)
        self.player.observe_property('time-pos', self.on_mpv_time_pos)
        
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

    def toggle_fullscreen(self):
        if self.isFullScreen():
            self.showNormal()
            self.right_panel.setVisible(True)
        else:
            self.showFullScreen()
            self.right_panel.setVisible(False)

    def toggle_playback(self):
        if self.player.pause is not None:
            self.player.pause = not self.player.pause

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
