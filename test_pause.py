import sys, os, threading, asyncio, time
os.environ["QT_QPA_PLATFORM"] = "offscreen"
from client import SyncPlayClient, QApplication, QListWidgetItem
from PyQt6.QtCore import QTimer

app = QApplication(sys.argv)

client1 = SyncPlayClient()
client1.host_server() # client1 is host
client1.show()

client2 = SyncPlayClient()
client2.ip_input.setText("ws://localhost:8765")
client2.show()

url1 = "https://www.youtube.com/watch?v=9bZkp7q19f0"
url2 = "https://www.youtube.com/watch?v=dQw4w9WgXcQ"

item1 = QListWidgetItem(url1)
item2 = QListWidgetItem(url2)
client1.file_list.addItem(item1)
client1.file_list.addItem(item2)

def connect_client2():
    client2.connect_to_server()

def set_current():
    client1.file_list.setCurrentItem(client1.file_list.item(0))

def do_play():
    client1.play_file(client1.file_list.item(0))

def do_next():
    print("Playing next item via EOF event on client 1...")
    client1.mpv_signals.eof_reached.emit()

def check_state():
    print("Client 1 URL:", client1.file_list.currentItem().text() if client1.file_list.currentItem() else None)
    print("Client 1 pause:", client1.player.pause)
    print("Client 1 pending pause:", getattr(client1, '_pending_pause', None))

    print("Client 2 URL:", client2.file_list.currentItem().text() if client2.file_list.currentItem() else None)
    print("Client 2 pause:", client2.player.pause)
    print("Client 2 pending pause:", getattr(client2, '_pending_pause', None))

    app.quit()

QTimer.singleShot(2000, connect_client2)
QTimer.singleShot(3000, set_current)
QTimer.singleShot(4000, do_play)
QTimer.singleShot(10000, do_next)
QTimer.singleShot(15000, check_state)

app.exec()
