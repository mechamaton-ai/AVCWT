use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use egui::{
    Align, Color32, ComboBox, Context, Frame, Id, Key, Layout, Margin, Order, Pos2, Rect,
    RichText, Rounding, Sense, Slider, Stroke, SystemTheme, TextureHandle, TextureOptions, Theme,
    ThemePreference, Vec2, ViewportCommand,
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
    settings_dialog_open: bool,
    edit_playlist_dialog_open: bool,
    edit_playlist_buffer: String,

    // Settings & network state
    ip_input: String,
    current_cache_size: i64,
    viewers_count: usize,
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
            settings_dialog_open: false,
            edit_playlist_dialog_open: false,
            edit_playlist_buffer: String::new(),

            ip_input: saved.host_address,
            current_cache_size: saved.cache_size,
            viewers_count: 0,
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

    pub fn play_next_item(&mut self) {
        let Some(ref current_url) = self.current_playing_url else {
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
                    if self.pending_pause != Some(false) || !state {
                        self.pending_pause = Some(state);
                    }
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
                    self.play_url(url, false);
                }
            }
            SyncMessage::TimeSync { .. } => {}
        }
    }

    fn on_mpv_pause(&mut self, value: bool) {
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
                    if let Some(ref mut player) = self.player {
                        player.set_time_pos(seek_pos);
                    }
                }
            }
            self.pending_pause_apply_at = Some(Instant::now() + Duration::from_millis(300));
        }
    }

    fn apply_pending_pause(&mut self) {
        if let Some(target_pause) = self.pending_pause.take() {
            if self.is_paused != target_pause {
                self.ignore_next_pause = true;
                self.is_paused = target_pause;
                if let Some(ref mut player) = self.player {
                    player.set_pause(target_pause);
                }
            }
        }
    }

    fn on_mpv_duration(&mut self, duration: f64) {
        self.current_duration = duration;
        self.complete_video_loading();
    }

    fn on_mpv_time_pos(&mut self, value: f64) {
        self.current_time = value;
        self.complete_video_loading();

        if self.last_time_pos.is_none() {
            self.last_time_pos = Some(value);
        }

        if let Some(last_pos) = self.last_time_pos {
            if !self.is_live {
                if (value - last_pos).abs() > 1.5 {
                    if self.ignore_next_seek {
                        self.ignore_next_seek = false;
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
                MpvEvent::MouseMoved => self.show_osc(),
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
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new(RichText::new("Edit Playlist").color(Color32::WHITE))
                            .fill(Color32::from_rgb(62, 62, 66))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(90, 90, 96)))
                            .rounding(Rounding::same(6.0)),
                    )
                    .clicked()
                {
                    self.edit_playlist_buffer = self.playlist.join("\n");
                    self.edit_playlist_dialog_open = true;
                }

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
                    [ui.available_width(), 28.0],
                    egui::TextEdit::singleline(&mut self.ip_input)
                        .hint_text("localhost:8765")
                        .text_color(Color32::WHITE),
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

                // Detect mouse movement or click inside video panel to show OSC
                let pointer_pos =
                    ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos()));
                if pointer_pos != self.last_pointer_pos {
                    self.last_pointer_pos = pointer_pos;
                    if let Some(pos) = pointer_pos {
                        if video_rect.contains(pos) {
                            self.show_osc();
                        }
                    }
                }
                if response.clicked() || response.dragged() {
                    self.show_osc();
                }

                // Scroll wheel over the player controls audio volume
                let combo_open = ComboBox::is_open(ctx, Id::new("osc_audio_combo"))
                    || ComboBox::is_open(ctx, Id::new("osc_subs_combo"));
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
                        self.show_osc();
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
                let max_w = 1280.0_f32;
                let max_h = 720.0_f32;
                let scale = (max_w / fit_w).min(max_h / fit_h).min(1.0);
                let target_w = ((fit_w * scale).round() as usize).max(2);
                let target_h = ((fit_h * scale).round() as usize).max(2);

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
                    || ComboBox::is_open(ctx, Id::new("osc_audio_combo"))
                    || ComboBox::is_open(ctx, Id::new("osc_subs_combo"));
                if popup_active {
                    self.last_mouse_activity = Instant::now();
                } else if self.osc_visible
                    && self.last_mouse_activity.elapsed() >= Duration::from_secs(5)
                {
                    self.osc_visible = false;
                }

                if self.osc_visible {
                    self.render_osc_overlays(ctx, video_rect);
                }
            });
    }

    fn render_osc_overlays(&mut self, ctx: &Context, video_rect: Rect) {
        let margin = 20.0;
        let osc_width = (video_rect.width() - margin * 2.0).max(120.0);
        let inner_osc_width = (osc_width - 24.0).max(96.0);
        let osc_bg = Color32::from_rgba_unmultiplied(30, 30, 30, 200);
        let osc_stroke = Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(80, 80, 86, 180),
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
                                ui.label(
                                    RichText::new(format!("Volume: {:.0}%", self.volume))
                                        .color(Color32::WHITE),
                                );
                            });
                        });
                    });
            });

        // Bottom OSC Bar: Playback controls, seekbar, track selectors, volume, sidebar, fullscreen
        let bottom_h = 54.0;
        let bottom_pos = Pos2::new(
            video_rect.left() + margin,
            (video_rect.bottom() - bottom_h - margin).max(video_rect.top() + margin),
        );
        let mut volume_btn_rect = None;

        egui::Area::new(Id::new("bottom_osc_overlay"))
            .order(Order::Middle)
            .fixed_pos(bottom_pos)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(osc_bg)
                    .stroke(osc_stroke)
                    .rounding(Rounding::same(15.0))
                    .inner_margin(Margin::symmetric(12.0, 10.0))
                    .show(ui, |ui| {
                        ui.set_min_width(inner_osc_width);
                        ui.set_max_width(inner_osc_width);
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.horizontal(|ui| {
                            let osc_btn = |label: &str| {
                                egui::Button::new(
                                    RichText::new(label).strong().color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgba_unmultiplied(62, 62, 68, 220))
                                .stroke(Stroke::new(
                                    1.0_f32,
                                    Color32::from_rgba_unmultiplied(105, 105, 112, 180),
                                ))
                                .rounding(Rounding::same(6.0))
                            };

                            // Play / Pause button on the left
                            let play_label = if self.is_paused { "Play" } else { "Pause" };
                            if ui.add_sized([50.0, 26.0], osc_btn(play_label)).clicked() {
                                self.toggle_playback();
                            }

                            // Lay out right-side controls from right to left, then fill the exact remaining space with the seek slider
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                // Fullscreen button
                                if ui.add_sized([26.0, 26.0], osc_btn("⛶")).clicked() {
                                    self.toggle_fullscreen(ctx);
                                }

                                // Toggle Sidebar button
                                if ui.add_sized([26.0, 26.0], osc_btn("☰")).clicked() {
                                    self.toggle_sidebar();
                                }

                                // Volume button
                                let vol_resp = ui.add_sized([26.0, 26.0], osc_btn("🔊"));
                                volume_btn_rect = Some(vol_resp.rect);
                                if vol_resp.clicked() {
                                    self.volume_popup_open = !self.volume_popup_open;
                                }

                                // Subtitle track selector
                                let current_sub_label = self
                                    .sub_tracks
                                    .get(self.selected_sub_idx)
                                    .map(|(_, l)| l.as_str())
                                    .unwrap_or("None")
                                    .to_string();
                                ComboBox::from_id_salt("osc_subs_combo")
                                    .width(64.0)
                                    .selected_text(
                                        RichText::new(current_sub_label).color(Color32::WHITE),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (idx, (track_id, label)) in
                                            self.sub_tracks.iter().enumerate()
                                        {
                                            if ui
                                                .selectable_value(
                                                    &mut self.selected_sub_idx,
                                                    idx,
                                                    label,
                                                )
                                                .changed()
                                            {
                                                if let Some(ref player) = self.player {
                                                    player.set_subtitle_track(*track_id);
                                                }
                                            }
                                        }
                                    });
                                ui.label(RichText::new("Subs:").color(Color32::WHITE));

                                // Audio track selector
                                let current_audio_label = self
                                    .audio_tracks
                                    .get(self.selected_audio_idx)
                                    .map(|(_, l)| l.as_str())
                                    .unwrap_or("None")
                                    .to_string();
                                ComboBox::from_id_salt("osc_audio_combo")
                                    .width(64.0)
                                    .selected_text(
                                        RichText::new(current_audio_label).color(Color32::WHITE),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (idx, (track_id, label)) in
                                            self.audio_tracks.iter().enumerate()
                                        {
                                            if ui
                                                .selectable_value(
                                                    &mut self.selected_audio_idx,
                                                    idx,
                                                    label,
                                                )
                                                .changed()
                                            {
                                                if let Some(ref player) = self.player {
                                                    player.set_audio_track(*track_id);
                                                }
                                            }
                                        }
                                    });
                                ui.label(RichText::new("Audio:").color(Color32::WHITE));

                                // Time label
                                let time_str = format!(
                                    "{} / {}",
                                    format_time(Some(self.current_time)),
                                    format_time(Some(self.current_duration))
                                );
                                ui.label(RichText::new(time_str).color(Color32::WHITE));

                                // Seek slider takes the exact remaining space between Play and Time label
                                let mut slider_val = if self.current_duration > 0.0 {
                                    ((self.current_time / self.current_duration) * 1000.0)
                                        .clamp(0.0, 1000.0)
                                } else {
                                    0.0
                                };

                                let slider_width = (ui.available_width() - 14.0).max(20.0);
                                ui.spacing_mut().slider_width = slider_width;
                                let slider_resp = ui.add_enabled(
                                    self.current_duration > 0.0,
                                    Slider::new(&mut slider_val, 0.0..=1000.0).show_value(false),
                                );
                                if slider_resp.dragged() || slider_resp.changed() {
                                    self.last_mouse_activity = Instant::now();
                                    self.seek_video(slider_val);
                                }
                            });
                        });
                    });
            });

        // Floating vertical Volume Popup above the volume button
        if self.volume_popup_open {
            if let Some(btn_rect) = volume_btn_rect {
                let popup_pos = Pos2::new(btn_rect.center().x - 18.0, btn_rect.top() - 135.0);
                egui::Area::new(Id::new("volume_popup_overlay"))
                    .order(Order::Tooltip)
                    .fixed_pos(popup_pos)
                    .show(ctx, |ui| {
                        Frame::none()
                            .fill(osc_bg)
                            .rounding(Rounding::same(10.0))
                            .inner_margin(Margin::same(8.0))
                            .show(ui, |ui| {
                                let mut vol = self.volume;
                                let resp = ui.add(
                                    Slider::new(&mut vol, 0.0..=100.0)
                                        .vertical()
                                        .show_value(false),
                                );
                                if resp.changed() || resp.dragged() {
                                    self.last_mouse_activity = Instant::now();
                                    self.change_volume(vol);
                                }
                            });
                    });
            }
        }
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
        if !ctx.wants_keyboard_input() {
            if ctx.input(|i| i.key_pressed(Key::Space)) {
                self.toggle_playback();
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
    }
}
