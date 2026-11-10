use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use log::{error, info};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::SyncMessage;

/// Events delivered from the background WebSocket client task to the GUI thread.
#[derive(Debug, Clone)]
pub enum NetworkEvent {
    ConnectionStatus(bool),
    MessageReceived(SyncMessage),
}

/// Manages a background WebSocket connection to a `SyncServer` (matching `WebSocketThread` in `client.py`).
pub struct WebSocketClient {
    event_rx: std_mpsc::Receiver<NetworkEvent>,
    event_tx: std_mpsc::Sender<NetworkEvent>,
    outbound_tx: Option<tokio_mpsc::UnboundedSender<SyncMessage>>,
    is_running: bool,
}

impl Default for WebSocketClient {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSocketClient {
    pub fn new() -> Self {
        let (event_tx, event_rx) = std_mpsc::channel();
        Self {
            event_rx,
            event_tx,
            outbound_tx: None,
            is_running: false,
        }
    }

    pub fn is_running(&self) -> bool {
        self.is_running
    }

    pub fn set_running(&mut self, running: bool) {
        self.is_running = running;
        if !running {
            self.outbound_tx = None;
        }
    }

    /// Polls all pending network events without blocking.
    pub fn poll_events(&self) -> Vec<NetworkEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.event_rx.try_recv() {
            events.push(ev);
        }
        events
    }

    /// Connects to the given `ws://` or `wss://` URI in a background thread.
    pub fn connect_to(&mut self, uri: String, repaint_ctx: Option<egui::Context>) {
        let (out_tx, mut out_rx) = tokio_mpsc::unbounded_channel::<SyncMessage>();
        self.outbound_tx = Some(out_tx);
        let event_tx = self.event_tx.clone();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to create WebSocket runtime: {}", e);
                    let _ = event_tx.send(NetworkEvent::ConnectionStatus(false));
                    if let Some(ref ctx) = repaint_ctx {
                        ctx.request_repaint();
                    }
                    return;
                }
            };

            rt.block_on(async move {
                match connect_async(&uri).await {
                    Ok((ws_stream, _)) => {
                        info!("Connected to server at {}", uri);
                        let _ = event_tx.send(NetworkEvent::ConnectionStatus(true));
                        if let Some(ref ctx) = repaint_ctx {
                            ctx.request_repaint();
                        }

                        let (mut ws_sink, mut ws_source) = ws_stream.split();

                        loop {
                            tokio::select! {
                                outbound = out_rx.recv() => {
                                    match outbound {
                                        Some(msg) => {
                                            if let Ok(json) = serde_json::to_string(&msg) {
                                                if ws_sink.send(Message::Text(json)).await.is_err() {
                                                    break;
                                                }
                                            }
                                        }
                                        None => {
                                            let _ = ws_sink.send(Message::Close(None)).await;
                                            break;
                                        }
                                    }
                                }
                                inbound = tokio::time::timeout(Duration::from_secs(1), ws_source.next()) => {
                                    match inbound {
                                        Ok(Some(Ok(Message::Text(text)))) => {
                                            if let Ok(parsed) = serde_json::from_str::<SyncMessage>(&text) {
                                                let _ = event_tx.send(NetworkEvent::MessageReceived(parsed));
                                                if let Some(ref ctx) = repaint_ctx {
                                                    ctx.request_repaint();
                                                }
                                            }
                                        }
                                        Ok(Some(Ok(Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => {
                                            break;
                                        }
                                        Ok(Some(Ok(_))) | Err(_) => {
                                            // Timeout or ping/pong frame; continue listening
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Connection failed: {}", e);
                    }
                }

                let _ = event_tx.send(NetworkEvent::ConnectionStatus(false));
                if let Some(ref ctx) = repaint_ctx {
                    ctx.request_repaint();
                }
            });
        });
    }

    /// Sends a `SyncMessage` over the active WebSocket connection, if connected.
    pub fn send_message(&self, msg: SyncMessage) {
        if self.is_running {
            if let Some(ref tx) = self.outbound_tx {
                let _ = tx.send(msg);
            }
        }
    }

    /// Stops the active WebSocket connection.
    pub fn stop(&mut self) {
        self.is_running = false;
        self.outbound_tx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::SyncServer;
    use tokio::net::TcpListener;

    #[test]
    fn test_websocket_client_server_roundtrip() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        let port = rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            tokio::spawn(async move {
                let server = SyncServer::new("127.0.0.1", port);
                let _ = server.serve_listener(listener).await;
            });
            port
        });

        let mut client1 = WebSocketClient::new();
        let mut client2 = WebSocketClient::new();
        client1.connect_to(format!("ws://127.0.0.1:{port}"), None);
        client2.connect_to(format!("ws://127.0.0.1:{port}"), None);

        // Wait until both clients connect
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut c1_connected = false;
        let mut c2_connected = false;
        while std::time::Instant::now() < deadline && (!c1_connected || !c2_connected) {
            for ev in client1.poll_events() {
                if let NetworkEvent::ConnectionStatus(true) = ev {
                    client1.set_running(true);
                    c1_connected = true;
                }
            }
            for ev in client2.poll_events() {
                if let NetworkEvent::ConnectionStatus(true) = ev {
                    client2.set_running(true);
                    c2_connected = true;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(c1_connected && c2_connected);

        // Client 1 sends a Play message, verify Client 2 receives it
        client1.send_message(SyncMessage::Play {
            url: "https://example.com/stream.mp4".to_string(),
        });

        let mut received_play = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && !received_play {
            for ev in client2.poll_events() {
                if let NetworkEvent::MessageReceived(SyncMessage::Play { url }) = ev {
                    if url == "https://example.com/stream.mp4" {
                        received_play = true;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(received_play);

        client1.stop();
        client2.stop();
    }
}

