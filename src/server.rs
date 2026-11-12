use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use log::{error, info};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::SyncMessage;

type ClientId = u64;
type ClientSender = mpsc::UnboundedSender<String>;

/// Shared state of the synchronization server, matching `SyncServer` in `server.py`.
#[derive(Debug)]
pub struct ServerState {
    pub clients: HashMap<ClientId, ClientSender>,
    pub current_url: Option<String>,
    pub playlist: Vec<String>,
    pub current_time: f64,
    pub is_paused: bool,
    pub client_times: HashMap<ClientId, f64>,
    pub ready_clients: HashSet<ClientId>,
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
            current_url: None,
            playlist: Vec::new(),
            current_time: 0.0,
            is_paused: true,
            client_times: HashMap::new(),
            ready_clients: HashSet::new(),
        }
    }

    pub fn broadcast_all(&self, message: &str) {
        for sender in self.clients.values() {
            let _ = sender.send(message.to_string());
        }
    }

    pub fn broadcast_except(&self, sender_id: ClientId, message: &str) {
        for (&id, sender) in &self.clients {
            if id != sender_id {
                let _ = sender.send(message.to_string());
            }
        }
    }

    pub fn broadcast_user_count(&self) {
        if !self.clients.is_empty() {
            if let Ok(msg) = serde_json::to_string(&SyncMessage::UserCount {
                count: self.clients.len(),
            }) {
                self.broadcast_all(&msg);
            }
        }
    }

    pub fn broadcast_ready_count(&self) {
        if let Ok(msg) = serde_json::to_string(&SyncMessage::ReadyCount {
            count: self.ready_clients.len(),
        }) {
            self.broadcast_all(&msg);
        }
    }

    pub fn register_client(&mut self, id: ClientId, tx: ClientSender) {
        self.clients.insert(id, tx.clone());
        info!("Client connected. Total clients: {}", self.clients.len());
        self.broadcast_user_count();

        if let Ok(ready_msg) = serde_json::to_string(&SyncMessage::ReadyCount {
            count: self.ready_clients.len(),
        }) {
            let _ = tx.send(ready_msg);
        }

        if !self.playlist.is_empty() {
            if let Ok(msg) = serde_json::to_string(&SyncMessage::Playlist {
                urls: self.playlist.clone(),
            }) {
                let _ = tx.send(msg);
            }
        }

        if let Some(ref url) = self.current_url {
            if let Ok(play_msg) = serde_json::to_string(&SyncMessage::Play { url: url.clone() }) {
                let _ = tx.send(play_msg);
            }
            if let Ok(seek_msg) = serde_json::to_string(&SyncMessage::Seek {
                position: self.current_time,
            }) {
                let _ = tx.send(seek_msg);
            }
            if let Ok(pause_msg) = serde_json::to_string(&SyncMessage::Pause {
                state: self.is_paused,
            }) {
                let _ = tx.send(pause_msg);
            }
        }
    }

    pub fn unregister_client(&mut self, id: ClientId) {
        self.clients.remove(&id);
        self.client_times.remove(&id);
        let was_ready = self.ready_clients.remove(&id);
        info!("Client disconnected. Total clients: {}", self.clients.len());
        self.broadcast_user_count();
        if was_ready {
            self.broadcast_ready_count();
        }

        self.is_paused = true;
        if !self.clients.is_empty() {
            if let Ok(pause_msg) = serde_json::to_string(&SyncMessage::Pause { state: true }) {
                self.broadcast_all(&pause_msg);
            }
        }
    }

    pub fn process_message(&mut self, sender_id: ClientId, raw_message: &str) {
        match serde_json::from_str::<SyncMessage>(raw_message) {
            Ok(data) => {
                info!("Received message: {:?}", data);
                match data {
                    SyncMessage::Play { url } => {
                        self.current_url = Some(url);
                        self.current_time = 0.0;
                        self.client_times.clear();
                    }
                    SyncMessage::Playlist { urls } => {
                        self.playlist = urls;
                    }
                    SyncMessage::Pause { state } => {
                        self.is_paused = state;
                    }
                    SyncMessage::Seek { position } => {
                        self.current_time = position;
                        self.client_times.clear();
                    }
                    SyncMessage::TimeSync { position } => {
                        if position < 0.0 {
                            return;
                        }
                        self.current_time = position;
                        self.client_times.insert(sender_id, position);
                        if self.client_times.len() > 1 {
                            let min_time = self
                                .client_times
                                .values()
                                .copied()
                                .fold(f64::INFINITY, f64::min);
                            let max_time = self
                                .client_times
                                .values()
                                .copied()
                                .fold(f64::NEG_INFINITY, f64::max);
                            let drift = max_time - min_time;
                            if drift > 2.0 {
                                self.client_times.clear();
                                if drift <= 5.0 {
                                    // Minor drift between active players: sync everyone to min_time
                                    if let Ok(seek_msg) = serde_json::to_string(&SyncMessage::Seek {
                                        position: min_time,
                                    }) {
                                        self.broadcast_all(&seek_msg);
                                    }
                                } else {
                                    // Large drift (e.g. reload or buffering): do not kick active players back to start!
                                    // Sync only the lagging client(s) forward to max_time
                                    if let Ok(seek_msg) = serde_json::to_string(&SyncMessage::Seek {
                                        position: max_time,
                                    }) {
                                        for (&client_id, tx) in &self.clients {
                                            if client_id == sender_id && position < max_time - 2.0 {
                                                let _ = tx.send(seek_msg.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Do not broadcast time_sync to avoid stuttering clients
                        return;
                    }
                    SyncMessage::RequestSync => {
                        let room_time = self
                            .client_times
                            .iter()
                            .filter(|(&id, _)| id != sender_id)
                            .map(|(_, &t)| t)
                            .fold(self.current_time, f64::max);
                        self.client_times.remove(&sender_id);
                        if let Some(tx) = self.clients.get(&sender_id) {
                            if let Some(ref url) = self.current_url {
                                if let Ok(play_msg) =
                                    serde_json::to_string(&SyncMessage::Play { url: url.clone() })
                                {
                                    let _ = tx.send(play_msg);
                                }
                                if let Ok(seek_msg) =
                                    serde_json::to_string(&SyncMessage::Seek { position: room_time })
                                {
                                    let _ = tx.send(seek_msg);
                                }
                                if let Ok(pause_msg) =
                                    serde_json::to_string(&SyncMessage::Pause { state: self.is_paused })
                                {
                                    let _ = tx.send(pause_msg);
                                }
                            }
                            if let Ok(ready_msg) =
                                serde_json::to_string(&SyncMessage::ReadyCount { count: self.ready_clients.len() })
                            {
                                let _ = tx.send(ready_msg);
                            }
                        }
                        return;
                    }
                    SyncMessage::Ready { state } => {
                        if state {
                            self.ready_clients.insert(sender_id);
                        } else {
                            self.ready_clients.remove(&sender_id);
                        }
                        self.broadcast_ready_count();
                        return;
                    }
                    SyncMessage::ReadyCount { .. } => {}
                    SyncMessage::UserCount { .. } => {}
                }

                // Simple broadcast of original message to all other clients
                self.broadcast_except(sender_id, raw_message);
            }
            Err(err) => {
                error!("Invalid JSON received ({}): {}", err, raw_message);
            }
        }
    }
}

/// Asynchronous WebSocket synchronization server (`SyncServer`).
pub struct SyncServer {
    pub host: String,
    pub port: u16,
    state: Arc<Mutex<ServerState>>,
    next_client_id: Arc<AtomicU64>,
}

impl Default for SyncServer {
    fn default() -> Self {
        Self::new("0.0.0.0", 8765)
    }
}

impl SyncServer {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            host: host.to_string(),
            port,
            state: Arc::new(Mutex::new(ServerState::new())),
            next_client_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Binds a `TcpListener` on `host:port` and serves clients indefinitely.
    pub async fn start(&self) -> std::io::Result<()> {
        let addr = format!("{}:{}", self.host, self.port);
        info!("Starting server on {}...", addr);
        let listener = TcpListener::bind(&addr).await?;
        self.serve_listener(listener).await
    }

    /// Runs the accept loop on an existing `TcpListener`.
    pub async fn serve_listener(&self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, peer_addr) = listener.accept().await?;
            let state = Arc::clone(&self.state);
            let client_id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                handle_connection(stream, peer_addr, client_id, state).await;
            });
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    _peer_addr: SocketAddr,
    client_id: ClientId,
    state: Arc<Mutex<ServerState>>,
) {
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("WebSocket handshake error: {}", e);
            return;
        }
    };

    let (mut ws_sink, mut ws_source) = ws_stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    {
        let mut locked = state.lock().await;
        locked.register_client(client_id, tx);
    }

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sink.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    while let Some(result) = ws_source.next().await {
        match result {
            Ok(Message::Text(text)) => {
                let mut locked = state.lock().await;
                locked.process_message(client_id, &text);
            }
            Ok(Message::Close(_)) => break,
            Err(_) => break,
            _ => {}
        }
    }

    {
        let mut locked = state.lock().await;
        locked.unregister_client(client_id);
    }

    send_task.abort();
}

/// Spawns the `SyncServer` in a dedicated background daemon thread (used by the GUI's "Host Server" button).
pub fn spawn_server_thread(host: &str, port: u16) -> std::thread::JoinHandle<()> {
    let host = host.to_string();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create Tokio runtime for SyncServer");
        rt.block_on(async move {
            let server = SyncServer::new(&host, port);
            if let Err(e) = server.start().await {
                error!("SyncServer exited with error: {}", e);
            }
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_state_lifecycle_and_time_sync() {
        let mut state = ServerState::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();

        // Register client 1
        state.register_client(1, tx1);
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::UserCount { count: 1 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 0 }
        );

        // Client 1 sets playlist and plays URL
        state.process_message(1, r#"{"action":"playlist","urls":["v1.mp4","v2.mp4"]}"#);
        state.process_message(1, r#"{"action":"play","url":"v1.mp4"}"#);
        state.process_message(1, r#"{"action":"seek","position":15.0}"#);
        state.process_message(1, r#"{"action":"pause","state":false}"#);

        // Register client 2 -> should receive user_count, ready_count, playlist, play, seek, pause
        state.register_client(2, tx2);
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::UserCount { count: 2 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::UserCount { count: 2 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 0 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Playlist {
                urls: vec!["v1.mp4".to_string(), "v2.mp4".to_string()]
            }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Play {
                url: "v1.mp4".to_string()
            }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Seek { position: 15.0 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Pause { state: false }
        );

        // Time sync within 2.0s should not emit a seek
        state.process_message(1, r#"{"action":"time_sync","position":20.0}"#);
        state.process_message(2, r#"{"action":"time_sync","position":21.5}"#);
        assert!(rx1.try_recv().is_err());
        assert!(rx2.try_recv().is_err());

        // Time sync drift > 2.0s should broadcast seek to min_time to both clients
        state.process_message(1, r#"{"action":"time_sync","position":25.0}"#);
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::Seek { position: 21.5 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Seek { position: 21.5 }
        );

        // Client 2 disconnects -> remaining client receives user_count=1 and pause=true
        state.unregister_client(2);
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::UserCount { count: 1 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::Pause { state: true }
        );
    }

    #[test]
    fn test_request_sync_and_large_drift() {
        let mut state = ServerState::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();

        state.register_client(1, tx1);
        let _ = rx1.try_recv(); // user count
        let _ = rx1.try_recv(); // ready count

        state.process_message(1, r#"{"action":"play","url":"movie.mp4"}"#);
        state.process_message(1, r#"{"action":"seek","position":120.0}"#);
        state.process_message(1, r#"{"action":"pause","state":false}"#);

        state.register_client(2, tx2);
        let _ = rx1.try_recv(); // user count
        let _ = rx2.try_recv(); // user count
        let _ = rx2.try_recv(); // ready count
        let _ = rx2.try_recv(); // play
        let _ = rx2.try_recv(); // seek
        let _ = rx2.try_recv(); // pause

        // Client 1 is playing ahead at 150.0
        state.process_message(1, r#"{"action":"time_sync","position":150.0}"#);

        // Client 2 requests sync during reload
        state.process_message(2, r#"{"action":"request_sync"}"#);

        // Client 1 should NOT receive anything
        assert!(rx1.try_recv().is_err());

        // Client 2 should receive current room time (150.0)
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Play { url: "movie.mp4".to_string() }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Seek { position: 150.0 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Pause { state: false }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 0 }
        );

        // Large drift (> 5.0) from a lagging client must not kick client 1 back to 0.0
        state.process_message(1, r#"{"action":"time_sync","position":155.0}"#);
        state.process_message(2, r#"{"action":"time_sync","position":0.0}"#);

        // Client 1 should NOT be sought back to 0.0
        assert!(rx1.try_recv().is_err());

        // Only client 2 receives seek to catch up to max_time (155.0)
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::Seek { position: 155.0 }
        );
    }

    #[test]
    fn test_ready_counter_flow() {
        let mut state = ServerState::new();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let (tx3, mut rx3) = mpsc::unbounded_channel();

        state.register_client(1, tx1);
        let _ = rx1.try_recv(); // user count
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 0 }
        );

        state.register_client(2, tx2);
        let _ = rx1.try_recv(); // user count
        let _ = rx2.try_recv(); // user count
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 0 }
        );

        state.register_client(3, tx3);
        let _ = rx1.try_recv(); // user count
        let _ = rx2.try_recv(); // user count
        let _ = rx3.try_recv(); // user count
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx3.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 0 }
        );

        // Client 1 clicks ready
        state.process_message(1, r#"{"action":"ready","state":true}"#);
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 1 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 1 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx3.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 1 }
        );

        // Client 2 clicks ready
        state.process_message(2, r#"{"action":"ready","state":true}"#);
        let _ = rx1.try_recv();
        let _ = rx2.try_recv();
        let _ = rx3.try_recv();

        // Client 3 clicks ready -> all 3 ready!
        state.process_message(3, r#"{"action":"ready","state":true}"#);
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 3 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 3 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx3.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 3 }
        );

        // Client 3 un-readies
        state.process_message(3, r#"{"action":"ready","state":false}"#);
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 2 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx2.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 2 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx3.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 2 }
        );

        // Client 2 disconnects -> ready count drops to 1
        state.unregister_client(2);
        let _ = rx1.try_recv(); // user count = 2
        let _ = rx3.try_recv(); // user count = 2
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx1.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 1 }
        );
        assert_eq!(
            serde_json::from_str::<SyncMessage>(&rx3.try_recv().unwrap()).unwrap(),
            SyncMessage::ReadyCount { count: 1 }
        );
    }
}
