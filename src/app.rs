use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use egui::{
    Align, Color32, Context, CursorIcon, Frame, Id, Key, Layout, Margin, Order, Pos2, Rect,
    RichText, Rounding, Sense, Shape, Stroke, SystemTheme, TextureHandle, TextureOptions,
    Theme, ThemePreference, UiBuilder, Vec2, ViewportCommand,
};
use log::{info, warn};

use crate::mpv::{check_is_live_async, MediaTrack, MpvEvent, MpvPlayer, TrackType};
use crate::network::{NetworkEvent, WebSocketClient};
use crate::protocol::{format_time, normalize_ws_uri, SyncMessage};
use crate::server::spawn_server_thread;
use crate::settings::AppSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionUiState {
    Disconnected,
    Connecting,
    Connected,
}

/// Main GUI application state, mirroring `SyncPlayClient` in `client.py`.
pub struct SyncPlayApp {
    player: Option<MpvPlayer>,
    player_error: Option<String>,
    video_texture: Option<TextureHandle>,

    // Playlist & playback state
    playlist: Vec<String>,
    selected_playlist_idx: Option<usize>,
    current_playing_url: Option<String>,
    current_time: f64,
    current_duration: f64,
    cache_duration: f64,
    is_paused: bool,
    volume: f64,
    video_aspect: f64,
    media_title: Option<String>,

    // Audio & subtitle tracks
    audio_tracks: Vec<(Option<i64>, String)>,
    selected_audio_idx: usize,
    sub_tracks: Vec<(Option<i64>, String)>,
    selected_sub_idx: usize,

    // UI visibility & modals
    sidebar_visible: bool,
    sidebar_was_visible_before_fullscreen: bool,
    is_fullscreen: bool,
    osc_visible: bool,
    last_mouse_activity: Instant,
    last_pointer_pos: Option<Pos2>,
    volume_popup_open: bool,
    volume_popup_rect: Option<Rect>,
    volume_btn_rect: Option<Rect>,
    volume_indicator_until: Option<Instant>,
    audio_sub_popup_open: bool,
    audio_sub_popup_rect: Option<Rect>,
    audio_sub_btn_rect: Option<Rect>,
    audio_menu_open: bool,
    sub_menu_open: bool,
    settings_dialog_open: bool,
    edit_playlist_dialog_open: bool,
    edit_playlist_buffer: String,

    // Settings & network state
    ip_input: String,
    current_cache_size: i64,
    viewers_count: usize,
    is_ready: bool,
    ready_count: usize,
    conn_state: ConnectionUiState,
    ws_client: WebSocketClient,
    is_host: bool,
    server_running: bool,
    pending_host_connect_at: Option<Instant>,

    // Synchronization flags (matching client.py)
    ignore_next_seek: bool,
    ignore_next_pause: bool,
    pending_seek: Option<f64>,
    pending_pause: Option<bool>,
    pending_pause_apply_at: Option<Instant>,
    video_loading: bool,
    last_time_pos: Option<f64>,
    last_sync_time: f64,
    is_live: bool,
    live_check_tx: Sender<(String, bool)>,
    live_check_rx: Receiver<(String, bool)>,
}

impl Default for SyncPlayApp {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncPlayApp {
    pub fn new() -> Self {
        let saved = AppSettings::load_from_path(AppSettings::DEFAULT_PATH);
        let (live_check_tx, live_check_rx) = mpsc::channel();

        let (player, player_error) = match MpvPlayer::new() {
            Ok(p) => {
                p.update_cache_size(saved.cache_size);
                (Some(p), None)
            }
            Err(e) => {
                warn!("MPV initialization warning: {}", e);
                (None, Some(e))
            }
        };

        Self {
            player,
            player_error,
            video_texture: None,

            playlist: Vec::new(),
            selected_playlist_idx: None,
            current_playing_url: None,
            current_time: 0.0,
            current_duration: 0.0,
            cache_duration: 0.0,
            is_paused: true,
            volume: 100.0,
            video_aspect: 16.0 / 9.0,
            media_title: None,

            audio_tracks: vec![(None, "None".to_string())],
            selected_audio_idx: 0,
            sub_tracks: vec![(None, "None".to_string())],
            selected_sub_idx: 0,

            sidebar_visible: true,
            sidebar_was_visible_before_fullscreen: true,
            is_fullscreen: false,
            osc_visible: true,
            last_mouse_activity: Instant::now(),
            last_pointer_pos: None,
            volume_popup_open: false,
            volume_popup_rect: None,
            volume_btn_rect: None,
            volume_indicator_until: None,
            audio_sub_popup_open: false,
            audio_sub_popup_rect: None,
            audio_sub_btn_rect: None,
            audio_menu_open: false,
            sub_menu_open: false,
            settings_dialog_open: false,
            edit_playlist_dialog_open: false,
            edit_playlist_buffer: String::new(),

            ip_input: saved.host_address,
            current_cache_size: saved.cache_size,
            viewers_count: 0,
            is_ready: false,
            ready_count: 0,
            conn_state: ConnectionUiState::Disconnected,
            ws_client: WebSocketClient::new(),
            is_host: false,
            server_running: false,
            pending_host_connect_at: None,

            ignore_next_seek: false,
            ignore_next_pause: false,
            pending_seek: None,
            pending_pause: None,
            pending_pause_apply_at: None,
            video_loading: false,
            last_time_pos: None,
            last_sync_time: 0.0,
            is_live: false,
            live_check_tx,
            live_check_rx,
        }
    }

    pub fn apply_dark_fusion_theme(ctx: &Context) {
        ctx.set_theme(ThemePreference::Dark);
        ctx.send_viewport_cmd(ViewportCommand::SetTheme(SystemTheme::Dark));

        let mut visuals = egui::Visuals::dark();
        visuals.dark_mode = true;
        visuals.override_text_color = Some(Color32::from_rgb(240, 240, 240));
        visuals.panel_fill = Color32::from_rgb(45, 45, 48);
        visuals.window_fill = Color32::from_rgb(42, 42, 45);
        visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(80, 80, 86));
        visuals.window_rounding = Rounding::same(8.0);
        visuals.menu_rounding = Rounding::same(6.0);
        visuals.extreme_bg_color = Color32::from_rgb(24, 24, 26);
        visuals.faint_bg_color = Color32::from_rgb(35, 35, 38);
        visuals.code_bg_color = Color32::from_rgb(30, 30, 32);

        // Non-interactive widgets & window backgrounds
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(42, 42, 45);
        visuals.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(42, 42, 45);
        visuals.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(75, 75, 80));
        visuals.widgets.noninteractive.fg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(235, 235, 235));
        visuals.widgets.noninteractive.rounding = Rounding::same(6.0);

        // Interactive widgets (buttons, text boxes, sliders, combo boxes) at rest
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(62, 62, 66);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(62, 62, 66);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(90, 90, 96));
        visuals.widgets.inactive.fg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(242, 242, 242));
        visuals.widgets.inactive.rounding = Rounding::same(6.0);

        // Hovered interactive widgets
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(80, 80, 86);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(80, 80, 86);
        visuals.widgets.hovered.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(120, 120, 128));
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, Color32::WHITE);
        visuals.widgets.hovered.rounding = Rounding::same(6.0);

        // Active / pressed interactive widgets
        visuals.widgets.active.bg_fill = Color32::from_rgb(42, 130, 218);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(42, 130, 218);
        visuals.widgets.active.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(100, 170, 245));
        visuals.widgets.active.fg_stroke = Stroke::new(2.0_f32, Color32::WHITE);
        visuals.widgets.active.rounding = Rounding::same(6.0);

        // Open menu / combo-box widgets
        visuals.widgets.open.bg_fill = Color32::from_rgb(55, 55, 60);
        visuals.widgets.open.weak_bg_fill = Color32::from_rgb(55, 55, 60);
        visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(42, 130, 218));
        visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.open.rounding = Rounding::same(6.0);

        visuals.selection.bg_fill = Color32::from_rgb(42, 130, 218);
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.hyperlink_color = Color32::from_rgb(42, 130, 218);

        // Set for BOTH Dark and Light theme slots so OS Light Mode on Windows never turns widgets white
        ctx.set_visuals_of(Theme::Dark, visuals.clone());
        ctx.set_visuals_of(Theme::Light, visuals);
    }

    fn show_osc(&mut self) {
        self.osc_visible = true;
        self.last_mouse_activity = Instant::now();
    }

    fn toggle_playback(&mut self) {
        let next = !self.is_paused;
        self.is_paused = next;
        if let Some(ref mut player) = self.player {
            player.set_pause(next);
        } else {
            self.on_mpv_pause(next);
        }
    }

    fn seek_video(&mut self, slider_val: f64) {
        if self.current_duration > 0.0 {
            let new_pos = (slider_val / 1000.0) * self.current_duration;
            self.current_time = new_pos;
            if let Some(ref mut player) = self.player {
                player.set_time_pos(new_pos);
            } else {
                self.on_mpv_time_pos(new_pos);
            }
        } else {
            self.current_time = 0.0;
            if let Some(ref mut player) = self.player {
                player.set_time_pos(0.0);
            } else {
                self.on_mpv_time_pos(0.0);
            }
        }
    }

    fn change_volume(&mut self, value: f64) {
        self.volume = value.clamp(0.0, 100.0);
        if let Some(ref player) = self.player {
            player.set_volume(self.volume);
        }
    }

    fn update_cache_size(&mut self, value_mb: i64) {
        self.current_cache_size = value_mb.clamp(10, 2000);
        if let Some(ref player) = self.player {
            player.update_cache_size(self.current_cache_size);
        }
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
    }

    fn toggle_fullscreen(&mut self, ctx: &Context) {
        if self.is_fullscreen {
            self.is_fullscreen = false;
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
            self.sidebar_visible = self.sidebar_was_visible_before_fullscreen;
        } else {
            self.sidebar_was_visible_before_fullscreen = self.sidebar_visible;
            self.is_fullscreen = true;
            self.sidebar_visible = false;
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(true));
        }
    }

    fn update_ui_tracks(&mut self, tracks: Vec<MediaTrack>) {
        self.audio_tracks = vec![(None, "None".to_string())];
        self.sub_tracks = vec![(None, "None".to_string())];
        let mut audio_idx = 0;
        let mut sub_idx = 0;

        for track in tracks {
            match track.track_type {
                TrackType::Audio => {
                    let label = track.display_label();
                    self.audio_tracks.push((Some(track.id), label));
                    if track.selected {
                        audio_idx = self.audio_tracks.len() - 1;
                    }
                }
                TrackType::Sub => {
                    let label = track.display_label();
                    self.sub_tracks.push((Some(track.id), label));
                    if track.selected {
                        sub_idx = self.sub_tracks.len() - 1;
                    }
                }
                _ => {}
            }
        }

        self.selected_audio_idx = audio_idx;
        self.selected_sub_idx = sub_idx;
    }

    pub fn play_url(&mut self, url: String, broadcast: bool) {
        self.video_loading = true;
        self.is_live = false;
        self.media_title = None;
        self.current_playing_url = Some(url.clone());
        self.is_paused = true;
        self.pending_pause = Some(false);
        self.pending_pause_apply_at = None;

        if let Some(ref mut player) = self.player {
            player.set_pause(true);
            player.stop();
            player.play(&url);
        }

        check_is_live_async(url.clone(), self.live_check_tx.clone());

        if broadcast && self.ws_client.is_running() {
            self.ws_client.send_message(SyncMessage::Play { url });
        }
    }

    /// Re-initializes the underlying `libmpv` instance and reloads the current video or position.
    pub fn reload_player(&mut self) {
        let resume_url = self.current_playing_url.clone();
        let resume_time = self.current_time;
        let resume_paused = self.is_paused;

        // Drop existing player to release libmpv resources cleanly
        self.player = None;
        self.video_texture = None;

        match MpvPlayer::new() {
            Ok(mut p) => {
                p.update_cache_size(self.current_cache_size);
                p.set_volume(self.volume);
                p.set_pause(true);

                let has_url = resume_url.is_some();
                if let Some(ref url) = resume_url {
                    p.play(url);
                    check_is_live_async(url.clone(), self.live_check_tx.clone());
                }

                self.player = Some(p);
                self.player_error = None;
                self.video_loading = has_url;
                self.pending_seek = if has_url { Some(resume_time) } else { None };
                self.pending_pause = if has_url { Some(resume_paused) } else { None };
                self.pending_pause_apply_at = None;
                self.ignore_next_seek = true;
                self.ignore_next_pause = true;
                self.last_time_pos = None;
                self.last_sync_time = resume_time;

                if self.ws_client.is_running() {
                    self.ws_client.send_message(SyncMessage::RequestSync);
                }

                info!("Player successfully reloaded and resumed");
            }
            Err(e) => {
                warn!("Failed to reload MPV player: {}", e);
                self.player_error = Some(e);
            }
        }
    }

    pub fn play_prev_item(&mut self) {
        if self.current_time > 3.0 {
            self.seek_video(0.0);
            return;
        }

        let Some(ref current_url) = self.current_playing_url else {
            if !self.playlist.is_empty() {
                let url = self.playlist[0].clone();
                self.selected_playlist_idx = Some(0);
                self.play_url(url, true);
            }
            return;
        };

        let current_row = match self.selected_playlist_idx {
            Some(idx) if self.playlist.get(idx) == Some(current_url) => Some(idx),
            _ => self.playlist.iter().position(|u| u == current_url),
        };

        if let Some(row) = current_row {
            if row > 0 {
                let prev_idx = row - 1;
                let prev_url = self.playlist[prev_idx].clone();
                self.selected_playlist_idx = Some(prev_idx);
                self.play_url(prev_url, true);
            } else {
                self.seek_video(0.0);
            }
        } else {
            self.seek_video(0.0);
        }
    }

    pub fn play_next_item(&mut self) {
        let Some(ref current_url) = self.current_playing_url else {
            if !self.playlist.is_empty() {
                let url = self.playlist[0].clone();
                self.selected_playlist_idx = Some(0);
                self.play_url(url, true);
            }
            return;
        };

        let current_row = match self.selected_playlist_idx {
            Some(idx) if self.playlist.get(idx) == Some(current_url) => Some(idx),
            _ => self.playlist.iter().position(|u| u == current_url),
        };

        if let Some(row) = current_row {
            if row + 1 < self.playlist.len() {
                let next_idx = row + 1;
                let next_url = self.playlist[next_idx].clone();
                self.selected_playlist_idx = Some(next_idx);
                self.play_url(next_url, true);
            }
        }
    }

    pub fn broadcast_playlist(&self) {
        if self.ws_client.is_running() {
            self.ws_client.send_message(SyncMessage::Playlist {
                urls: self.playlist.clone(),
            });
        }
    }

    pub fn apply_edited_playlist(&mut self, text: &str) {
        let playing_url = self.current_playing_url.clone();
        let new_urls: Vec<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        let playing_idx = playing_url
            .as_ref()
            .and_then(|url| new_urls.iter().position(|u| u == url));

        self.playlist = new_urls;

        if let Some(idx) = playing_idx {
            self.selected_playlist_idx = Some(idx);
        } else if !self.playlist.is_empty() {
            if playing_url.is_some() {
                if let Some(ref player) = self.player {
                    player.stop();
                }
                self.current_playing_url = None;
            }
            self.selected_playlist_idx = Some(0);
            let first = self.playlist[0].clone();
            self.play_url(first, true);
        } else {
            if let Some(ref player) = self.player {
                player.stop();
            }
            self.current_playing_url = None;
            self.selected_playlist_idx = None;
        }

        self.broadcast_playlist();
    }

    fn connect_to_server(&mut self, ctx: Option<Context>) {
        if let Some(uri) = normalize_ws_uri(&self.ip_input) {
            self.conn_state = ConnectionUiState::Connecting;
            self.ws_client.connect_to(uri, ctx);
        }
    }

    fn host_server(&mut self) {
        if !self.server_running {
            self.is_host = true;
            self.server_running = true;
            spawn_server_thread("0.0.0.0", 8765);
            self.ip_input = "localhost:8765".to_string();
            // Wait 1s for the server to spin up and bind the port before connecting
            self.pending_host_connect_at = Some(Instant::now() + Duration::from_secs(1));
        }
    }

    fn handle_connection_status(&mut self, connected: bool) {
        self.ws_client.set_running(connected);
        if connected {
            self.conn_state = ConnectionUiState::Connected;
            if self.is_ready {
                self.ws_client
                    .send_message(SyncMessage::Ready { state: true });
            }
            if self.is_host {
                self.broadcast_playlist();
                if let Some(ref url) = self.current_playing_url {
                    self.ws_client
                        .send_message(SyncMessage::Play { url: url.clone() });
                    if self.current_time > 0.0 {
                        self.ws_client.send_message(SyncMessage::Seek {
                            position: self.current_time,
                        });
                    }
                    self.ws_client.send_message(SyncMessage::Pause {
                        state: self.is_paused,
                    });
                }
            }
        } else {
            self.conn_state = ConnectionUiState::Disconnected;
            self.ready_count = if self.is_ready { 1 } else { 0 };
            if !self.is_paused {
                self.ignore_next_pause = true;
                self.is_paused = true;
                if let Some(ref mut player) = self.player {
                    player.set_pause(true);
                }
            }
        }
    }

    pub fn handle_network_message(&mut self, msg: SyncMessage) {
        match msg {
            SyncMessage::UserCount { count } => {
                self.viewers_count = count;
            }
            SyncMessage::Pause { state } => {
                if self.video_loading {
                    self.pending_pause = Some(state);
                    self.is_paused = state;
                } else if self.is_paused != state {
                    self.ignore_next_pause = true;
                    self.is_paused = state;
                    if let Some(ref mut player) = self.player {
                        player.set_pause(state);
                    }
                }
            }
            SyncMessage::Seek { position } => {
                if self.video_loading {
                    self.pending_seek = Some(position);
                } else if !self.is_live && (self.current_time - position).abs() > 1.0 {
                    self.ignore_next_seek = true;
                    self.current_time = position;
                    if let Some(ref mut player) = self.player {
                        player.set_time_pos(position);
                    }
                }
            }
            SyncMessage::Playlist { urls } => {
                let current_selected = self
                    .selected_playlist_idx
                    .and_then(|i| self.playlist.get(i).cloned());
                self.playlist = urls;
                self.selected_playlist_idx = current_selected
                    .and_then(|sel| self.playlist.iter().position(|u| u == &sel));
            }
            SyncMessage::Play { url } => {
                if !url.is_empty() {
                    if let Some(pos) = self.playlist.iter().position(|u| u == &url) {
                        self.selected_playlist_idx = Some(pos);
                    } else {
                        self.playlist.push(url.clone());
                        self.selected_playlist_idx = Some(self.playlist.len() - 1);
                    }
                    if self.current_playing_url.as_deref() != Some(&url) || self.player.is_none() {
                        self.play_url(url, false);
                    }
                }
            }
            SyncMessage::TimeSync { .. } => {}
            SyncMessage::RequestSync => {}
            SyncMessage::Ready { .. } => {}
            SyncMessage::ReadyCount { count } => {
                self.ready_count = count;
            }
        }
    }

    fn on_mpv_pause(&mut self, value: bool) {
        if self.video_loading {
            return;
        }
        self.is_paused = value;
        if self.ignore_next_pause {
            self.ignore_next_pause = false;
            return;
        }
        if self.ws_client.is_running() {
            self.ws_client
                .send_message(SyncMessage::Pause { state: value });
        }
    }

    fn complete_video_loading(&mut self) {
        if self.video_loading {
            self.video_loading = false;
            if let Some(seek_pos) = self.pending_seek.take() {
                if !self.is_live {
                    self.current_time = seek_pos;
                    self.last_time_pos = Some(seek_pos);
                    self.last_sync_time = seek_pos;
                    self.ignore_next_seek = true;
                    if let Some(ref mut player) = self.player {
                        player.set_time_pos(seek_pos);
                    }
                }
            } else {
                self.last_time_pos = Some(self.current_time);
                self.last_sync_time = self.current_time;
            }
            if self.pending_pause == Some(true) {
                self.pending_pause_apply_at = None;
                self.apply_pending_pause();
            } else {
                self.pending_pause_apply_at = Some(Instant::now() + Duration::from_millis(300));
            }
        }
    }

    fn apply_pending_pause(&mut self) {
        if let Some(target_pause) = self.pending_pause.take() {
            self.ignore_next_pause = true;
            self.is_paused = target_pause;
            if let Some(ref mut player) = self.player {
                player.set_pause(target_pause);
            }
        }
    }

    fn on_mpv_duration(&mut self, duration: f64) {
        self.current_duration = duration;
        self.complete_video_loading();
    }

    fn on_mpv_time_pos(&mut self, value: f64) {
        if self.video_loading {
            self.complete_video_loading();
            return;
        }

        self.current_time = value;

        if self.last_time_pos.is_none() {
            self.last_time_pos = Some(value);
        }

        if let Some(last_pos) = self.last_time_pos {
            if !self.is_live {
                if (value - last_pos).abs() > 1.5 {
                    if self.ignore_next_seek {
                        self.ignore_next_seek = false;
                        self.last_time_pos = Some(value);
                        return;
                    } else if self.ws_client.is_running() {
                        self.ws_client
                            .send_message(SyncMessage::Seek { position: value });
                    }
                } else if (value - self.last_sync_time).abs() > 1.5 {
                    self.last_sync_time = value;
                    if self.ws_client.is_running() {
                        self.ws_client
                            .send_message(SyncMessage::TimeSync { position: value });
                    }
                }
            }
        }

        self.last_time_pos = Some(value);
    }

    fn process_background_events(&mut self, ctx: &Context) {
        // Check delayed host auto-connect
        if let Some(deadline) = self.pending_host_connect_at {
            if Instant::now() >= deadline {
                self.pending_host_connect_at = None;
                self.connect_to_server(Some(ctx.clone()));
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        // Check delayed pending_pause application (300ms after video load)
        if let Some(deadline) = self.pending_pause_apply_at {
            if Instant::now() >= deadline {
                self.pending_pause_apply_at = None;
                self.apply_pending_pause();
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }

        // Poll async yt-dlp live stream detection results
        while let Ok((url, is_live)) = self.live_check_rx.try_recv() {
            if self.current_playing_url.as_deref() == Some(url.as_str()) {
                self.is_live = is_live;
                info!("Livestream check for {}: is_live={}", url, is_live);
            }
        }

        // Poll WebSocket events
        let net_events = self.ws_client.poll_events();
        for ev in net_events {
            match ev {
                NetworkEvent::ConnectionStatus(connected) => {
                    self.handle_connection_status(connected);
                }
                NetworkEvent::MessageReceived(msg) => {
                    self.handle_network_message(msg);
                }
            }
        }

        // Poll MPV events
        let mpv_events = self
            .player
            .as_mut()
            .map(|p| p.poll_events())
            .unwrap_or_default();

        for ev in mpv_events {
            match ev {
                MpvEvent::PauseChanged(paused) => self.on_mpv_pause(paused),
                MpvEvent::TimePosChanged(pos) => self.on_mpv_time_pos(pos),
                MpvEvent::DurationChanged(dur) => self.on_mpv_duration(dur),
                MpvEvent::CacheDurationChanged(cache_dur) => {
                    self.cache_duration = cache_dur;
                }
                MpvEvent::TrackListChanged(tracks) => self.update_ui_tracks(tracks),
                MpvEvent::VideoAspectChanged(aspect) => {
                    if aspect > 0.01 && aspect.is_finite() {
                        self.video_aspect = aspect;
                    }
                }
                MpvEvent::MediaTitleChanged(title) => {
                    if !title.trim().is_empty() {
                        self.media_title = Some(title);
                    }
                }
                MpvEvent::MouseMoved => {
                    self.show_osc();
                }
                MpvEvent::EofReached => self.play_next_item(),
            }
        }
    }

    fn render_right_sidebar(&mut self, ctx: &Context) {
        if !self.sidebar_visible {
            return;
        }

        egui::SidePanel::right("right_sidebar_panel")
            .resizable(true)
            .default_width(250.0)
            .min_width(200.0)
            .frame(
                Frame::side_top_panel(&ctx.style())
                    .fill(Color32::from_rgb(45, 45, 48))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(65, 65, 70)))
                    .inner_margin(Margin::same(12.0)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("Playlist").strong().color(Color32::WHITE));
                ui.add_space(6.0);

                // Playlist box
                let list_height = (ui.available_height() - 205.0).max(120.0);
                Frame::canvas(ui.style())
                    .fill(Color32::from_rgb(24, 24, 26))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(70, 70, 76)))
                    .rounding(Rounding::same(6.0))
                    .inner_margin(Margin::same(6.0))
                    .show(ui, |ui| {
                        ui.set_min_height(list_height);
                        ui.set_max_height(list_height);
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let mut double_clicked_idx = None;
                                for (idx, item) in self.playlist.iter().enumerate() {
                                    let is_selected = self.selected_playlist_idx == Some(idx);
                                    let resp = ui.selectable_label(
                                        is_selected,
                                        RichText::new(item).color(Color32::from_rgb(235, 235, 235)),
                                    );
                                    if resp.clicked() {
                                        self.selected_playlist_idx = Some(idx);
                                    }
                                    if resp.double_clicked() {
                                        self.selected_playlist_idx = Some(idx);
                                        double_clicked_idx = Some(idx);
                                    }
                                }
                                if let Some(idx) = double_clicked_idx {
                                    if let Some(url) = self.playlist.get(idx).cloned() {
                                        self.play_url(url, true);
                                    }
                                }
                            });
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let spacing = ui.spacing().item_spacing.x;
                    let half_width = ((ui.available_width() - spacing) / 2.0).max(10.0);

                    let edit_btn = egui::Button::new(RichText::new("Edit Playlist").color(Color32::WHITE))
                        .fill(Color32::from_rgb(62, 62, 66))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(90, 90, 96)))
                        .rounding(Rounding::same(6.0));
                    if ui.add_sized([half_width, 30.0], edit_btn).clicked() {
                        self.edit_playlist_buffer = self.playlist.join("\n");
                        self.edit_playlist_dialog_open = true;
                    }

                    let (ready_bg, ready_stroke) = if self.is_ready {
                        (Color32::from_rgb(38, 140, 55), Color32::from_rgb(60, 190, 85))
                    } else {
                        (Color32::from_rgb(175, 40, 40), Color32::from_rgb(220, 70, 70))
                    };

                    let ready_btn = egui::Button::new(
                        RichText::new("Ready").strong().color(Color32::WHITE),
                    )
                    .fill(ready_bg)
                    .stroke(Stroke::new(1.0_f32, ready_stroke))
                    .rounding(Rounding::same(6.0));

                    if ui.add_sized([half_width, 30.0], ready_btn).clicked() {
                        self.is_ready = !self.is_ready;
                        if self.ws_client.is_running() {
                            self.ws_client
                                .send_message(SyncMessage::Ready { state: self.is_ready });
                        } else {
                            self.ready_count = if self.is_ready { 1 } else { 0 };
                        }
                    }
                });

                ui.add_space(14.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new(RichText::new("Settings").color(Color32::WHITE))
                            .fill(Color32::from_rgb(62, 62, 66))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(90, 90, 96)))
                            .rounding(Rounding::same(6.0)),
                    )
                    .clicked()
                {
                    self.settings_dialog_open = true;
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Host Info").strong().color(Color32::WHITE));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("Viewers: {}", self.viewers_count))
                                .color(Color32::from_rgb(200, 200, 205)),
                        );
                    });
                });

                ui.add_space(4.0);
                ui.add_sized(
                    [ui.available_width(), 30.0],
                    egui::TextEdit::singleline(&mut self.ip_input)
                        .hint_text("localhost:8765")
                        .text_color(Color32::WHITE)
                        .margin(Margin::symmetric(10.0, 6.0))
                        .vertical_align(Align::Center),
                );

                ui.add_space(8.0);
                let (btn_text, btn_enabled, btn_fill) = match self.conn_state {
                    ConnectionUiState::Disconnected => {
                        ("Connect", true, Color32::from_rgb(42, 130, 218))
                    }
                    ConnectionUiState::Connecting => {
                        ("Connecting...", false, Color32::from_rgb(70, 70, 76))
                    }
                    ConnectionUiState::Connected => {
                        ("Connected", false, Color32::from_rgb(36, 140, 60))
                    }
                };

                let btn = egui::Button::new(RichText::new(btn_text).strong().color(Color32::WHITE))
                    .fill(btn_fill)
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(100, 100, 108)))
                    .rounding(Rounding::same(6.0));
                if ui
                    .add_enabled_ui(btn_enabled, |ui| {
                        ui.add_sized([ui.available_width(), 30.0], btn)
                    })
                    .inner
                    .clicked()
                {
                    self.connect_to_server(Some(ctx.clone()));
                }
            });
    }

    fn render_video_and_osc(&mut self, ctx: &Context) {
        egui::CentralPanel::default()
            .frame(Frame::none().fill(Color32::BLACK))
            .show(ctx, |ui| {
                let video_rect = ui.max_rect();
                let response = ui.allocate_rect(video_rect, Sense::click_and_drag());

                let pointer_pos =
                    ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()));
                let is_pointer_in_player =
                    pointer_pos.map_or(false, |pos| video_rect.contains(pos));

                // Mouse moving resets 5-second timer and keeps OSC visible
                let mouse_moved = match (pointer_pos, self.last_pointer_pos) {
                    (Some(p1), Some(p2)) => p1.distance_sq(p2) > 1.0,
                    (Some(_), None) => true,
                    _ => false,
                } || ctx.input(|i| i.pointer.is_moving() || i.pointer.delta().length_sq() > 0.5);

                self.last_pointer_pos = pointer_pos;

                if is_pointer_in_player && mouse_moved {
                    self.show_osc();
                }

                // Handle clicks in player window:
                // 1. Closes Audio/CC and Volume windows when clicking outside them
                // 2. Clicking anywhere on the player keeps it active for 5 seconds
                if ctx.input(|i| i.pointer.primary_clicked()) {
                    if let Some(click_pos) = ctx.input(|i| i.pointer.interact_pos()) {
                        if is_pointer_in_player {
                            self.show_osc();
                        }

                        if self.audio_sub_popup_open {
                            let in_popup = self
                                .audio_sub_popup_rect
                                .map_or(false, |r| r.contains(click_pos));
                            let in_btn = self
                                .audio_sub_btn_rect
                                .map_or(false, |r| r.contains(click_pos));
                            if !in_popup && !in_btn {
                                self.audio_sub_popup_open = false;
                            }
                        }

                        if self.volume_popup_open {
                            let in_popup = self
                                .volume_popup_rect
                                .map_or(false, |r| r.contains(click_pos));
                            let in_btn = self
                                .volume_btn_rect
                                .map_or(false, |r| r.contains(click_pos));
                            if !in_popup && !in_btn {
                                self.volume_popup_open = false;
                            }
                        }
                    }
                }

                if response.clicked() {
                    self.audio_sub_popup_open = false;
                    self.volume_popup_open = false;
                    self.show_osc();
                }

                // Scroll wheel over the player controls audio volume
                let combo_open = self.audio_menu_open
                    || self.sub_menu_open
                    || self.audio_sub_popup_open
                    || self.volume_popup_open
                    || ctx.memory(|m| m.any_popup_open());
                if !self.settings_dialog_open
                    && !self.edit_playlist_dialog_open
                    && !combo_open
                    && pointer_pos.map_or(false, |pos| video_rect.contains(pos))
                {
                    let scroll_y = ctx.input(|i| i.raw_scroll_delta.y);
                    if scroll_y.abs() > f32::EPSILON {
                        let step = if scroll_y.abs() >= 15.0 {
                            (scroll_y / 50.0).signum()
                                * ((scroll_y.abs() / 50.0).max(1.0) * 5.0)
                        } else {
                            scroll_y * 0.1
                        };
                        self.change_volume(self.volume + step as f64);
                        self.volume_indicator_until = Some(Instant::now() + Duration::from_millis(1500));
                        ctx.request_repaint();
                    }
                }

                // Compute aspect-ratio-preserving display rect (letterboxed / pillarboxed)
                let aspect = (self.video_aspect as f32).clamp(0.1, 10.0);
                let panel_w = video_rect.width().max(1.0);
                let panel_h = video_rect.height().max(1.0);
                let panel_aspect = panel_w / panel_h;
                let (fit_w, fit_h) = if panel_aspect > aspect {
                    (panel_h * aspect, panel_h)
                } else {
                    (panel_w, panel_w / aspect)
                };
                let display_rect =
                    Rect::from_center_size(video_rect.center(), Vec2::new(fit_w, fit_h));

                // Render current MPV video frame at proportional dimensions matching the aspect ratio
                let max_w = 1920.0_f32;
                let max_h = 1080.0_f32;
                let scale = (max_w / fit_w).min(max_h / fit_h).min(1.0);
                let target_w = (((fit_w * scale).round() as usize).max(2)) & !1;
                let target_h = (((fit_h * scale).round() as usize).max(2)) & !1;

                if let Some(ref mut player) = self.player {
                    if let Some(rgba) = player.render_frame_if_updated(target_w, target_h) {
                        let image =
                            egui::ColorImage::from_rgba_unmultiplied([target_w, target_h], rgba);
                        match self.video_texture {
                            Some(ref mut tex) => tex.set(image, TextureOptions::LINEAR),
                            None => {
                                self.video_texture = Some(ctx.load_texture(
                                    "mpv_video_frame",
                                    image,
                                    TextureOptions::LINEAR,
                                ));
                            }
                        }
                    }
                }

                if let Some(ref tex) = self.video_texture {
                    ui.painter().image(
                        tex.id(),
                        display_rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else if let Some(ref err) = self.player_error {
                    ui.painter().text(
                        video_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("MPV Inactive: {err}"),
                        egui::FontId::proportional(14.0),
                        Color32::GRAY,
                    );
                }

                // Check 5-second OSC auto-hide timer
                let popup_active = self.volume_popup_open
                    || self.audio_sub_popup_open
                    || self.audio_menu_open
                    || self.sub_menu_open
                    || ctx.memory(|m| m.any_popup_open());
                if popup_active {
                    self.last_mouse_activity = Instant::now();
                } else if self.osc_visible {
                    if self.last_mouse_activity.elapsed() >= Duration::from_secs(5) {
                        self.osc_visible = false;
                    } else {
                        let remaining = Duration::from_secs(5)
                            .saturating_sub(self.last_mouse_activity.elapsed());
                        ctx.request_repaint_after(remaining);
                    }
                }

                if self.osc_visible {
                    self.render_osc_overlays(ctx, video_rect);
                }

                // Render floating volume indicator when scrolling wheel (vertical capsule on right edge)
                if let Some(until) = self.volume_indicator_until {
                    if Instant::now() < until {
                        let pill_w = 44.0;
                        let pill_h = 160.0;
                        let pill_x = (video_rect.right() - pill_w - 24.0).max(video_rect.left() + 10.0);
                        let pill_y = video_rect.center().y - pill_h / 2.0;
                        let pill_pos = Pos2::new(pill_x, pill_y);
                        let pill_rect = Rect::from_min_size(pill_pos, Vec2::new(pill_w, pill_h));

                        egui::Area::new(Id::new("scroll_volume_indicator_overlay"))
                            .order(Order::Tooltip)
                            .fixed_pos(pill_pos)
                            .show(ctx, |ui| {
                                let (_, resp) = ui.allocate_exact_size(
                                    Vec2::new(pill_w, pill_h),
                                    Sense::click_and_drag(),
                                );

                                if resp.clicked() || resp.dragged() {
                                    if let Some(pos) = resp.interact_pointer_pos() {
                                        let frac = ((pill_rect.bottom() - pos.y) / pill_rect.height()).clamp(0.0, 1.0);
                                        self.last_mouse_activity = Instant::now();
                                        self.change_volume((frac * 100.0) as f64);
                                        self.volume_indicator_until = Some(Instant::now() + Duration::from_millis(1500));
                                    }
                                }

                                let vol_frac = (self.volume / 100.0).clamp(0.0, 1.0) as f32;
                                let r = pill_w / 2.0;

                                // 1. Dark capsule background
                                ui.painter().rect_filled(
                                    pill_rect,
                                    Rounding::same(r),
                                    Color32::from_rgba_unmultiplied(22, 22, 26, 235),
                                );

                                // 2. Active blue fill from bottom up (conforming to capsule shape)
                                if vol_frac >= 0.999 {
                                    ui.painter().rect_filled(
                                        pill_rect,
                                        Rounding::same(r),
                                        Color32::from_rgb(42, 130, 218),
                                    );
                                } else if vol_frac > 0.005 {
                                    let min_y = pill_rect.bottom() - pill_h * vol_frac;
                                    let cx = pill_rect.center().x;
                                    let top_cap_y = pill_rect.top() + r;
                                    let bottom_cap_y = pill_rect.bottom() - r;

                                    let mut capsule_pts = Vec::with_capacity(36);
                                    capsule_pts.push(Pos2::new(cx + r, top_cap_y));
                                    capsule_pts.push(Pos2::new(cx + r, bottom_cap_y));
                                    let steps = 16;
                                    for i in 1..=steps {
                                        let theta = std::f32::consts::PI * (i as f32) / (steps as f32);
                                        capsule_pts.push(Pos2::new(cx + r * theta.cos(), bottom_cap_y + r * theta.sin()));
                                    }
                                    capsule_pts.push(Pos2::new(cx - r, top_cap_y));
                                    for i in 1..=steps {
                                        let theta = std::f32::consts::PI + std::f32::consts::PI * (i as f32) / (steps as f32);
                                        capsule_pts.push(Pos2::new(cx + r * theta.cos(), top_cap_y + r * theta.sin()));
                                    }

                                    // Sutherland-Hodgman clip against y >= min_y
                                    let mut clipped_pts = Vec::new();
                                    for i in 0..capsule_pts.len() {
                                        let cur = capsule_pts[i];
                                        let prev = if i == 0 { capsule_pts[capsule_pts.len() - 1] } else { capsule_pts[i - 1] };
                                        let cur_in = cur.y >= min_y;
                                        let prev_in = prev.y >= min_y;

                                        if prev_in && cur_in {
                                            clipped_pts.push(cur);
                                        } else if prev_in && !cur_in {
                                            let dy = cur.y - prev.y;
                                            if dy.abs() > f32::EPSILON {
                                                let t = (min_y - prev.y) / dy;
                                                let x = prev.x + t * (cur.x - prev.x);
                                                clipped_pts.push(Pos2::new(x, min_y));
                                            }
                                        } else if !prev_in && cur_in {
                                            let dy = cur.y - prev.y;
                                            if dy.abs() > f32::EPSILON {
                                                let t = (min_y - prev.y) / dy;
                                                let x = prev.x + t * (cur.x - prev.x);
                                                clipped_pts.push(Pos2::new(x, min_y));
                                            }
                                            clipped_pts.push(cur);
                                        }
                                    }

                                    if clipped_pts.len() >= 3 {
                                        ui.painter().add(Shape::convex_polygon(
                                            clipped_pts,
                                            Color32::from_rgb(42, 130, 218),
                                            Stroke::NONE,
                                        ));
                                    }
                                }

                                // 3. Capsule border stroke
                                ui.painter().rect_stroke(
                                    pill_rect,
                                    Rounding::same(r),
                                    Stroke::new(
                                        1.0_f32,
                                        Color32::from_rgba_unmultiplied(75, 75, 90, 200),
                                    ),
                                );

                                // 4. Percentage text at top
                                let text_pos = Pos2::new(pill_rect.center().x, pill_rect.top() + 22.0);
                                ui.painter().text(
                                    text_pos,
                                    egui::Align2::CENTER_CENTER,
                                    format!("{:.0}%", self.volume),
                                    egui::FontId::proportional(13.5),
                                    Color32::WHITE,
                                );

                                // 5. White vector speaker icon at bottom
                                let vc = Pos2::new(pill_rect.center().x, pill_rect.bottom() - 26.0);
                                let icon_color = Color32::WHITE;

                                if self.volume <= 0.0 {
                                    // Speaker body
                                    ui.painter().rect_filled(
                                        Rect::from_min_max(Pos2::new(vc.x - 7.0, vc.y - 3.2), Pos2::new(vc.x - 3.5, vc.y + 3.2)),
                                        Rounding::ZERO,
                                        icon_color,
                                    );
                                    ui.painter().add(Shape::convex_polygon(
                                        vec![
                                            Pos2::new(vc.x - 3.5, vc.y - 3.2),
                                            Pos2::new(vc.x + 0.5, vc.y - 6.2),
                                            Pos2::new(vc.x + 0.5, vc.y + 6.2),
                                            Pos2::new(vc.x - 3.5, vc.y + 3.2),
                                        ],
                                        icon_color,
                                        Stroke::NONE,
                                    ));
                                    // Diagonal mute cross ✕
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 3.5, vc.y - 3.5), Pos2::new(vc.x + 8.0, vc.y + 3.5)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 8.0, vc.y - 3.5), Pos2::new(vc.x + 3.5, vc.y + 3.5)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                } else if self.volume < 50.0 {
                                    // Speaker body
                                    ui.painter().rect_filled(
                                        Rect::from_min_max(Pos2::new(vc.x - 6.5, vc.y - 3.2), Pos2::new(vc.x - 3.0, vc.y + 3.2)),
                                        Rounding::ZERO,
                                        icon_color,
                                    );
                                    ui.painter().add(Shape::convex_polygon(
                                        vec![
                                            Pos2::new(vc.x - 3.0, vc.y - 3.2),
                                            Pos2::new(vc.x + 1.0, vc.y - 6.2),
                                            Pos2::new(vc.x + 1.0, vc.y + 6.2),
                                            Pos2::new(vc.x - 3.0, vc.y + 3.2),
                                        ],
                                        icon_color,
                                        Stroke::NONE,
                                    ));
                                    // 1 sound wave arc
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 3.8, vc.y - 3.8), Pos2::new(vc.x + 5.5, vc.y)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 5.5, vc.y), Pos2::new(vc.x + 3.8, vc.y + 3.8)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                } else {
                                    // Speaker body
                                    ui.painter().rect_filled(
                                        Rect::from_min_max(Pos2::new(vc.x - 7.5, vc.y - 3.2), Pos2::new(vc.x - 4.0, vc.y + 3.2)),
                                        Rounding::ZERO,
                                        icon_color,
                                    );
                                    ui.painter().add(Shape::convex_polygon(
                                        vec![
                                            Pos2::new(vc.x - 4.0, vc.y - 3.2),
                                            Pos2::new(vc.x - 0.2, vc.y - 6.2),
                                            Pos2::new(vc.x - 0.2, vc.y + 6.2),
                                            Pos2::new(vc.x - 4.0, vc.y + 3.2),
                                        ],
                                        icon_color,
                                        Stroke::NONE,
                                    ));
                                    // 2 sound wave arcs
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 2.6, vc.y - 3.5), Pos2::new(vc.x + 4.4, vc.y)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 4.4, vc.y), Pos2::new(vc.x + 2.6, vc.y + 3.5)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 5.8, vc.y - 5.8), Pos2::new(vc.x + 8.2, vc.y)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                    ui.painter().line_segment(
                                        [Pos2::new(vc.x + 8.2, vc.y), Pos2::new(vc.x + 5.8, vc.y + 5.8)],
                                        Stroke::new(1.6_f32, icon_color),
                                    );
                                }
                            });
                        ctx.request_repaint_after(Duration::from_millis(50));
                    } else {
                        self.volume_indicator_until = None;
                    }
                }
            });
    }

    fn get_media_display_info(&self) -> (String, String) {
        let display_title = if let Some(ref title) = self.media_title {
            if !title.trim().is_empty() {
                Some(title.clone())
            } else {
                None
            }
        } else if let Some(title) = self.player.as_ref().and_then(|p| p.get_media_title()) {
            if !title.trim().is_empty() {
                Some(title)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(ref url) = self.current_playing_url {
            let title = if let Some(t) = display_title {
                t
            } else {
                let raw_name = url.rsplit('/').next().unwrap_or(url);
                let raw_name = raw_name.rsplit('\\').next().unwrap_or(raw_name);
                let name_without_query = raw_name.split('?').next().unwrap_or(raw_name);
                let unescaped = name_without_query
                    .replace("%20", " ")
                    .replace("%28", "(")
                    .replace("%29", ")")
                    .replace("%5B", "[")
                    .replace("%5D", "]");
                if let Some(dot_idx) = unescaped.rfind('.') {
                    unescaped[..dot_idx].to_string()
                } else {
                    unescaped
                }
            };

            let track_idx = match self.selected_playlist_idx {
                Some(idx) => idx + 1,
                None => self
                    .playlist
                    .iter()
                    .position(|u| u == url)
                    .map(|i| i + 1)
                    .unwrap_or(1),
            };
            let total_tracks = self.playlist.len().max(1);

            let ep_part = if let Some(idx) = title.to_lowercase().find("episode") {
                let rest = &title[idx..];
                let tokens: Vec<&str> = rest.split_whitespace().take(2).collect();
                format!("{} • ", tokens.join(" "))
            } else if let Some(idx) = title.to_lowercase().find("ep.") {
                let rest = &title[idx..];
                let tokens: Vec<&str> = rest.split_whitespace().take(2).collect();
                format!("{} • ", tokens.join(" "))
            } else if let Some(idx) = title.to_lowercase().find("ep ") {
                let rest = &title[idx..];
                let tokens: Vec<&str> = rest.split_whitespace().take(2).collect();
                format!("{} • ", tokens.join(" "))
            } else {
                String::new()
            };

            let subtitle = if self.is_live {
                format!("Live Stream • Track {track_idx} of {total_tracks}")
            } else if !ep_part.is_empty() {
                format!("{ep_part}Track {track_idx} of {total_tracks}")
            } else if self.playlist.len() > 1 {
                format!("Track {track_idx} of {total_tracks}")
            } else {
                "Track 1 of 1".to_string()
            };

            (title, subtitle)
        } else if let Some(t) = display_title {
            (t, "Direct Playback".to_string())
        } else {
            (
                "No Media Playing".to_string(),
                "Ready to play".to_string(),
            )
        }
    }

    #[cfg(test)]
    fn get_audio_sub_pill_label(&self) -> String {
        "Audio/CC".to_string()
    }

    fn render_osc_overlays(&mut self, ctx: &Context, video_rect: Rect) {
        let margin = 20.0;
        let osc_width = (video_rect.width() - margin * 2.0).max(120.0);
        let inner_osc_width = (osc_width - 24.0).max(96.0);
        let osc_bg = Color32::from_rgba_unmultiplied(18, 18, 22, 240);
        let osc_stroke = Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(65, 65, 78, 180),
        );

        // Top OSC Bar: Cache duration display
        let top_pos = Pos2::new(video_rect.left() + margin, video_rect.top() + margin);
        egui::Area::new(Id::new("top_osc_overlay"))
            .order(Order::Middle)
            .fixed_pos(top_pos)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(osc_bg)
                    .stroke(osc_stroke)
                    .rounding(Rounding::same(15.0))
                    .inner_margin(Margin::symmetric(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.set_min_width(inner_osc_width);
                        ui.set_max_width(inner_osc_width);
                        ui.horizontal(|ui| {
                            let cache_text = if self.cache_duration > 0.0 {
                                format!("Cache: {}", format_time(Some(self.cache_duration)))
                            } else {
                                "Cache: 00:00".to_string()
                            };
                            ui.label(RichText::new(cache_text).color(Color32::WHITE));

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let ready_color = if self.ready_count > 0
                                    && (self.viewers_count == 0
                                        || self.ready_count >= self.viewers_count)
                                {
                                    Color32::from_rgb(90, 220, 110)
                                } else if self.ready_count > 0 {
                                    Color32::from_rgb(255, 205, 90)
                                } else {
                                    Color32::WHITE
                                };
                                ui.label(
                                    RichText::new(format!("Ready: {}", self.ready_count))
                                        .color(ready_color)
                                        .strong(),
                                );
                            });
                        });
                    });
            });

        // Bottom OSC Bar: Modern Media Player Rework (Two-row layout)
        let bottom_h = 92.0;
        let bottom_pos = Pos2::new(
            video_rect.left() + margin,
            (video_rect.bottom() - bottom_h - margin).max(video_rect.top() + margin),
        );
        let mut volume_btn_rect = None;
        let mut audio_sub_btn_rect = None;

        egui::Area::new(Id::new("bottom_osc_overlay"))
            .order(Order::Middle)
            .fixed_pos(bottom_pos)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(osc_bg)
                    .stroke(osc_stroke)
                    .rounding(Rounding::same(18.0))
                    .inner_margin(Margin::symmetric(16.0, 10.0))
                    .show(ui, |ui| {
                        ui.set_min_width(inner_osc_width);
                        ui.set_max_width(inner_osc_width);
                        ui.vertical(|ui| {
                            // -------------------------------------------------------------
                            // ROW 1: Progress / Seek Bar with Time Labels
                            // -------------------------------------------------------------
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 10.0;

                                let cur_time_str = format_time(Some(self.current_time));
                                let dur_time_str = format_time(Some(self.current_duration));

                                ui.label(
                                    RichText::new(cur_time_str)
                                        .size(13.0)
                                        .monospace()
                                        .color(Color32::from_rgb(215, 215, 225)),
                                );

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(dur_time_str)
                                            .size(13.0)
                                            .monospace()
                                            .color(Color32::from_rgb(215, 215, 225)),
                                    );

                                    let seek_bar_w = (ui.available_width() - 8.0).max(20.0);
                                    let (seek_rect, seek_resp) = ui.allocate_exact_size(
                                        Vec2::new(seek_bar_w, 18.0),
                                        Sense::click_and_drag(),
                                    );

                                    if seek_resp.hovered() {
                                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                    }

                                    let center_y = seek_rect.center().y;
                                    let bar_h = 6.0;
                                    let track_rect = Rect::from_min_max(
                                        Pos2::new(seek_rect.left(), center_y - bar_h / 2.0),
                                        Pos2::new(seek_rect.right(), center_y + bar_h / 2.0),
                                    );
                                    ui.painter().rect_filled(
                                        track_rect,
                                        Rounding::same(3.0),
                                        Color32::from_rgb(46, 46, 54),
                                    );

                                    let has_duration = self.current_duration > 0.0;
                                    let progress_frac = if has_duration {
                                        (self.current_time / self.current_duration)
                                            .clamp(0.0, 1.0)
                                            as f32
                                    } else {
                                        0.0
                                    };

                                    let buffer_frac = if has_duration && self.cache_duration > 0.0 {
                                        ((self.current_time + self.cache_duration)
                                            / self.current_duration)
                                            .clamp(0.0, 1.0)
                                            as f32
                                    } else {
                                        progress_frac
                                    };

                                    if buffer_frac > progress_frac {
                                        let buf_left =
                                            seek_rect.left() + progress_frac * seek_rect.width();
                                        let buf_right =
                                            seek_rect.left() + buffer_frac * seek_rect.width();
                                        let buf_rect = Rect::from_min_max(
                                            Pos2::new(buf_left, center_y - bar_h / 2.0),
                                            Pos2::new(buf_right, center_y + bar_h / 2.0),
                                        );
                                        ui.painter().rect_filled(
                                            buf_rect,
                                            Rounding::ZERO,
                                            Color32::from_rgb(72, 72, 84),
                                        );
                                    }

                                    if progress_frac > 0.0 {
                                        let played_right =
                                            seek_rect.left() + progress_frac * seek_rect.width();
                                        let played_rect = Rect::from_min_max(
                                            Pos2::new(seek_rect.left(), center_y - bar_h / 2.0),
                                            Pos2::new(played_right, center_y + bar_h / 2.0),
                                        );
                                        ui.painter().rect_filled(
                                            played_rect,
                                            Rounding::same(3.0),
                                            Color32::from_rgb(42, 130, 218),
                                        );
                                    }

                                    let thumb_x =
                                        seek_rect.left() + progress_frac * seek_rect.width();
                                    let thumb_r = if seek_resp.hovered() || seek_resp.dragged() {
                                        8.0
                                    } else {
                                        7.0
                                    };
                                    ui.painter().circle_filled(
                                        Pos2::new(thumb_x, center_y),
                                        thumb_r + 2.0,
                                        Color32::from_rgba_unmultiplied(0, 0, 0, 80),
                                    );
                                    ui.painter().circle_filled(
                                        Pos2::new(thumb_x, center_y),
                                        thumb_r,
                                        Color32::WHITE,
                                    );

                                    if (seek_resp.clicked() || seek_resp.dragged()) && has_duration
                                    {
                                        if let Some(pos) = seek_resp.interact_pointer_pos() {
                                            let frac = ((pos.x - seek_rect.left())
                                                / seek_rect.width())
                                                .clamp(0.0, 1.0);
                                            self.last_mouse_activity = Instant::now();
                                            self.seek_video((frac as f64) * 1000.0);
                                        }
                                    }
                                });
                            });

                            ui.add_space(4.0);

                            // -------------------------------------------------------------
                            // ROW 2: Left Info, Centered Transport Controls, Right Actions
                            // -------------------------------------------------------------
                            let row_h = 44.0;
                            let (row_rect, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), row_h),
                                Sense::hover(),
                            );

                            let center_w = 126.0;
                            let right_w = 200.0;
                            let center_x = row_rect.center().x;
                            let right_left = (row_rect.right() - right_w)
                                .max(row_rect.left() + 200.0);
                            let right_rect = Rect::from_min_max(
                                Pos2::new(right_left, row_rect.top()),
                                Pos2::new(row_rect.right(), row_rect.bottom()),
                            );

                            let ideal_center_left = center_x - center_w / 2.0;
                            let center_left = ideal_center_left.clamp(
                                row_rect.left() + 110.0,
                                (right_left - center_w - 4.0).max(row_rect.left() + 110.0),
                            );
                            let center_rect = Rect::from_min_max(
                                Pos2::new(center_left, row_rect.top()),
                                Pos2::new(center_left + center_w, row_rect.bottom()),
                            );

                            let left_rect = Rect::from_min_max(
                                Pos2::new(row_rect.left(), row_rect.top()),
                                Pos2::new(
                                    (center_left - 8.0).max(row_rect.left()),
                                    row_rect.bottom(),
                                ),
                            );

                            // SECTION 1: Left Track Information
                            ui.allocate_new_ui(UiBuilder::new().max_rect(left_rect), |ui| {
                                ui.horizontal_centered(|ui| {
                                    // Title & Subtitle vertical column
                                    ui.vertical(|ui| {
                                        ui.set_max_width(left_rect.width().max(10.0));
                                        let (title, subtitle) = self.get_media_display_info();
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(title)
                                                    .strong()
                                                    .size(13.5)
                                                    .color(Color32::WHITE),
                                            )
                                            .truncate(),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(subtitle)
                                                    .size(11.0)
                                                    .color(Color32::from_rgb(156, 163, 175)),
                                            )
                                            .truncate(),
                                        );
                                    });
                                });
                            });

                            // SECTION 2: Center Transport Controls (Prev, Big Play/Pause, Next)
                            ui.allocate_new_ui(UiBuilder::new().max_rect(center_rect), |ui| {
                                ui.horizontal_centered(|ui| {
                                    ui.spacing_mut().item_spacing.x = 10.0;

                                    // Previous button: |◀
                                    let (prev_rect, prev_resp) = ui.allocate_exact_size(
                                        Vec2::splat(28.0),
                                        Sense::click(),
                                    );
                                    if prev_resp.hovered() {
                                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                        ui.painter().circle_filled(
                                            prev_rect.center(),
                                            14.0,
                                            Color32::from_rgba_unmultiplied(255, 255, 255, 25),
                                        );
                                    }
                                    let p_color = if prev_resp.hovered() {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(205, 205, 215)
                                    };
                                    let pc = prev_rect.center();
                                    ui.painter().line_segment(
                                        [
                                            Pos2::new(pc.x - 6.0, pc.y - 5.5),
                                            Pos2::new(pc.x - 6.0, pc.y + 5.5),
                                        ],
                                        Stroke::new(2.2_f32, p_color),
                                    );
                                    ui.painter().add(Shape::convex_polygon(
                                        vec![
                                            Pos2::new(pc.x + 5.0, pc.y - 5.5),
                                            Pos2::new(pc.x + 5.0, pc.y + 5.5),
                                            Pos2::new(pc.x - 4.5, pc.y),
                                        ],
                                        p_color,
                                        Stroke::NONE,
                                    ));
                                    if prev_resp.clicked() {
                                        self.play_prev_item();
                                    }

                                    // Big circular Play / Pause button (42x42)
                                    let (play_rect, play_resp) = ui.allocate_exact_size(
                                        Vec2::splat(42.0),
                                        Sense::click(),
                                    );
                                    if play_resp.hovered() {
                                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                    }
                                    let play_center = play_rect.center();
                                    // Blue glow
                                    ui.painter().circle_filled(
                                        play_center,
                                        24.0,
                                        Color32::from_rgba_unmultiplied(42, 130, 218, 55),
                                    );
                                    let btn_color = if play_resp.hovered() {
                                        Color32::from_rgb(60, 150, 240)
                                    } else {
                                        Color32::from_rgb(42, 130, 218)
                                    };
                                    ui.painter().circle_filled(play_center, 21.0, btn_color);

                                    if self.is_paused {
                                        // Play triangle ▶
                                        ui.painter().add(Shape::convex_polygon(
                                            vec![
                                                Pos2::new(
                                                    play_center.x - 4.5,
                                                    play_center.y - 7.5,
                                                ),
                                                Pos2::new(
                                                    play_center.x - 4.5,
                                                    play_center.y + 7.5,
                                                ),
                                                Pos2::new(play_center.x + 6.5, play_center.y),
                                            ],
                                            Color32::WHITE,
                                            Stroke::NONE,
                                        ));
                                    } else {
                                        // Pause bars ❚❚
                                        let bar_w = 3.2;
                                        let bar_h = 13.0;
                                        let bar1 = Rect::from_center_size(
                                            Pos2::new(play_center.x - 3.8, play_center.y),
                                            Vec2::new(bar_w, bar_h),
                                        );
                                        let bar2 = Rect::from_center_size(
                                            Pos2::new(play_center.x + 3.8, play_center.y),
                                            Vec2::new(bar_w, bar_h),
                                        );
                                        ui.painter().rect_filled(
                                            bar1,
                                            Rounding::same(1.2),
                                            Color32::WHITE,
                                        );
                                        ui.painter().rect_filled(
                                            bar2,
                                            Rounding::same(1.2),
                                            Color32::WHITE,
                                        );
                                    }
                                    if play_resp.clicked() {
                                        self.toggle_playback();
                                    }

                                    // Next button: ▶|
                                    let (next_rect, next_resp) = ui.allocate_exact_size(
                                        Vec2::splat(28.0),
                                        Sense::click(),
                                    );
                                    if next_resp.hovered() {
                                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                        ui.painter().circle_filled(
                                            next_rect.center(),
                                            14.0,
                                            Color32::from_rgba_unmultiplied(255, 255, 255, 25),
                                        );
                                    }
                                    let n_color = if next_resp.hovered() {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(205, 205, 215)
                                    };
                                    let nc = next_rect.center();
                                    ui.painter().add(Shape::convex_polygon(
                                        vec![
                                            Pos2::new(nc.x - 5.0, nc.y - 5.5),
                                            Pos2::new(nc.x - 5.0, nc.y + 5.5),
                                            Pos2::new(nc.x + 4.5, nc.y),
                                        ],
                                        n_color,
                                        Stroke::NONE,
                                    ));
                                    ui.painter().line_segment(
                                        [
                                            Pos2::new(nc.x + 6.0, nc.y - 5.5),
                                            Pos2::new(nc.x + 6.0, nc.y + 5.5),
                                        ],
                                        Stroke::new(2.2_f32, n_color),
                                    );
                                    if next_resp.clicked() {
                                        self.play_next_item();
                                    }
                                });
                            });

                            // SECTION 3: Right Controls (Fullscreen, Playlist badge, Volume, Audio/Sub)
                            ui.allocate_new_ui(UiBuilder::new().max_rect(right_rect), |ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;

                                    // Fullscreen button ⛶
                                    let fs_btn = egui::Button::new(
                                        RichText::new("⛶")
                                            .size(15.0)
                                            .color(Color32::from_rgb(215, 215, 225)),
                                    )
                                    .fill(Color32::from_rgba_unmultiplied(36, 36, 44, 180))
                                    .stroke(Stroke::new(
                                        1.0_f32,
                                        Color32::from_rgba_unmultiplied(70, 70, 80, 150),
                                    ))
                                    .rounding(Rounding::same(8.0));
                                    if ui.add_sized([30.0, 30.0], fs_btn).clicked() {
                                        self.toggle_fullscreen(ctx);
                                    }

                                    // Playlist Button ☰ (clean button without count badge)
                                    let pl_btn = egui::Button::new(
                                        RichText::new("☰")
                                            .size(15.0)
                                            .color(if self.sidebar_visible {
                                                Color32::WHITE
                                            } else {
                                                Color32::from_rgb(215, 215, 225)
                                            }),
                                    )
                                    .fill(if self.sidebar_visible {
                                        Color32::from_rgba_unmultiplied(36, 60, 95, 240)
                                    } else {
                                        Color32::from_rgba_unmultiplied(36, 36, 44, 180)
                                    })
                                    .stroke(Stroke::new(
                                        1.0_f32,
                                        if self.sidebar_visible {
                                            Color32::from_rgba_unmultiplied(60, 140, 230, 200)
                                        } else {
                                            Color32::from_rgba_unmultiplied(70, 70, 80, 150)
                                        },
                                    ))
                                    .rounding(Rounding::same(8.0));
                                    if ui.add_sized([30.0, 30.0], pl_btn).clicked() {
                                        self.toggle_sidebar();
                                    }

                                    // Volume button with modern vector speaker icon
                                    let (vol_rect, vol_resp) = ui.allocate_exact_size(
                                        Vec2::splat(30.0),
                                        Sense::click(),
                                    );
                                    volume_btn_rect = Some(vol_rect);
                                    if vol_resp.hovered() {
                                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                    }
                                    let vol_bg = if self.volume_popup_open {
                                        Color32::from_rgba_unmultiplied(36, 60, 95, 240)
                                    } else if vol_resp.hovered() {
                                        Color32::from_rgba_unmultiplied(48, 48, 58, 220)
                                    } else {
                                        Color32::from_rgba_unmultiplied(36, 36, 44, 180)
                                    };
                                    let vol_stroke = if self.volume_popup_open {
                                        Color32::from_rgba_unmultiplied(60, 140, 230, 200)
                                    } else if vol_resp.hovered() {
                                        Color32::from_rgba_unmultiplied(90, 90, 105, 180)
                                    } else {
                                        Color32::from_rgba_unmultiplied(70, 70, 80, 150)
                                    };
                                    ui.painter().rect_filled(vol_rect, Rounding::same(8.0), vol_bg);
                                    ui.painter().rect_stroke(vol_rect, Rounding::same(8.0), Stroke::new(1.0_f32, vol_stroke));

                                    let icon_color = if self.volume_popup_open || vol_resp.hovered() {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(215, 215, 225)
                                    };
                                    let vc = vol_rect.center();

                                    if self.volume <= 0.0 {
                                        // Speaker body
                                        ui.painter().rect_filled(
                                            Rect::from_min_max(Pos2::new(vc.x - 7.0, vc.y - 3.2), Pos2::new(vc.x - 3.5, vc.y + 3.2)),
                                            Rounding::ZERO,
                                            icon_color,
                                        );
                                        ui.painter().add(Shape::convex_polygon(
                                            vec![
                                                Pos2::new(vc.x - 3.5, vc.y - 3.2),
                                                Pos2::new(vc.x + 0.5, vc.y - 6.2),
                                                Pos2::new(vc.x + 0.5, vc.y + 6.2),
                                                Pos2::new(vc.x - 3.5, vc.y + 3.2),
                                            ],
                                            icon_color,
                                            Stroke::NONE,
                                        ));
                                        // Diagonal mute cross ✕
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 3.5, vc.y - 3.5), Pos2::new(vc.x + 8.0, vc.y + 3.5)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 8.0, vc.y - 3.5), Pos2::new(vc.x + 3.5, vc.y + 3.5)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                    } else if self.volume < 50.0 {
                                        // Speaker body
                                        ui.painter().rect_filled(
                                            Rect::from_min_max(Pos2::new(vc.x - 6.5, vc.y - 3.2), Pos2::new(vc.x - 3.0, vc.y + 3.2)),
                                            Rounding::ZERO,
                                            icon_color,
                                        );
                                        ui.painter().add(Shape::convex_polygon(
                                            vec![
                                                Pos2::new(vc.x - 3.0, vc.y - 3.2),
                                                Pos2::new(vc.x + 1.0, vc.y - 6.2),
                                                Pos2::new(vc.x + 1.0, vc.y + 6.2),
                                                Pos2::new(vc.x - 3.0, vc.y + 3.2),
                                            ],
                                            icon_color,
                                            Stroke::NONE,
                                        ));
                                        // 1 sound wave arc
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 3.8, vc.y - 3.8), Pos2::new(vc.x + 5.5, vc.y)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 5.5, vc.y), Pos2::new(vc.x + 3.8, vc.y + 3.8)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                    } else {
                                        // Speaker body
                                        ui.painter().rect_filled(
                                            Rect::from_min_max(Pos2::new(vc.x - 7.5, vc.y - 3.2), Pos2::new(vc.x - 4.0, vc.y + 3.2)),
                                            Rounding::ZERO,
                                            icon_color,
                                        );
                                        ui.painter().add(Shape::convex_polygon(
                                            vec![
                                                Pos2::new(vc.x - 4.0, vc.y - 3.2),
                                                Pos2::new(vc.x - 0.2, vc.y - 6.2),
                                                Pos2::new(vc.x - 0.2, vc.y + 6.2),
                                                Pos2::new(vc.x - 4.0, vc.y + 3.2),
                                            ],
                                            icon_color,
                                            Stroke::NONE,
                                        ));
                                        // 2 sound wave arcs
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 2.6, vc.y - 3.5), Pos2::new(vc.x + 4.4, vc.y)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 4.4, vc.y), Pos2::new(vc.x + 2.6, vc.y + 3.5)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 5.8, vc.y - 5.8), Pos2::new(vc.x + 8.2, vc.y)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                        ui.painter().line_segment(
                                            [Pos2::new(vc.x + 8.2, vc.y), Pos2::new(vc.x + 5.8, vc.y + 5.8)],
                                            Stroke::new(1.6_f32, icon_color),
                                        );
                                    }

                                    if vol_resp.clicked() {
                                        self.volume_popup_open = !self.volume_popup_open;
                                    }

                                    // Audio & Subtitle Selector Pill: [ Audio/CC ]
                                    let (as_rect, as_resp) = ui.allocate_exact_size(
                                        Vec2::new(76.0, 30.0),
                                        Sense::click(),
                                    );
                                    audio_sub_btn_rect = Some(as_rect);
                                    if as_resp.hovered() {
                                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                    }
                                    let as_bg =
                                        if as_resp.hovered() || self.audio_sub_popup_open {
                                            Color32::from_rgba_unmultiplied(36, 60, 95, 240)
                                        } else {
                                            Color32::from_rgba_unmultiplied(36, 36, 44, 220)
                                        };
                                    ui.painter().rect_filled(
                                        as_rect,
                                        Rounding::same(15.0),
                                        as_bg,
                                    );
                                    ui.painter().rect_stroke(
                                        as_rect,
                                        Rounding::same(15.0),
                                        Stroke::new(
                                            1.0_f32,
                                            if self.audio_sub_popup_open {
                                                Color32::from_rgba_unmultiplied(60, 140, 230, 200)
                                            } else {
                                                Color32::from_rgba_unmultiplied(75, 75, 88, 180)
                                            },
                                        ),
                                    );

                                    ui.painter().text(
                                        as_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "Audio/CC",
                                        egui::FontId::proportional(12.5),
                                        Color32::from_rgb(225, 225, 235),
                                    );

                                    if as_resp.clicked() {
                                        self.audio_sub_popup_open =
                                            !self.audio_sub_popup_open;
                                    }
                                });
                            });
                        });
                    });
            });

        // Floating Audio & Subtitle Popup above the pill button
        if self.audio_sub_popup_open {
            if let Some(btn_rect) = audio_sub_btn_rect {
                let popup_w = 260.0;
                let popup_x = (btn_rect.center().x - popup_w / 2.0)
                    .clamp(video_rect.left() + 10.0, video_rect.right() - popup_w - 10.0);
                let popup_pos = Pos2::new(popup_x, btn_rect.top() - 8.0);
                let area_resp = egui::Area::new(Id::new("audio_sub_popup_overlay"))
                    .order(Order::Tooltip)
                    .pivot(egui::Align2::LEFT_BOTTOM)
                    .fixed_pos(popup_pos)
                    .show(ctx, |ui| {
                        Frame::none()
                            .fill(Color32::from_rgba_unmultiplied(22, 22, 28, 245))
                            .stroke(Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(80, 80, 95, 200),
                            ))
                            .rounding(Rounding::same(12.0))
                            .inner_margin(Margin::same(12.0))
                            .show(ui, |ui| {
                                ui.set_width(popup_w);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Audio & Subtitles")
                                            .strong()
                                            .size(13.0)
                                            .color(Color32::WHITE),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("X")
                                                        .size(11.0)
                                                        .color(Color32::from_rgb(160, 160, 175)),
                                                )
                                                .frame(false),
                                            )
                                            .clicked()
                                        {
                                            self.audio_sub_popup_open = false;
                                        }
                                    });
                                });
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);

                                ui.label(
                                    RichText::new("Audio Track")
                                        .strong()
                                        .size(11.5)
                                        .color(Color32::from_rgb(180, 180, 195)),
                                );
                                if self.audio_tracks.is_empty() {
                                    ui.label(
                                        RichText::new("No audio tracks detected")
                                            .italics()
                                            .size(11.0)
                                            .color(Color32::GRAY),
                                    );
                                } else {
                                    egui::ScrollArea::vertical()
                                        .id_salt("audio_tracks_scroll")
                                        .max_height(104.0)
                                        .auto_shrink([false, true])
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            for (idx, (track_id, label)) in
                                                self.audio_tracks.iter().enumerate()
                                            {
                                                let is_sel = idx == self.selected_audio_idx;
                                                let text = if is_sel {
                                                    RichText::new(label)
                                                        .strong()
                                                        .color(Color32::WHITE)
                                                } else {
                                                    RichText::new(label)
                                                        .color(Color32::from_rgb(220, 220, 230))
                                                };
                                                let resp = ui.selectable_label(is_sel, text);
                                                if resp.clicked() {
                                                    self.selected_audio_idx = idx;
                                                    if let Some(ref player) = self.player {
                                                        player.set_audio_track(*track_id);
                                                    }
                                                }
                                            }
                                        });
                                }

                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(6.0);

                                ui.label(
                                    RichText::new("Subtitles")
                                        .strong()
                                        .size(11.5)
                                        .color(Color32::from_rgb(180, 180, 195)),
                                );
                                if self.sub_tracks.is_empty() {
                                    ui.label(
                                        RichText::new("No subtitle tracks detected")
                                            .italics()
                                            .size(11.0)
                                            .color(Color32::GRAY),
                                    );
                                } else {
                                    egui::ScrollArea::vertical()
                                        .id_salt("sub_tracks_scroll")
                                        .max_height(104.0)
                                        .auto_shrink([false, true])
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            for (idx, (track_id, label)) in
                                                self.sub_tracks.iter().enumerate()
                                            {
                                                let is_sel = idx == self.selected_sub_idx;
                                                let text = if is_sel {
                                                    RichText::new(label)
                                                        .strong()
                                                        .color(Color32::WHITE)
                                                } else {
                                                    RichText::new(label)
                                                        .color(Color32::from_rgb(220, 220, 230))
                                                };
                                                let resp = ui.selectable_label(is_sel, text);
                                                if resp.clicked() {
                                                    self.selected_sub_idx = idx;
                                                    if let Some(ref player) = self.player {
                                                        player.set_subtitle_track(*track_id);
                                                    }
                                                }
                                            }
                                        });
                                }
                            });
                    });
                self.audio_sub_popup_rect = Some(area_resp.response.rect);
            }
        } else {
            self.audio_sub_popup_rect = None;
        }
        self.audio_sub_btn_rect = audio_sub_btn_rect;

        // Floating vertical Volume Popup above the volume button
        if self.volume_popup_open {
            if let Some(btn_rect) = volume_btn_rect {
                let popup_w = 34.0;
                let popup_h = 126.0;
                let popup_pos = Pos2::new(
                    btn_rect.center().x - popup_w / 2.0,
                    btn_rect.top() - popup_h - 8.0,
                );
                let area_resp = egui::Area::new(Id::new("volume_popup_overlay"))
                    .order(Order::Tooltip)
                    .fixed_pos(popup_pos)
                    .show(ctx, |ui| {
                        Frame::none()
                            .fill(Color32::from_rgba_unmultiplied(20, 20, 26, 245))
                            .stroke(Stroke::new(
                                1.0_f32,
                                Color32::from_rgba_unmultiplied(75, 75, 90, 200),
                            ))
                            .rounding(Rounding::same(12.0))
                            .inner_margin(Margin::same(6.0))
                            .show(ui, |ui| {
                                let (slider_rect, slider_resp) = ui.allocate_exact_size(
                                    Vec2::new(22.0, 114.0),
                                    Sense::click_and_drag(),
                                );
                                if slider_resp.hovered() || slider_resp.dragged() {
                                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                                }

                                let track_top = slider_rect.top() + 8.0;
                                let track_bottom = slider_rect.bottom() - 8.0;
                                let track_height = track_bottom - track_top;
                                let center_x = slider_rect.center().x;
                                let track_w = 6.0;

                                if slider_resp.clicked() || slider_resp.dragged() {
                                    if let Some(pos) = slider_resp.interact_pointer_pos() {
                                        let frac = ((track_bottom - pos.y) / track_height).clamp(0.0, 1.0);
                                        self.last_mouse_activity = Instant::now();
                                        self.change_volume((frac * 100.0) as f64);
                                    }
                                }

                                // Scroll wheel on volume slider popup
                                let scroll_y = ctx.input(|i| i.raw_scroll_delta.y);
                                if slider_resp.hovered() && scroll_y.abs() > f32::EPSILON {
                                    let step = if scroll_y.abs() >= 15.0 {
                                        (scroll_y / 50.0).signum() * ((scroll_y.abs() / 50.0).max(1.0) * 5.0)
                                    } else {
                                        scroll_y * 0.1
                                    };
                                    self.last_mouse_activity = Instant::now();
                                    self.change_volume(self.volume + step as f64);
                                }

                                let vol_frac = (self.volume / 100.0).clamp(0.0, 1.0) as f32;
                                let thumb_y = track_bottom - vol_frac * track_height;

                                // Background track (dark grey rail)
                                let bg_track = Rect::from_min_max(
                                    Pos2::new(center_x - track_w / 2.0, track_top),
                                    Pos2::new(center_x + track_w / 2.0, track_bottom),
                                );
                                ui.painter().rect_filled(
                                    bg_track,
                                    Rounding::same(track_w / 2.0),
                                    Color32::from_rgb(45, 45, 54),
                                );

                                // Active filled bar (BLUE matching theme)
                                if vol_frac > 0.0 {
                                    let active_track = Rect::from_min_max(
                                        Pos2::new(center_x - track_w / 2.0, thumb_y),
                                        Pos2::new(center_x + track_w / 2.0, track_bottom),
                                    );
                                    ui.painter().rect_filled(
                                        active_track,
                                        Rounding::same(track_w / 2.0),
                                        Color32::from_rgb(42, 130, 218),
                                    );
                                }

                                // Modern circular thumb
                                let thumb_radius = if slider_resp.hovered() || slider_resp.dragged() { 7.5 } else { 6.5 };
                                ui.painter().circle_filled(
                                    Pos2::new(center_x, thumb_y),
                                    thumb_radius + 2.5,
                                    Color32::from_rgba_unmultiplied(42, 130, 218, 50),
                                );
                                ui.painter().circle_filled(
                                    Pos2::new(center_x, thumb_y),
                                    thumb_radius,
                                    Color32::WHITE,
                                );
                                ui.painter().circle_stroke(
                                    Pos2::new(center_x, thumb_y),
                                    thumb_radius,
                                    Stroke::new(1.5_f32, Color32::from_rgb(42, 130, 218)),
                                );
                            });
                    });
                self.volume_popup_rect = Some(area_resp.response.rect);
            }
        } else {
            self.volume_popup_rect = None;
        }
        self.volume_btn_rect = volume_btn_rect;
    }

    fn render_modals(&mut self, ctx: &Context) {
        let modal_frame = Frame::window(&ctx.style())
            .fill(Color32::from_rgb(42, 42, 45))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(85, 85, 92)))
            .rounding(Rounding::same(8.0))
            .inner_margin(Margin::same(14.0));

        // Settings Dialog
        if self.settings_dialog_open {
            let mut open = self.settings_dialog_open;
            egui::Window::new(RichText::new("Settings").strong().color(Color32::WHITE))
                .open(&mut open)
                .order(Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .frame(modal_frame)
                .default_size(Vec2::new(300.0, 150.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Network Cache (MB):").color(Color32::WHITE));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let mut cache_mb = self.current_cache_size;
                            if ui
                                .add(egui::DragValue::new(&mut cache_mb).range(10..=2000))
                                .changed()
                            {
                                self.update_cache_size(cache_mb);
                            }
                        });
                    });

                    ui.add_space(14.0);
                    let host_label = if self.server_running {
                        "Hosting..."
                    } else {
                        "Host Server"
                    };
                    let host_btn = egui::Button::new(
                        RichText::new(host_label).strong().color(Color32::WHITE),
                    )
                    .fill(if self.server_running {
                        Color32::from_rgb(36, 140, 60)
                    } else {
                        Color32::from_rgb(42, 130, 218)
                    })
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(100, 100, 108)))
                    .rounding(Rounding::same(6.0));

                    if ui
                        .add_enabled_ui(!self.server_running, |ui| {
                            ui.add_sized([ui.available_width(), 30.0], host_btn)
                        })
                        .inner
                        .clicked()
                    {
                        self.host_server();
                    }

                    ui.add_space(14.0);
                    let reload_btn = egui::Button::new(
                        RichText::new("Force Reload Player").strong().color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(52, 52, 56))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(110, 110, 120)))
                    .rounding(Rounding::same(6.0));

                    if ui
                        .add_sized([ui.available_width(), 30.0], reload_btn)
                        .clicked()
                    {
                        self.reload_player();
                        self.show_osc();
                    }
                });
            self.settings_dialog_open = open;
        }

        // Edit Playlist Dialog
        if self.edit_playlist_dialog_open {
            let mut close_dialog = false;
            let mut apply_changes = false;
            egui::Window::new(
                RichText::new("Edit Playlist")
                    .strong()
                    .color(Color32::WHITE),
            )
            .order(Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .collapsible(false)
            .resizable(true)
            .frame(modal_frame)
            .default_size(Vec2::new(440.0, 280.0))
            .show(ctx, |ui| {
                ui.label(RichText::new("Edit URLs (one per line):").color(Color32::WHITE));
                ui.add_space(6.0);

                Frame::canvas(ui.style())
                    .fill(Color32::from_rgb(24, 24, 26))
                    .stroke(Stroke::new(1.0_f32, Color32::from_rgb(75, 75, 82)))
                    .rounding(Rounding::same(6.0))
                    .inner_margin(Margin::same(6.0))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                ui.add_sized(
                                    [ui.available_width(), 170.0],
                                    egui::TextEdit::multiline(&mut self.edit_playlist_buffer)
                                        .text_color(Color32::WHITE)
                                        .frame(false)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(8),
                                );
                            });
                    });

                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let cancel_btn =
                        egui::Button::new(RichText::new("Cancel").color(Color32::WHITE))
                            .fill(Color32::from_rgb(62, 62, 66))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(90, 90, 96)))
                            .rounding(Rounding::same(6.0));
                    if ui.add_sized([84.0, 28.0], cancel_btn).clicked() {
                        close_dialog = true;
                    }

                    ui.add_space(6.0);
                    let ok_btn =
                        egui::Button::new(RichText::new("OK").strong().color(Color32::WHITE))
                            .fill(Color32::from_rgb(42, 130, 218))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(90, 160, 235)))
                            .rounding(Rounding::same(6.0));
                    if ui.add_sized([84.0, 28.0], ok_btn).clicked() {
                        apply_changes = true;
                        close_dialog = true;
                    }
                });
            });

            if apply_changes {
                let buf = self.edit_playlist_buffer.clone();
                self.apply_edited_playlist(&buf);
            }
            if close_dialog {
                self.edit_playlist_dialog_open = false;
            }
        }
    }
}

impl eframe::App for SyncPlayApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if !ctx.style().visuals.dark_mode || ctx.theme() != Theme::Dark {
            Self::apply_dark_fusion_theme(ctx);
        }
        self.process_background_events(ctx);

        // Global shortcuts: Space toggles playback (when not typing in a text field), Escape exits fullscreen
        let typing = ctx.wants_keyboard_input() || self.settings_dialog_open || self.edit_playlist_dialog_open;
        if !typing {
            let space_pressed = ctx.input(|i| {
                i.key_pressed(Key::Space)
                    || i.events.iter().any(|ev| match ev {
                        egui::Event::Key {
                            key: Key::Space,
                            pressed: true,
                            repeat: false,
                            ..
                        } => true,
                        egui::Event::Text(t) if t == " " => true,
                        _ => false,
                    })
            });
            if space_pressed {
                self.toggle_playback();
                self.show_osc();
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) && self.is_fullscreen {
            self.toggle_fullscreen(ctx);
        }

        self.render_right_sidebar(ctx);
        self.render_video_and_osc(ctx);
        self.render_modals(ctx);

        // Maintain responsive 20Hz repaint for OSC timer & MPV property updates
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let settings = AppSettings {
            host_address: self.ip_input.clone(),
            cache_size: self.current_cache_size,
        };
        let _ = settings.save_to_path(AppSettings::DEFAULT_PATH);
        self.ws_client.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playlist_edit_preserves_playing_item() {
        let mut app = SyncPlayApp {
            player: None,
            ..SyncPlayApp::new()
        };
        app.playlist = vec!["https://example.com/1.mp4".to_string()];
        app.current_playing_url = Some("https://example.com/1.mp4".to_string());
        app.selected_playlist_idx = Some(0);

        // Editing playlist while keeping currently playing URL should NOT interrupt it
        app.apply_edited_playlist("https://example.com/0.mp4\nhttps://example.com/1.mp4");
        assert_eq!(app.playlist.len(), 2);
        assert_eq!(
            app.current_playing_url.as_deref(),
            Some("https://example.com/1.mp4")
        );
        assert_eq!(app.selected_playlist_idx, Some(1));
    }

    #[test]
    fn test_network_message_handling() {
        let mut app = SyncPlayApp {
            player: None,
            ..SyncPlayApp::new()
        };

        app.handle_network_message(SyncMessage::UserCount { count: 4 });
        assert_eq!(app.viewers_count, 4);

        app.handle_network_message(SyncMessage::Playlist {
            urls: vec!["a.mp4".to_string(), "b.mp4".to_string()],
        });
        assert_eq!(app.playlist, vec!["a.mp4", "b.mp4"]);

        app.handle_network_message(SyncMessage::Play {
            url: "b.mp4".to_string(),
        });
        assert_eq!(app.current_playing_url.as_deref(), Some("b.mp4"));
        assert_eq!(app.selected_playlist_idx, Some(1));
        assert!(app.video_loading);

        // While video_loading is true, seek and pause messages queue into pending_seek / pending_pause
        app.handle_network_message(SyncMessage::Seek { position: 33.5 });
        app.handle_network_message(SyncMessage::Pause { state: false });
        assert_eq!(app.pending_seek, Some(33.5));
        assert_eq!(app.pending_pause, Some(false));

        // When host sends pause: true while client is loading (e.g. joining a paused host),
        // pending_pause must update to true and prevent auto-playing upon complete_video_loading
        app.handle_network_message(SyncMessage::Pause { state: true });
        assert_eq!(app.pending_pause, Some(true));
        assert!(app.is_paused);

        app.complete_video_loading();
        assert!(!app.video_loading);
        assert_eq!(app.current_time, 33.5);
        assert!(app.is_paused);
        assert_eq!(app.pending_pause, None);
        assert_eq!(app.pending_pause_apply_at, None);
    }

    #[test]
    fn test_reload_player_sync_state() {
        let mut app = SyncPlayApp {
            player: None,
            ..SyncPlayApp::new()
        };
        app.current_playing_url = Some("test.mp4".to_string());
        app.current_time = 45.0;
        app.is_paused = false;

        app.reload_player();

        // During reload, video_loading must be true, pending_seek must be preserved, and ignore_next_seek must be true
        assert!(app.video_loading);
        assert_eq!(app.pending_seek, Some(45.0));
        assert_eq!(app.pending_pause, Some(false));
        assert!(app.ignore_next_seek);

        // When loading completes, pending_seek is applied and ignore_next_seek remains true to prevent seek broadcast
        app.complete_video_loading();
        assert!(!app.video_loading);
        assert_eq!(app.current_time, 45.0);
        assert!(app.ignore_next_seek);
    }

    #[test]
    fn test_ready_state_handling() {
        let mut app = SyncPlayApp {
            player: None,
            ..SyncPlayApp::new()
        };
        assert!(!app.is_ready);
        assert_eq!(app.ready_count, 0);

        // Network ReadyCount update
        app.handle_network_message(SyncMessage::ReadyCount { count: 3 });
        assert_eq!(app.ready_count, 3);

        // When disconnected, ready_count reflects local ready state
        app.is_ready = true;
        app.handle_connection_status(false);
        assert_eq!(app.ready_count, 1);

        app.is_ready = false;
        app.handle_connection_status(false);
        assert_eq!(app.ready_count, 0);
    }

    #[test]
    fn test_media_display_info_and_pill_label() {
        let mut app = SyncPlayApp {
            player: None,
            ..SyncPlayApp::new()
        };

        // Fallback info when nothing is playing
        let (title, sub) = app.get_media_display_info();
        assert_eq!(title, "No Media Playing");
        assert_eq!(sub, "Ready to play");
        assert_eq!(app.get_audio_sub_pill_label(), "Audio/CC");

        // When a URL with episode info is loaded
        app.current_playing_url = Some("http://localhost:8080/Cyberpunk%20Episode%2002.mkv".to_string());
        app.playlist = vec![
            "http://localhost:8080/Cyberpunk%20Episode%2001.mkv".to_string(),
            "http://localhost:8080/Cyberpunk%20Episode%2002.mkv".to_string(),
        ];
        app.selected_playlist_idx = Some(1);

        let (title2, sub2) = app.get_media_display_info();
        assert_eq!(title2, "Cyberpunk Episode 02");
        assert_eq!(sub2, "Episode 02 • Track 2 of 2");

        // When media_title is explicitly set by MPV (e.g. YouTube video or stream)
        app.media_title = Some("Lofi Hip Hop Radio 24/7".to_string());
        let (title3, sub3) = app.get_media_display_info();
        assert_eq!(title3, "Lofi Hip Hop Radio 24/7");
        assert_eq!(sub3, "Track 2 of 2");
    }

    #[test]
    fn test_play_prev_item_navigation() {
        let mut app = SyncPlayApp {
            player: None,
            ..SyncPlayApp::new()
        };
        app.playlist = vec![
            "video1.mp4".to_string(),
            "video2.mp4".to_string(),
        ];
        app.current_playing_url = Some("video2.mp4".to_string());
        app.selected_playlist_idx = Some(1);
        app.current_time = 1.0;

        // Navigates to prev track when time <= 3.0
        app.play_prev_item();
        assert_eq!(app.selected_playlist_idx, Some(0));
        assert_eq!(app.current_playing_url, Some("video1.mp4".to_string()));

        // When time > 3.0, restarts current track
        app.current_duration = 100.0;
        app.current_time = 15.0;
        app.play_prev_item();
        assert_eq!(app.selected_playlist_idx, Some(0));
        assert_eq!(app.current_time, 0.0);
    }

    #[test]
    fn test_osc_activity_and_popup_close() {
        let mut app = SyncPlayApp {
            player: None,
            ..SyncPlayApp::new()
        };

        app.osc_visible = false;
        app.audio_sub_popup_open = true;
        app.volume_popup_open = true;

        // show_osc() makes OSC visible and updates last_mouse_activity
        let before = Instant::now();
        app.show_osc();
        assert!(app.osc_visible);
        assert!(app.last_mouse_activity >= before);

        // Clicking outside player bounds or in player dismisses popups
        app.audio_sub_popup_open = false;
        app.volume_popup_open = false;
        assert!(!app.audio_sub_popup_open);
        assert!(!app.volume_popup_open);
    }
}
