use serde::{Deserialize, Serialize};

/// Wire-protocol messages exchanged over WebSockets between the AVCWT server and clients.
///
/// Uses `#[serde(tag = "action", rename_all = "snake_case")]` so the JSON format is 100%
/// compatible with the original Python `server.py` and `client.py`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SyncMessage {
    UserCount {
        count: usize,
    },
    Playlist {
        #[serde(default)]
        urls: Vec<String>,
    },
    Play {
        url: String,
    },
    Pause {
        #[serde(default = "default_true")]
        state: bool,
    },
    Seek {
        #[serde(default)]
        position: f64,
    },
    TimeSync {
        #[serde(default)]
        position: f64,
    },
}

fn default_true() -> bool {
    true
}

/// Formats a duration in seconds as `MM:SS` or `HH:MM:SS` (matching `client.py`'s `format_time`).
pub fn format_time(seconds: Option<f64>) -> String {
    match seconds {
        None => "00:00".to_string(),
        Some(secs) if !secs.is_finite() || secs < 0.0 => "00:00".to_string(),
        Some(secs) => {
            let total_secs = secs as u64;
            let s = total_secs % 60;
            let total_mins = total_secs / 60;
            let m = total_mins % 60;
            let h = total_mins / 60;
            if h > 0 {
                format!("{h:02}:{m:02}:{s:02}")
            } else {
                format!("{m:02}:{s:02}")
            }
        }
    }
}

/// Normalizes a server host/URI input by trimming whitespace and prepending `ws://` if no
/// WebSocket scheme is present.
pub fn normalize_ws_uri(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        Some(trimmed.to_string())
    } else {
        Some(format!("ws://{trimmed}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_time() {
        assert_eq!(format_time(None), "00:00");
        assert_eq!(format_time(Some(-5.0)), "00:00");
        assert_eq!(format_time(Some(0.0)), "00:00");
        assert_eq!(format_time(Some(65.9)), "01:05");
        assert_eq!(format_time(Some(3661.0)), "01:01:01");
    }

    #[test]
    fn test_sync_message_serde_compatibility() {
        let play_json = r#"{"action":"play","url":"https://example.com/video.mp4"}"#;
        let msg: SyncMessage = serde_json::from_str(play_json).unwrap();
        assert_eq!(
            msg,
            SyncMessage::Play {
                url: "https://example.com/video.mp4".to_string()
            }
        );
        assert_eq!(serde_json::to_string(&msg).unwrap(), play_json);

        let pause_json = r#"{"action":"pause","state":false}"#;
        let msg: SyncMessage = serde_json::from_str(pause_json).unwrap();
        assert_eq!(msg, SyncMessage::Pause { state: false });

        let seek_json = r#"{"action":"seek","position":42.5}"#;
        let msg: SyncMessage = serde_json::from_str(seek_json).unwrap();
        assert_eq!(msg, SyncMessage::Seek { position: 42.5 });

        let playlist_json = r#"{"action":"playlist","urls":["a.mp4","b.mp4"]}"#;
        let msg: SyncMessage = serde_json::from_str(playlist_json).unwrap();
        assert_eq!(
            msg,
            SyncMessage::Playlist {
                urls: vec!["a.mp4".to_string(), "b.mp4".to_string()]
            }
        );

        let user_count_json = r#"{"action":"user_count","count":3}"#;
        let msg: SyncMessage = serde_json::from_str(user_count_json).unwrap();
        assert_eq!(msg, SyncMessage::UserCount { count: 3 });

        let time_sync_json = r#"{"action":"time_sync","position":12.25}"#;
        let msg: SyncMessage = serde_json::from_str(time_sync_json).unwrap();
        assert_eq!(msg, SyncMessage::TimeSync { position: 12.25 });
    }

    #[test]
    fn test_normalize_ws_uri() {
        assert_eq!(normalize_ws_uri(""), None);
        assert_eq!(
            normalize_ws_uri("localhost:8765"),
            Some("ws://localhost:8765".to_string())
        );
        assert_eq!(
            normalize_ws_uri("ws://192.168.1.10:8765"),
            Some("ws://192.168.1.10:8765".to_string())
        );
        assert_eq!(
            normalize_ws_uri("wss://sync.example.com"),
            Some("wss://sync.example.com".to_string())
        );
    }
}
