use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use libloading::Library;
use log::{info, warn};

const MPV_FORMAT_NONE: c_int = 0;
const MPV_FORMAT_STRING: c_int = 1;
const MPV_FORMAT_FLAG: c_int = 3;
const MPV_FORMAT_INT64: c_int = 4;
const MPV_FORMAT_DOUBLE: c_int = 5;

const MPV_EVENT_NONE: c_int = 0;
const MPV_EVENT_END_FILE: c_int = 7;
const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

const MPV_END_FILE_REASON_EOF: c_int = 0;

const MPV_RENDER_PARAM_INVALID: c_int = 0;
const MPV_RENDER_PARAM_API_TYPE: c_int = 1;
const MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
const MPV_RENDER_PARAM_SW_SIZE: c_int = 17;
const MPV_RENDER_PARAM_SW_FORMAT: c_int = 18;
const MPV_RENDER_PARAM_SW_STRIDE: c_int = 19;
const MPV_RENDER_PARAM_SW_POINTER: c_int = 20;

const MPV_RENDER_UPDATE_FRAME: u64 = 1 << 0;

#[repr(C)]
struct MpvHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct MpvRenderContext {
    _private: [u8; 0],
}

#[repr(C)]
struct MpvEventRaw {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventPropertyRaw {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct MpvEventEndFileRaw {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

#[repr(C)]
struct MpvRenderParam {
    type_: c_int,
    data: *mut c_void,
}

type MpvRenderUpdateFn = unsafe extern "C" fn(cb_ctx: *mut c_void);

struct LibMpvApi {
    _lib: Library,
    mpv_create: unsafe extern "C" fn() -> *mut MpvHandle,
    mpv_initialize: unsafe extern "C" fn(*mut MpvHandle) -> c_int,
    mpv_terminate_destroy: unsafe extern "C" fn(*mut MpvHandle),
    mpv_command: unsafe extern "C" fn(*mut MpvHandle, *const *const c_char) -> c_int,
    mpv_set_option_string:
        unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int,
    mpv_set_property:
        unsafe extern "C" fn(*mut MpvHandle, *const c_char, c_int, *const c_void) -> c_int,
    mpv_set_property_string:
        unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int,
    mpv_get_property:
        unsafe extern "C" fn(*mut MpvHandle, *const c_char, c_int, *mut c_void) -> c_int,
    mpv_observe_property:
        unsafe extern "C" fn(*mut MpvHandle, u64, *const c_char, c_int) -> c_int,
    mpv_wait_event: unsafe extern "C" fn(*mut MpvHandle, c_double) -> *mut MpvEventRaw,
    mpv_free: unsafe extern "C" fn(*mut c_void),
    mpv_render_context_create: unsafe extern "C" fn(
        *mut *mut MpvRenderContext,
        *mut MpvHandle,
        *mut MpvRenderParam,
    ) -> c_int,
    mpv_render_context_set_update_callback:
        unsafe extern "C" fn(*mut MpvRenderContext, MpvRenderUpdateFn, *mut c_void),
    mpv_render_context_update: unsafe extern "C" fn(*mut MpvRenderContext) -> u64,
    mpv_render_context_render:
        unsafe extern "C" fn(*mut MpvRenderContext, *mut MpvRenderParam) -> c_int,
    mpv_render_context_free: unsafe extern "C" fn(*mut MpvRenderContext),
}

impl LibMpvApi {
    fn load() -> Result<Self, String> {
        configure_runtime_environment();
        #[cfg(target_os = "windows")]
        ensure_windows_dependencies();

        let mut candidates: Vec<PathBuf> = Vec::new();
        let lib_names: &[&str] = if cfg!(target_os = "windows") {
            &["mpv-2.dll", "libmpv-2.dll", "mpv-1.dll", "libmpv-1.dll", "mpv.dll"]
        } else if cfg!(target_os = "macos") {
            &["libmpv.2.dylib", "libmpv.1.dylib", "libmpv.dylib"]
        } else {
            &["libmpv.so.2", "libmpv.so", "libmpv.so.1"]
        };

        for dir in candidate_runtime_dirs() {
            for name in lib_names {
                candidates.push(dir.join(name));
            }
        }
        for name in lib_names {
            candidates.push(PathBuf::from(name));
        }

        let mut last_err = String::new();
        let mut loaded_lib = None;
        for candidate in candidates {
            match unsafe { Library::new(&candidate) } {
                Ok(lib) => {
                    info!("Loaded libmpv from {:?}", candidate);
                    loaded_lib = Some(lib);
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                }
            }
        }

        let lib = loaded_lib.ok_or_else(|| {
            format!("Could not load libmpv shared library ({last_err}). Install libmpv-dev on Linux or place mpv-2.dll next to the executable on Windows.")
        })?;

        unsafe {
            let mpv_create = *lib.get(b"mpv_create\0").map_err(|e| e.to_string())?;
            let mpv_initialize = *lib.get(b"mpv_initialize\0").map_err(|e| e.to_string())?;
            let mpv_terminate_destroy = *lib
                .get(b"mpv_terminate_destroy\0")
                .map_err(|e| e.to_string())?;
            let mpv_command = *lib.get(b"mpv_command\0").map_err(|e| e.to_string())?;
            let mpv_set_option_string = *lib
                .get(b"mpv_set_option_string\0")
                .map_err(|e| e.to_string())?;
            let mpv_set_property = *lib.get(b"mpv_set_property\0").map_err(|e| e.to_string())?;
            let mpv_set_property_string = *lib
                .get(b"mpv_set_property_string\0")
                .map_err(|e| e.to_string())?;
            let mpv_get_property = *lib.get(b"mpv_get_property\0").map_err(|e| e.to_string())?;
            let mpv_observe_property = *lib
                .get(b"mpv_observe_property\0")
                .map_err(|e| e.to_string())?;
            let mpv_wait_event = *lib.get(b"mpv_wait_event\0").map_err(|e| e.to_string())?;
            let mpv_free = *lib.get(b"mpv_free\0").map_err(|e| e.to_string())?;
            let mpv_render_context_create = *lib
                .get(b"mpv_render_context_create\0")
                .map_err(|e| e.to_string())?;
            let mpv_render_context_set_update_callback = *lib
                .get(b"mpv_render_context_set_update_callback\0")
                .map_err(|e| e.to_string())?;
            let mpv_render_context_update = *lib
                .get(b"mpv_render_context_update\0")
                .map_err(|e| e.to_string())?;
            let mpv_render_context_render = *lib
                .get(b"mpv_render_context_render\0")
                .map_err(|e| e.to_string())?;
            let mpv_render_context_free = *lib
                .get(b"mpv_render_context_free\0")
                .map_err(|e| e.to_string())?;

            Ok(Self {
                _lib: lib,
                mpv_create,
                mpv_initialize,
                mpv_terminate_destroy,
                mpv_command,
                mpv_set_option_string,
                mpv_set_property,
                mpv_set_property_string,
                mpv_get_property,
                mpv_observe_property,
                mpv_wait_event,
                mpv_free,
                mpv_render_context_create,
                mpv_render_context_set_update_callback,
                mpv_render_context_update,
                mpv_render_context_render,
                mpv_render_context_free,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackType {
    Audio,
    Sub,
    Video,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTrack {
    pub id: i64,
    pub track_type: TrackType,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub selected: bool,
}

impl MediaTrack {
    pub fn display_label(&self) -> String {
        if let Some(ref title) = self.title {
            if !title.is_empty() {
                return title.clone();
            }
        }
        if let Some(ref lang) = self.lang {
            if !lang.is_empty() {
                return lang.clone();
            }
        }
        match self.track_type {
            TrackType::Audio => format!("Audio {}", self.id),
            TrackType::Sub => format!("Sub {}", self.id),
            TrackType::Video => format!("Video {}", self.id),
            TrackType::Other(_) => format!("Track {}", self.id),
        }
    }
}

#[derive(Debug, Clone)]
pub enum MpvEvent {
    TimePosChanged(f64),
    DurationChanged(f64),
    CacheDurationChanged(f64),
    PauseChanged(bool),
    TrackListChanged(Vec<MediaTrack>),
    VideoAspectChanged(f64),
    MouseMoved,
    EofReached,
}

unsafe extern "C" fn on_mpv_render_update(cb_ctx: *mut c_void) {
    if !cb_ctx.is_null() {
        let flag = &*(cb_ctx as *const AtomicBool);
        flag.store(true, Ordering::Release);
    }
}

/// Safe wrapper around `libmpv` client and software render context.
pub struct MpvPlayer {
    api: LibMpvApi,
    handle: *mut MpvHandle,
    render_ctx: *mut MpvRenderContext,
    frame_ready: Arc<AtomicBool>,
    frame_ready_raw: *const AtomicBool,
    pixel_buffer: Vec<u8>,
    buffer_size: (usize, usize),
    pub pause: bool,
    pub time_pos: Option<f64>,
}

impl MpvPlayer {
    pub fn new() -> Result<Self, String> {
        let api = LibMpvApi::load()?;

        unsafe {
            let handle = (api.mpv_create)();
            if handle.is_null() {
                return Err("mpv_create() returned NULL".to_string());
            }

            let set_opt = |k: &str, v: &str| {
                if let (Ok(ck), Ok(cv)) = (CString::new(k), CString::new(v)) {
                    (api.mpv_set_option_string)(handle, ck.as_ptr(), cv.as_ptr());
                }
            };

            // Match all options from client.py plus libmpv render backend
            set_opt("vo", "libmpv");
            set_opt("sw-fast", "yes");
            set_opt("input-default-bindings", "yes");
            set_opt("input-vo-keyboard", "yes");
            set_opt("osc", "no");
            set_opt("cache", "yes");
            set_opt("ytdl", "yes");
            let ytdl_opt = format!("ytdl_hook-ytdl_path={}", resolve_ytdlp_executable());
            set_opt("script-opts", &ytdl_opt);
            set_opt("hwdec", "auto");
            set_opt("ytdl-format", "bestvideo+bestaudio/best");

            let init_rc = (api.mpv_initialize)(handle);
            if init_rc < 0 {
                (api.mpv_terminate_destroy)(handle);
                return Err(format!("mpv_initialize failed with code {init_rc}"));
            }

            // Create software render context so video frames can be drawn directly into egui textures
            let sw_api = CString::new("sw").unwrap();
            let mut params = [
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_API_TYPE,
                    data: sw_api.as_ptr() as *mut c_void,
                },
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_INVALID,
                    data: std::ptr::null_mut(),
                },
            ];

            let mut render_ctx: *mut MpvRenderContext = std::ptr::null_mut();
            let rc = (api.mpv_render_context_create)(
                &mut render_ctx,
                handle,
                params.as_mut_ptr(),
            );
            if rc < 0 || render_ctx.is_null() {
                warn!("mpv_render_context_create (sw) returned {rc}; continuing without embedded render context");
                render_ctx = std::ptr::null_mut();
            }

            let frame_ready = Arc::new(AtomicBool::new(false));
            let frame_ready_raw = Arc::as_ptr(&frame_ready);
            if !render_ctx.is_null() {
                (api.mpv_render_context_set_update_callback)(
                    render_ctx,
                    on_mpv_render_update,
                    frame_ready_raw as *mut c_void,
                );
            }

            // Observe the same properties as client.py
            let observe = |userdata: u64, prop: &str, fmt: c_int| {
                if let Ok(cprop) = CString::new(prop) {
                    (api.mpv_observe_property)(handle, userdata, cprop.as_ptr(), fmt);
                }
            };
            observe(1, "pause", MPV_FORMAT_FLAG);
            observe(2, "time-pos", MPV_FORMAT_DOUBLE);
            observe(3, "duration", MPV_FORMAT_DOUBLE);
            observe(4, "demuxer-cache-duration", MPV_FORMAT_DOUBLE);
            observe(5, "track-list/count", MPV_FORMAT_INT64);
            observe(6, "mouse-pos", MPV_FORMAT_NONE);
            observe(7, "video-out-params/aspect", MPV_FORMAT_DOUBLE);
            observe(8, "video-params/aspect", MPV_FORMAT_DOUBLE);
            observe(9, "dwidth", MPV_FORMAT_INT64);
            observe(10, "dheight", MPV_FORMAT_INT64);

            Ok(Self {
                api,
                handle,
                render_ctx,
                frame_ready,
                frame_ready_raw,
                pixel_buffer: Vec::new(),
                buffer_size: (0, 0),
                pause: true,
                time_pos: None,
            })
        }
    }

    /// Executes an mpv command array (e.g. `&["stop"]` or `&["loadfile", url, "replace"]`).
    pub fn command(&self, args: &[&str]) {
        let c_args: Vec<CString> = args
            .iter()
            .filter_map(|s| CString::new(*s).ok())
            .collect();
        let mut ptrs: Vec<*const c_char> = c_args.iter().map(|cs| cs.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        unsafe {
            (self.api.mpv_command)(self.handle, ptrs.as_ptr());
        }
    }

    pub fn play(&self, url: &str) {
        self.command(&["loadfile", url, "replace"]);
    }

    pub fn stop(&self) {
        self.command(&["stop"]);
    }

    pub fn set_pause(&mut self, paused: bool) {
        self.pause = paused;
        let flag: c_int = if paused { 1 } else { 0 };
        if let Ok(name) = CString::new("pause") {
            unsafe {
                (self.api.mpv_set_property)(
                    self.handle,
                    name.as_ptr(),
                    MPV_FORMAT_FLAG,
                    &flag as *const c_int as *const c_void,
                );
            }
        }
    }

    pub fn set_time_pos(&mut self, pos: f64) {
        self.time_pos = Some(pos);
        if let Ok(name) = CString::new("time-pos") {
            unsafe {
                (self.api.mpv_set_property)(
                    self.handle,
                    name.as_ptr(),
                    MPV_FORMAT_DOUBLE,
                    &pos as *const f64 as *const c_void,
                );
            }
        }
    }

    pub fn set_volume(&self, volume: f64) {
        if let Ok(name) = CString::new("volume") {
            unsafe {
                (self.api.mpv_set_property)(
                    self.handle,
                    name.as_ptr(),
                    MPV_FORMAT_DOUBLE,
                    &volume as *const f64 as *const c_void,
                );
            }
        }
    }

    pub fn set_audio_track(&self, track_id: Option<i64>) {
        let val = match track_id {
            Some(id) => id.to_string(),
            None => "no".to_string(),
        };
        self.set_property_str("aid", &val);
    }

    pub fn set_subtitle_track(&self, track_id: Option<i64>) {
        let val = match track_id {
            Some(id) => id.to_string(),
            None => "no".to_string(),
        };
        self.set_property_str("sid", &val);
    }

    pub fn update_cache_size(&self, value_mb: i64) {
        let bytes_val = value_mb.saturating_mul(1024 * 1024);
        self.set_property_str("cache", "yes");
        self.set_property_i64("demuxer-max-bytes", bytes_val);
        self.set_property_i64("demuxer-max-back-bytes", bytes_val);
    }

    pub fn set_property_str(&self, prop: &str, value: &str) {
        if let (Ok(cprop), Ok(cval)) = (CString::new(prop), CString::new(value)) {
            unsafe {
                (self.api.mpv_set_property_string)(self.handle, cprop.as_ptr(), cval.as_ptr());
            }
        }
    }

    pub fn set_property_i64(&self, prop: &str, value: i64) {
        if let Ok(cprop) = CString::new(prop) {
            unsafe {
                (self.api.mpv_set_property)(
                    self.handle,
                    cprop.as_ptr(),
                    MPV_FORMAT_INT64,
                    &value as *const i64 as *const c_void,
                );
            }
        }
    }

    fn get_property_i64(&self, prop: &str) -> Option<i64> {
        let cprop = CString::new(prop).ok()?;
        let mut val: i64 = 0;
        let rc = unsafe {
            (self.api.mpv_get_property)(
                self.handle,
                cprop.as_ptr(),
                MPV_FORMAT_INT64,
                &mut val as *mut i64 as *mut c_void,
            )
        };
        if rc >= 0 {
            Some(val)
        } else {
            None
        }
    }

    fn get_property_double(&self, prop: &str) -> Option<f64> {
        let cprop = CString::new(prop).ok()?;
        let mut val: f64 = 0.0;
        let rc = unsafe {
            (self.api.mpv_get_property)(
                self.handle,
                cprop.as_ptr(),
                MPV_FORMAT_DOUBLE,
                &mut val as *mut f64 as *mut c_void,
            )
        };
        if rc >= 0 && val.is_finite() {
            Some(val)
        } else {
            None
        }
    }

    fn read_video_aspect(&self) -> Option<f64> {
        if let Some(aspect) = self.get_property_double("video-out-params/aspect") {
            if aspect > 0.01 {
                return Some(aspect);
            }
        }
        if let Some(aspect) = self.get_property_double("video-params/aspect") {
            if aspect > 0.01 {
                return Some(aspect);
            }
        }
        if let (Some(dw), Some(dh)) = (
            self.get_property_i64("dwidth"),
            self.get_property_i64("dheight"),
        ) {
            if dw > 0 && dh > 0 {
                return Some(dw as f64 / dh as f64);
            }
        }
        None
    }

    fn get_property_bool(&self, prop: &str) -> Option<bool> {
        let cprop = CString::new(prop).ok()?;
        let mut val: c_int = 0;
        let rc = unsafe {
            (self.api.mpv_get_property)(
                self.handle,
                cprop.as_ptr(),
                MPV_FORMAT_FLAG,
                &mut val as *mut c_int as *mut c_void,
            )
        };
        if rc >= 0 {
            Some(val != 0)
        } else {
            None
        }
    }

    fn get_property_string(&self, prop: &str) -> Option<String> {
        let cprop = CString::new(prop).ok()?;
        let mut ptr: *mut c_char = std::ptr::null_mut();
        let rc = unsafe {
            (self.api.mpv_get_property)(
                self.handle,
                cprop.as_ptr(),
                MPV_FORMAT_STRING,
                &mut ptr as *mut *mut c_char as *mut c_void,
            )
        };
        if rc >= 0 && !ptr.is_null() {
            let s = unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() };
            unsafe {
                (self.api.mpv_free)(ptr as *mut c_void);
            }
            Some(s)
        } else {
            None
        }
    }

    fn read_track_list(&self) -> Vec<MediaTrack> {
        let count = self.get_property_i64("track-list/count").unwrap_or(0);
        let mut tracks = Vec::new();
        for i in 0..count {
            let id = self
                .get_property_i64(&format!("track-list/{i}/id"))
                .unwrap_or(i + 1);
            let raw_type = self
                .get_property_string(&format!("track-list/{i}/type"))
                .unwrap_or_default();
            let track_type = match raw_type.as_str() {
                "audio" => TrackType::Audio,
                "sub" => TrackType::Sub,
                "video" => TrackType::Video,
                other => TrackType::Other(other.to_string()),
            };
            let title = self.get_property_string(&format!("track-list/{i}/title"));
            let lang = self.get_property_string(&format!("track-list/{i}/lang"));
            let selected = self
                .get_property_bool(&format!("track-list/{i}/selected"))
                .unwrap_or(false);
            tracks.push(MediaTrack {
                id,
                track_type,
                title,
                lang,
                selected,
            });
        }
        tracks
    }

    /// Drains all queued `mpv` events without blocking.
    pub fn poll_events(&mut self) -> Vec<MpvEvent> {
        let mut out = Vec::new();
        loop {
            let ev_ptr = unsafe { (self.api.mpv_wait_event)(self.handle, 0.0) };
            if ev_ptr.is_null() {
                break;
            }
            let ev = unsafe { &*ev_ptr };
            if ev.event_id == MPV_EVENT_NONE {
                break;
            }
            match ev.event_id {
                MPV_EVENT_PROPERTY_CHANGE if !ev.data.is_null() => {
                    let prop = unsafe { &*(ev.data as *const MpvEventPropertyRaw) };
                    if !prop.name.is_null() {
                        let name = unsafe { CStr::from_ptr(prop.name).to_string_lossy() };
                        match name.as_ref() {
                            "pause" if prop.format == MPV_FORMAT_FLAG && !prop.data.is_null() => {
                                let flag = unsafe { *(prop.data as *const c_int) != 0 };
                                self.pause = flag;
                                out.push(MpvEvent::PauseChanged(flag));
                            }
                            "time-pos"
                                if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() =>
                            {
                                let val = unsafe { *(prop.data as *const f64) };
                                self.time_pos = Some(val);
                                out.push(MpvEvent::TimePosChanged(val));
                            }
                            "duration"
                                if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() =>
                            {
                                let val = unsafe { *(prop.data as *const f64) };
                                out.push(MpvEvent::DurationChanged(val));
                            }
                            "demuxer-cache-duration"
                                if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() =>
                            {
                                let val = unsafe { *(prop.data as *const f64) };
                                out.push(MpvEvent::CacheDurationChanged(val));
                            }
                            "track-list/count" => {
                                let tracks = self.read_track_list();
                                out.push(MpvEvent::TrackListChanged(tracks));
                            }
                            "video-out-params/aspect" | "video-params/aspect"
                                if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() =>
                            {
                                let val = unsafe { *(prop.data as *const f64) };
                                if val > 0.01 && val.is_finite() {
                                    out.push(MpvEvent::VideoAspectChanged(val));
                                }
                            }
                            "dwidth" | "dheight" => {
                                if let Some(aspect) = self.read_video_aspect() {
                                    out.push(MpvEvent::VideoAspectChanged(aspect));
                                }
                            }
                            "mouse-pos" => {
                                out.push(MpvEvent::MouseMoved);
                            }
                            _ => {}
                        }
                    }
                }
                MPV_EVENT_END_FILE if !ev.data.is_null() => {
                    let end_file = unsafe { &*(ev.data as *const MpvEventEndFileRaw) };
                    if end_file.reason == MPV_END_FILE_REASON_EOF {
                        out.push(MpvEvent::EofReached);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// If a new video frame is ready, renders it into an RGBA buffer of size `(width, height)`.
    pub fn render_frame_if_updated(&mut self, width: usize, height: usize) -> Option<&[u8]> {
        if self.render_ctx.is_null() || width == 0 || height == 0 {
            return None;
        }

        let size_changed = self.buffer_size != (width, height);
        let signalled = self.frame_ready.swap(false, Ordering::AcqRel);
        if !signalled && !size_changed {
            return None;
        }

        unsafe {
            let flags = (self.api.mpv_render_context_update)(self.render_ctx);
            if (flags & MPV_RENDER_UPDATE_FRAME) == 0 && !size_changed {
                return None;
            }

            let stride = width * 4;
            let needed = stride * height;
            if self.pixel_buffer.len() != needed {
                self.pixel_buffer.resize(needed, 0);
            }
            self.buffer_size = (width, height);

            let mut size: [c_int; 2] = [width as c_int, height as c_int];
            let fmt = CString::new("rgb0").unwrap();
            let mut stride_c: usize = stride;
            let mut block_for_target: c_int = 0;

            let mut params = [
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_SW_SIZE,
                    data: size.as_mut_ptr() as *mut c_void,
                },
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_SW_FORMAT,
                    data: fmt.as_ptr() as *mut c_void,
                },
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_SW_STRIDE,
                    data: &mut stride_c as *mut usize as *mut c_void,
                },
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_SW_POINTER,
                    data: self.pixel_buffer.as_mut_ptr() as *mut c_void,
                },
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
                    data: &mut block_for_target as *mut c_int as *mut c_void,
                },
                MpvRenderParam {
                    type_: MPV_RENDER_PARAM_INVALID,
                    data: std::ptr::null_mut(),
                },
            ];

            let rc = (self.api.mpv_render_context_render)(self.render_ctx, params.as_mut_ptr());
            if rc < 0 {
                return None;
            }

            // Ensure alpha channel is opaque (255) for egui::ColorImage::from_rgba_unmultiplied
            for chunk in self.pixel_buffer.chunks_exact_mut(4) {
                chunk[3] = 255;
            }

            Some(&self.pixel_buffer)
        }
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        unsafe {
            if !self.render_ctx.is_null() {
                (self.api.mpv_render_context_free)(self.render_ctx);
                self.render_ctx = std::ptr::null_mut();
            }
            if !self.handle.is_null() {
                (self.api.mpv_terminate_destroy)(self.handle);
                self.handle = std::ptr::null_mut();
            }
            let _ = self.frame_ready_raw;
        }
    }
}

/// Returns candidate directories to search for bundled binaries/DLLs (`mpv-2.dll`, `yt-dlp.exe`):
/// the executable directory, its parent/grandparent (for `target/release` or `target/debug`),
/// and the current working directory.
pub fn candidate_runtime_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            dirs.push(exe_dir.to_path_buf());
            if let Some(parent) = exe_dir.parent() {
                dirs.push(parent.to_path_buf());
                if let Some(grandparent) = parent.parent() {
                    dirs.push(grandparent.to_path_buf());
                }
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if !dirs.iter().any(|d| d == &cwd) {
            dirs.push(cwd);
        }
    }
    dirs
}

/// Configures `LC_NUMERIC=C` and adds executable and user binary directories to `PATH`.
pub fn configure_runtime_environment() {
    extern "C" {
        fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
    }
    #[cfg(target_os = "linux")]
    const LC_NUMERIC: c_int = 1;
    #[cfg(not(target_os = "linux"))]
    const LC_NUMERIC: c_int = 4;

    if let Ok(c_locale) = CString::new("C") {
        unsafe {
            setlocale(LC_NUMERIC, c_locale.as_ptr());
        }
    }

    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();

    for dir in candidate_runtime_dirs() {
        if !paths.iter().any(|p| p == &dir) {
            paths.insert(0, dir);
        }
    }

    if cfg!(target_os = "linux") {
        if let Ok(home) = std::env::var("HOME") {
            let local_bin = PathBuf::from(home).join(".local").join("bin");
            if !paths.iter().any(|p| p == &local_bin) {
                paths.push(local_bin);
            }
        }
    }

    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var("PATH", joined);
    }
}

/// On Windows, when `AVCWT.exe` is launched by double-clicking, automatically downloads
/// `yt-dlp.exe` and `mpv-2.dll` next to the executable if they are not already present.
#[cfg(target_os = "windows")]
fn ensure_windows_dependencies() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let target_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let has_mpv_dll = candidate_runtime_dirs().iter().any(|dir| {
        ["mpv-2.dll", "libmpv-2.dll", "mpv-1.dll", "libmpv-1.dll", "mpv.dll"]
            .iter()
            .any(|name| dir.join(name).exists())
    });

    let has_ytdlp = candidate_runtime_dirs()
        .iter()
        .any(|dir| dir.join("yt-dlp.exe").exists());

    if has_mpv_dll && has_ytdlp {
        return;
    }

    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
if (-not (Test-Path 'yt-dlp.exe')) {
    Invoke-WebRequest -Uri 'https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe' -OutFile 'yt-dlp.exe'
}
if (-not (Test-Path 'mpv-2.dll') -and -not (Test-Path 'libmpv-2.dll')) {
    $releases = Invoke-RestMethod -Uri 'https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases?per_page=10'
    $asset = $null
    foreach ($rel in $releases) {
        $asset = $rel.assets | Where-Object { $_.name -match '^mpv-dev-x86_64-20.*\.7z$' -and $_.name -notmatch 'v3' } | Select-Object -First 1
        if ($asset) { break }
    }
    if ($asset) {
        Invoke-WebRequest -Uri $asset.browser_download_url -OutFile 'mpv-dev.7z'
        if (Get-Command 7z -ErrorAction SilentlyContinue) {
            7z e mpv-dev.7z '*mpv*.dll' -y | Out-Null
        } elseif (Test-Path "$env:ProgramFiles\7-Zip\7z.exe") {
            & "$env:ProgramFiles\7-Zip\7z.exe" e mpv-dev.7z '*mpv*.dll' -y | Out-Null
        } else {
            tar -xf mpv-dev.7z '*mpv*.dll'
        }
        if ((Test-Path 'libmpv-2.dll') -and -not (Test-Path 'mpv-2.dll')) {
            Move-Item -Force 'libmpv-2.dll' 'mpv-2.dll'
        }
        Remove-Item -Force 'mpv-dev.7z' -ErrorAction SilentlyContinue
    }
}
"#;

    let _ = Command::new("powershell")
        .current_dir(&target_dir)
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

/// Resolves the path to `yt-dlp` (or `yt-dlp.exe` on Windows) from candidate runtime directories,
/// falling back to `"yt-dlp"` on the system `PATH`.
pub fn resolve_ytdlp_executable() -> String {
    let bin_name = if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp"
    };
    for dir in candidate_runtime_dirs() {
        let candidate = dir.join(bin_name);
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    "yt-dlp".to_string()
}

/// Asynchronously checks whether a URL is a livestream using `yt-dlp` (matching `_check_is_live` in `client.py`).
pub fn check_is_live_async(url: String, tx: Sender<(String, bool)>) {
    std::thread::spawn(move || {
        let is_live = if url.starts_with("http://") || url.starts_with("https://") {
            let mut cmd = Command::new(resolve_ytdlp_executable());
            cmd.args([
                "--dump-single-json",
                "--flat-playlist",
                "--quiet",
                "--no-warnings",
                &url,
            ]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            cmd.output()
                .ok()
                .and_then(|out| {
                    if out.status.success() {
                        serde_json::from_slice::<serde_json::Value>(&out.stdout).ok()
                    } else {
                        None
                    }
                })
                .and_then(|val| val.get("is_live")?.as_bool())
                .unwrap_or(false)
        } else {
            false
        };
        let _ = tx.send((url, is_live));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpv_player_initialization_and_properties() {
        let mut player = MpvPlayer::new().expect("libmpv should initialize cleanly");
        player.update_cache_size(250);
        player.set_volume(80.0);
        player.set_pause(true);
        player.set_audio_track(None);
        player.set_subtitle_track(None);
        let _events = player.poll_events();
    }
}

