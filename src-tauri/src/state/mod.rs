#[cfg(not(target_os = "windows"))]
use std::process::Child;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HANDLE;

#[derive(Debug, Clone, serde::Serialize)]
pub enum AppStatus {
    Login,
    Capturing,
    FoundValue,
    Launching,
    Running,
    Error,
}

#[derive(Clone, Copy, serde::Serialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub struct StatusPayload {
    pub status: AppStatus,
    pub message: Option<String>,
}

pub struct ProjectorHandle {
    pub process: ProjectorProcess,
    pub hwnd: isize,
    pub original_style: isize,
}

#[cfg(target_os = "windows")]
pub struct ProjectorProcess {
    pub handle: HANDLE,
    pub pid: u32,
}

#[cfg(target_os = "windows")]
unsafe impl Send for ProjectorProcess {}

#[cfg(not(target_os = "windows"))]
pub struct ProjectorProcess {
    pub child: Child,
    pub pid: u32,
}

pub struct AppState {
    pub status: AppStatus,
    pub message: Option<String>,
    pub theme_mode: ThemeMode,
    pub swf_url: Option<String>,
    pub capture_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub projector: Option<ProjectorHandle>,
    pub last_projector_rect: Option<(i32, i32, i32, i32)>,
    pub qq_num: Option<u64>,
    pub wpe_interceptor: Option<Arc<crate::wpe::PacketInterceptor>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            status: AppStatus::Login,
            message: None,
            theme_mode: ThemeMode::Dark,
            swf_url: None,
            capture_stop: None,
            projector: None,
            last_projector_rect: None,
            qq_num: None,
            wpe_interceptor: None,
        }
    }
}

pub fn emit_status(app: &AppHandle, state: &AppState) {
    let payload = StatusPayload {
        status: state.status.clone(),
        message: state.message.clone(),
    };
    let _ = app.emit("status_changed", payload);
}
