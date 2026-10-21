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

    async def register(self, websocket):
        self.clients.add(websocket)
        logging.info(f"Client connected. Total clients: {len(self.clients)}")

    async def unregister(self, websocket):
        self.clients.remove(websocket)
        logging.info(f"Client disconnected. Total clients: {len(self.clients)}")

    async def handler(self, websocket, path):
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
