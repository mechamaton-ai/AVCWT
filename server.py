import asyncio
import json
import logging
import websockets

logging.basicConfig(level=logging.INFO)

class SyncServer:
    def __init__(self):
        self.clients = set()
        self.host = "0.0.0.0"
        self.port = 8765
        self.current_url = None
        self.playlist = []
        self.current_time = 0.0
        self.is_paused = True

    async def broadcast_user_count(self):
        if self.clients:
            message = json.dumps({"action": "user_count", "count": len(self.clients)})
            tasks = [asyncio.create_task(client.send(message)) for client in self.clients]
            if tasks:
                await asyncio.wait(tasks)

    async def register(self, websocket):
        self.clients.add(websocket)
        logging.info(f"Client connected. Total clients: {len(self.clients)}")
        await self.broadcast_user_count()

        if self.playlist:
            await websocket.send(json.dumps({
                "action": "playlist",
                "urls": self.playlist
            }))

        if self.current_url:
            await websocket.send(json.dumps({
                "action": "play",
                "url": self.current_url
            }))
            await websocket.send(json.dumps({
                "action": "seek",
                "position": self.current_time
            }))
            await websocket.send(json.dumps({
                "action": "pause",
                "state": self.is_paused
            }))

    async def unregister(self, websocket):
        self.clients.remove(websocket)
        logging.info(f"Client disconnected. Total clients: {len(self.clients)}")
        await self.broadcast_user_count()

    async def handler(self, websocket):
        await self.register(websocket)
        try:
            async for message in websocket:
                await self.process_message(websocket, message)
        except websockets.exceptions.ConnectionClosed:
            pass
        finally:
            await self.unregister(websocket)

    async def process_message(self, sender_websocket, message_str):
        try:
            data = json.loads(message_str)
            logging.info(f"Received message: {data}")
            
            if data.get("action") == "play":
                self.current_url = data.get("url")
            elif data.get("action") == "playlist":
                self.playlist = data.get("urls", [])
            elif data.get("action") == "pause":
                self.is_paused = data.get("state", True)
            elif data.get("action") == "seek":
                self.current_time = data.get("position", 0.0)

            # Simple broadcast to all other clients
            if self.clients:
                tasks = []
                for client in self.clients:
                    if client != sender_websocket:
                        tasks.append(asyncio.create_task(client.send(message_str)))
                
                if tasks:
                    await asyncio.wait(tasks)
        except json.JSONDecodeError:
            logging.error(f"Invalid JSON received: {message_str}")
        except Exception as e:
            logging.error(f"Error processing message: {e}")

    async def start(self):
        logging.info(f"Starting server on {self.host}:{self.port}...")
        async with websockets.serve(self.handler, self.host, self.port):
            await asyncio.Future()  # run forever

def run_server():
    server = SyncServer()
    asyncio.run(server.start())

if __name__ == "__main__":
    run_server()
