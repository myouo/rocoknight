#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod debug;
mod debug_console_layer;
mod debug_log_bus;
mod embed_win32;
mod error_handling;
mod launcher;
mod login3_capture;
mod projector;
mod request_context;
mod state;
mod wpe;

use std::io::Write;
use std::sync::{Mutex, OnceLock};

use log::LevelFilter;
use tauri::path::BaseDirectory;
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Size, State};
use tauri_utils::config::WebviewUrl;
use tracing::{error, info};

use crate::embed_win32::{disable_maximize_resize, parent_client_size, set_dpi_awareness};
use crate::launcher::{
    resize_login_to_window, resize_projector_to_window, schedule_login_layout,
    stop_projector as stop_projector_state,
};
use crate::state::{emit_status, AppState, AppStatus, ThemeMode};

// 全局退出标志（所有模块可见）
pub static EXITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 请求退出（必定在 100ms 内退出进程）
fn request_exit() {
    // 设置全局退出标志
    if EXITING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        // 已经在退出中，直接返回
        startup_log("request_exit: already exiting");
        return;
    }

    startup_log("request_exit: EXITING set to true");

    // 立即启动兜底线程（100ms 后强制退出）
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(100));
        startup_log("request_exit: fallback triggered -> process::exit(0)");
        std::process::exit(0);
    });

    startup_log("request_exit: fallback thread spawned (will exit in 100ms)");
}


static LAST_WINDOW_SIZE: OnceLock<Mutex<Option<PhysicalSize<u32>>>> = OnceLock::new();
const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const LOG_TRIM_BYTES: usize = 1 * 1024 * 1024;
const UI_BAR_HEIGHT: u32 = 36;

#[derive(serde::Deserialize)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn with_state<R>(state: &State<Mutex<AppState>>, f: impl FnOnce(&mut AppState) -> R) -> R {
    let mut guard = state.lock().expect("state lock");
    f(&mut guard)
}

fn parse_theme_mode(theme: &str) -> Option<ThemeMode> {
    match theme.trim().to_ascii_lowercase().as_str() {
        "dark" => Some(ThemeMode::Dark),
        "light" => Some(ThemeMode::Light),
        _ => None,
    }
}

fn apply_theme_to_app(app: &AppHandle, mode: ThemeMode) {
    let class_script = match mode {
        ThemeMode::Dark => "document.body.classList.remove('light');",
        ThemeMode::Light => "document.body.classList.add('light');",
    };

    for label in ["toolbar", "main"] {
        if let Some(webview) = app.get_webview(label) {
            let _ = webview.eval(class_script);
        }
    }
}

static STARTUP_LOG: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> =
    std::sync::OnceLock::new();

fn trim_log_file(path: &std::path::Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() <= LOG_MAX_BYTES {
        return;
    }
    let Ok(data) = std::fs::read(path) else {
        return;
    };
    let keep_from = LOG_TRIM_BYTES.min(data.len());
    let _ = std::fs::write(path, &data[keep_from..]);
}

fn init_startup_log() {
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let path = std::path::PathBuf::from(local)
                .join("RocoKnight")
                .join("logs")
                .join("rocoknight.log");
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            trim_log_file(&path);
            if let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = STARTUP_LOG.set(std::sync::Mutex::new(file));
                startup_log(&format!("startup log initialized: {}", path.display()));
            }
        }
    }
}

fn startup_log(message: &str) {
    if let Some(lock) = STARTUP_LOG.get() {
        if let Ok(mut file) = lock.lock() {
            let _ = writeln!(file, "[{:?}] {}", std::time::SystemTime::now(), message);
        }
    }
}

fn show_boot_message(step: &str) {
    startup_log(step);
}

fn show_error_message(step: &str) {
    startup_log(step);
}

fn track_last_size(size: PhysicalSize<u32>) {
    if size.width == 0 || size.height == 0 {
        return;
    }
    let lock = LAST_WINDOW_SIZE.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().expect("window size lock");
    *guard = Some(size);
}

fn compute_window_size(screen: PhysicalSize<u32>, scale_factor: f64) -> PhysicalSize<u32> {
    let area = (screen.width as f64) * (screen.height as f64) * 0.4;
    let ratio = 12.0 / 7.0;
    let mut w = (area * ratio).sqrt();
    let mut h = w / ratio;

    if w > screen.width as f64 {
        w = screen.width as f64;
        h = w / ratio;
    }
    if h > screen.height as f64 {
        h = screen.height as f64;
        w = h * ratio;
    }

    let w = w.round().max(640.0) as u32;
    let h = h.round().max(360.0) as u32;
    let bar_h = ((UI_BAR_HEIGHT as f64) * scale_factor).round().max(1.0) as u32;
    PhysicalSize::new(w, h + bar_h)
}

fn center_window(window: &tauri::Window, size: PhysicalSize<u32>) {
    let monitor = window.current_monitor().ok().flatten();
    let (screen_pos, screen_size) = if let Some(mon) = monitor {
        (*mon.position(), *mon.size())
    } else {
        (PhysicalPosition::new(0, 0), PhysicalSize::new(1920, 1080))
    };
    let x = screen_pos.x + ((screen_size.width as i32 - size.width as i32) / 2);
    let y = screen_pos.y + ((screen_size.height as i32 - size.height as i32) / 2);
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn align_window_height_for_game_ratio(window: &tauri::Window) {
    let Ok(outer) = window.inner_size() else {
        return;
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let Some((client_w, client_h)) = parent_client_size(hwnd) else {
        return;
    };

    let scale = window.scale_factor().unwrap_or(1.0);
    let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round().max(1.0) as i32;
    let desired_game_h = ((client_w as f64) * 7.0 / 12.0).round() as i32;
    let desired_client_h = (desired_game_h + bar_h).max(1);
    let delta = desired_client_h - client_h;
    if delta == 0 {
        return;
    }

    let new_outer_h = (outer.height as i32 + delta).max(240) as u32;
    let new_outer = PhysicalSize::new(outer.width, new_outer_h);
    let _ = window.set_size(Size::Physical(new_outer));
    let _ = window.set_min_size(Some(Size::Physical(new_outer)));
    let _ = window.set_max_size(Some(Size::Physical(new_outer)));
}

#[tauri::command]
fn set_login_bounds(app: AppHandle, rect: Rect) -> Result<(), String> {
    let webview = app
        .get_webview("login")
        .ok_or_else(|| "Login WebView not found.".to_string())?;
    webview
        .set_position(LogicalPosition::new(rect.x, rect.y))
        .map_err(|_| "Failed to update login webview position.".to_string())?;
    webview
        .set_size(LogicalSize::new(rect.w, rect.h))
        .map_err(|_| "Failed to update login webview size.".to_string())?;
    Ok(())
}

#[tauri::command]
fn show_login_webview(app: AppHandle) -> Result<(), String> {
    let webview = app
        .get_webview("login")
        .ok_or_else(|| "Login WebView not found.".to_string())?;
    webview
        .show()
        .map_err(|_| "Failed to show login view.".to_string())?;
    Ok(())
}

#[tauri::command]
fn hide_login_webview(app: AppHandle) -> Result<(), String> {
    let webview = app
        .get_webview("login")
        .ok_or_else(|| "Login WebView not found.".to_string())?;
    webview
        .hide()
        .map_err(|_| "Failed to hide login view.".to_string())?;
    Ok(())
}

#[tauri::command]
fn get_theme_mode(state: State<Mutex<AppState>>) -> String {
    with_state(&state, |s| s.theme_mode.as_str().to_string())
}

#[tauri::command]
fn set_theme_mode(
    app: AppHandle,
    state: State<Mutex<AppState>>,
    theme: String,
) -> Result<String, String> {
    let mode = parse_theme_mode(&theme)
        .ok_or_else(|| "Invalid theme. Use 'dark' or 'light'.".to_string())?;
    with_state(&state, |s| {
        s.theme_mode = mode;
    });
    apply_theme_to_app(&app, mode);
    Ok(mode.as_str().to_string())
}

#[tauri::command]
fn start_login3_capture(app: AppHandle, state: State<Mutex<AppState>>) -> Result<(), String> {
    let _timer = request_context::CommandTimer::new("start_login3_capture", 500);

    tracing::info!("command invoked");
    startup_log("start_login3_capture");

    match login3_capture::start(app, state) {
        Ok(()) => {
            tracing::info!("capture started successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!(error = %e, "capture start failed");
            Err(e)
        }
    }
}

#[tauri::command]
fn stop_login3_capture(app: AppHandle, state: State<Mutex<AppState>>) {
    let _timer = request_context::CommandTimer::new("stop_login3_capture", 200);
    tracing::info!("command invoked");
    login3_capture::stop(app, state);
    tracing::info!("capture stopped");
}

#[tauri::command]
fn launch_projector(
    app: AppHandle,
    state: State<Mutex<AppState>>,
    rect: Rect,
) -> Result<(), String> {
    let _timer = request_context::CommandTimer::new("launch_projector", 2000);

    let swf_url = with_state(&state, |s| s.swf_url.clone());
    tracing::info!(
        has_swf_url = swf_url.is_some(),
        rect_w = rect.w,
        rect_h = rect.h,
        "command invoked"
    );

    match crate::launcher::launch_projector_auto(&app, &state) {
        Ok(()) => {
            tracing::info!("projector launched successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!(error = %e, "projector launch failed");
            Err(e)
        }
    }
}

#[tauri::command]
fn resize_projector(app: AppHandle, state: State<Mutex<AppState>>, rect: Rect) {
    let _ = rect;
    resize_projector_to_window(&app, &state);
}

fn stop_projector_command(state: &State<Mutex<AppState>>) {
    stop_projector_state(state);
}

#[tauri::command]
fn stop_projector(app: AppHandle, state: State<Mutex<AppState>>) {
    let _timer = request_context::CommandTimer::new("stop_projector", 500);
    tracing::info!("command invoked");
    stop_projector_command(&state);
    emit_status(&app, &state.lock().expect("state lock"));
    tracing::info!("projector stopped and status emitted");
}

#[tauri::command]
fn restart_projector(
    app: AppHandle,
    state: State<Mutex<AppState>>,
    rect: Rect,
) -> Result<(), String> {
    let _timer = request_context::CommandTimer::new("restart_projector", 2000);
    tracing::info!(rect_w = rect.w, rect_h = rect.h, "command invoked");

    stop_projector_command(&state);
    tracing::info!("projector stopped");

    match launch_projector(app, state, rect) {
        Ok(()) => {
            tracing::info!("projector restarted successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!(error = %e, "projector restart failed");
            Err(e)
        }
    }
}

#[tauri::command]
fn change_channel(app: AppHandle, state: State<Mutex<AppState>>) -> Result<(), String> {
    let _timer = request_context::CommandTimer::new("change_channel", 2000);

    // 阶段 1：验证状态
    let (has_projector, has_swf) = {
        let _stage = request_context::StageTimer::new("validate_state");
        let result = with_state(&state, |s| (s.projector.is_some(), s.swf_url.is_some()));
        tracing::info!(
            has_projector = result.0,
            has_swf = result.1,
            "state validated"
        );
        result
    };

    if !has_projector {
        tracing::warn!("projector not running, fallback to relogin");
        return reset_to_login(app, state);
    }

    if !has_swf {
        tracing::error!("missing swf url");
        return Err("Missing swf url.".to_string());
    }

    // 阶段 2：重启投影器
    {
        let _stage = request_context::StageTimer::new("relaunch_projector");
        match crate::launcher::launch_projector_auto(&app, &state) {
            Ok(()) => {
                tracing::info!("projector relaunched successfully");
            }
            Err(e) => {
                tracing::error!(error = %e, "projector relaunch failed");
                return Err(e);
            }
        }
    }

    tracing::info!("channel changed successfully");
    Ok(())
}

#[tauri::command]
fn reset_to_login(app: AppHandle, state: State<Mutex<AppState>>) -> Result<(), String> {
    let _timer = request_context::CommandTimer::new("reset_to_login", 1000);

    let current_status = with_state(&state, |s| s.status.clone());
    tracing::info!(current_status = ?current_status, "command invoked");

    // 阶段 1：停止投影器
    {
        let _stage = request_context::StageTimer::new("stop_projector");
        stop_projector_command(&state);
        login3_capture::stop_timer_only(&state);
        tracing::info!("projector and capture stopped");
    }

    // 阶段 2：重置状态
    {
        let _stage = request_context::StageTimer::new("reset_state");
        with_state(&state, |s| {
            tracing::info!(
              old_status = ?s.status,
              new_status = ?AppStatus::Login,
              "state transition"
            );
            s.status = AppStatus::Login;
            s.message = None;
            s.swf_url = None;
        });
        tracing::info!("state reset complete");
    }

    // 阶段 3：显示登录窗口
    {
        let _stage = request_context::StageTimer::new("show_login");

        if let Some(main) = app.get_webview("main") {
            let _ = main.show();
        }

        let login = app.get_webview("login").ok_or_else(|| {
            tracing::error!("login webview not found");
            "Login WebView not found.".to_string()
        })?;

        login.show().map_err(|e| {
            tracing::error!(error = ?e, "failed to show login webview");
            "Failed to show login webview.".to_string()
        })?;

        tracing::info!("login webview shown");
    }

    // 阶段 4：导航到登录页
    {
        let _stage = request_context::StageTimer::new("navigate");

        let login = app.get_webview("login").unwrap();
        let url = "https://17roco.qq.com/login.html".parse().map_err(|e| {
            tracing::error!(error = ?e, "invalid login URL");
            "Invalid login URL.".to_string()
        })?;

        login.navigate(url).map_err(|e| {
            tracing::error!(error = ?e, "failed to navigate login webview");
            "Failed to navigate login webview.".to_string()
        })?;

        tracing::info!(
            url = "https://17roco.qq.com/login.html",
            "navigation complete"
        );
    }

    // 阶段 5：调整布局
    {
        let _stage = request_context::StageTimer::new("adjust_layout");
        resize_login_to_window(&app);
        schedule_login_layout(app.clone());
        tracing::info!("layout adjusted");
    }

    // 阶段 6：发送状态
    {
        let _stage = request_context::StageTimer::new("emit_status");
        emit_status(&app, &state.lock().expect("state lock"));
        tracing::info!("status emitted");
    }

    tracing::info!("reset to login completed successfully");
    Ok(())
}

#[tauri::command]
fn toggle_debug_window(app: AppHandle) -> Result<bool, String> {
    // 全局退出标志（用于在退出时拒绝所有 debug 命令）
    static EXITING_GLOBAL: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    // 如果正在退出，拒绝所有 debug 命令
    if EXITING.load(std::sync::atomic::Ordering::SeqCst) {
        startup_log("TOGGLE: REJECTED due to EXITING=true");
        return Err("Cannot toggle debug window while exiting".to_string());
    }

    // 重入保护：防止并发调用
    static TOGGLE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    static DEBUG_OPENED_ONCE: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    let lock = TOGGLE_LOCK.get_or_init(|| std::sync::Mutex::new(()));

    // 尝试获取锁，如果失败说明正在执行
    let _guard = match lock.try_lock() {
        Ok(g) => g,
        Err(_) => {
            startup_log("TOGGLE_REENTRY: already running, skipping");
            return Err("Toggle already in progress".to_string());
        }
    };

    // 使用 catch_unwind 捕获 panic，防止程序崩溃
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // T0: 函数入口
        startup_log("TOGGLE_T0: entered");

        // T1: 检查窗口是否存在
        let window = match app.get_webview_window("debug") {
            Some(w) => {
                startup_log("TOGGLE_T1: got window (Some)");
                w
            }
            None => {
                startup_log("TOGGLE_T1: got window (None)");
                return Err("Debug window is not initialized.".to_string());
            }
        };

        // T2: 获取可见状态
        let is_visible = match window.is_visible() {
            Ok(v) => {
                startup_log(&format!("TOGGLE_T2: is_visible Ok({})", v));
                v
            }
            Err(e) => {
                startup_log(&format!("TOGGLE_T2: is_visible Err({:?})", e));
                false
            }
        };

        // T3: 计算新状态
        let new_state = !is_visible;
        startup_log(&format!("TOGGLE_T3: new_state={}", new_state));

        // 标记：第一次打开 debug 窗口
        if new_state && !DEBUG_OPENED_ONCE.swap(true, std::sync::atomic::Ordering::SeqCst) {
            startup_log("TOGGLE_DEBUG_OPENED_ONCE: true (第一次打开 debug 窗口)");
        }

        // T4: 准备执行窗口操作
        startup_log("TOGGLE_T4: before spawn");

        // 异步执行窗口操作
        let window_clone = window.clone();
        std::thread::spawn(move || {
            startup_log("TOGGLE_T4.1: inside spawn");

            // 如果正在退出，不执行窗口操作
            if EXITING.load(std::sync::atomic::Ordering::SeqCst) {
                startup_log("TOGGLE: REJECTED in spawn due to EXITING=true");
                return;
            }

            if new_state {
                // T5: 执行 show
                startup_log("TOGGLE_T5: calling show");
                match window_clone.show() {
                    Ok(_) => {
                        startup_log("TOGGLE_SHOW: Ok");
                        match window_clone.set_focus() {
                            Ok(_) => {}
                            Err(e) => {
                                startup_log(&format!("TOGGLE_SET_FOCUS: Err({:?})", e));
                            }
                        }
                    }
                    Err(e) => {
                        startup_log(&format!("TOGGLE_SHOW: Err({:?})", e));
                    }
                }
                // T6: 更新状态
                startup_log("TOGGLE_T6: updating state (show)");
                debug::set_debug_window_state(true);

                // 延迟调用 set_window_open，避免在窗口操作期间触发 emit
                std::thread::sleep(std::time::Duration::from_millis(50));
                debug_log_bus::set_window_open(true);
            } else {
                // T5: 执行 hide
                startup_log("TOGGLE_T5: calling hide");
                match window_clone.hide() {
                    Ok(_) => {
                        startup_log("TOGGLE_HIDE: Ok");
                    }
                    Err(e) => {
                        startup_log(&format!("TOGGLE_HIDE: Err({:?})", e));
                    }
                }
                // T6: 更新状态
                startup_log("TOGGLE_T6: updating state (hide)");
                debug::set_debug_window_state(false);

                // 延迟调用 set_window_open，避免在窗口操作期间触发 emit
                std::thread::sleep(std::time::Duration::from_millis(50));
                debug_log_bus::set_window_open(false);
            }

            startup_log("TOGGLE_T7: spawn completed");
        });

        // T7: 返回
        startup_log(&format!("TOGGLE_T7: returning {}", new_state));
        Ok(new_state)
    }));

    // 处理 panic
    match result {
        Ok(r) => r,
        Err(panic_info) => {
            let panic_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };
            startup_log(&format!("TOGGLE_PANIC: {}", panic_msg));
            Err(format!("Panic in toggle_debug_window: {}", panic_msg))
        }
    }
}

#[tauri::command]
fn debug_log(app: AppHandle, level: String, message: String) {
    let _ = app.emit(
        "debug_log",
        serde_json::json!({
          "level": level,
          "message": message
        }),
    );
}

#[tauri::command]
fn get_debug_stats() -> debug_log_bus::LogBusStats {
    debug_log_bus::get_stats()
}

#[tauri::command]
fn debug_get_recent_logs(limit: usize) -> Vec<debug_log_bus::LogEvent> {
    debug_log_bus::get_recent_logs(limit)
}

// main window helpers moved to launcher.rs

fn init_logging(app: &tauri::App) -> Result<std::path::PathBuf, String> {
    let log_path = app
        .path()
        .resolve("logs/rocoknight.log", BaseDirectory::AppData)
        .map_err(|_| "Failed to resolve logs directory.".to_string())?;

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|_| "Failed to create log directory.".to_string())?;
    }
    trim_log_file(&log_path);

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|_| "Failed to open log file.".to_string())?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file);
    std::mem::forget(guard);

    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());

    // 桥接 log crate 到 tracing
    tracing_log::LogTracer::init().ok();

    // 创建多层订阅器：文件输出 + Debug Console
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false);

    let debug_console_layer = debug_console_layer::DebugConsoleLayer::new();

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(debug_console_layer)
        .try_init()
        .ok();

    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };

        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "unknown location".to_string());

        let panic_msg = format!("PANIC: {} at {}", message, location);

        // 记录到 tracing（会进入 Debug Console）
        error!("{}", panic_msg);

        // 记录到 startup log
        startup_log(&panic_msg);

        // 尝试获取 backtrace（需要 RUST_BACKTRACE=1）
        if std::env::var("RUST_BACKTRACE").is_ok() {
            let backtrace = std::backtrace::Backtrace::capture();
            let backtrace_str = format!("{:?}", backtrace);
            error!("Backtrace:\n{}", backtrace_str);
            startup_log(&format!("Backtrace:\n{}", backtrace_str));
        }
    }));

    info!("logging initialized: {}", log_path.display());
    Ok(log_path)
}

fn main() {
    let _ = set_dpi_awareness();
    init_startup_log();

    // 🔴 验证标记：如果看到这行，说明是新编译的版本
    startup_log("🔴🔴🔴 VERSION: 2026-02-12-PATCH-V2 🔴🔴🔴");

    // [日志点 1] 应用启动
    dbglog!(INFO, "Application starting...");

    show_boot_message("A: main entered");

    let context = tauri::generate_context!();
    show_boot_message("B: tauri context loaded");

    let app_result = tauri::Builder::default()
        .manage(Mutex::new(AppState::new()))
        .setup(|app| {
            // [日志点 2] Setup 开始
            dbglog!(INFO, "Setup phase started");
            show_boot_message("C: setup entered");
            app.handle()
                .plugin(
                    tauri_plugin_log::Builder::default()
                        .level(LevelFilter::Info)
                        .build(),
                )
                .map_err(|e| format!("log plugin init failed: {e}"))?;
            match init_logging(app) {
                Ok(path) => info!("log file at {}", path.display()),
                Err(msg) => error!("logging init failed: {msg}"),
            }

            let main_window = app.get_window("main").ok_or_else(|| {
                error!("main window not found");
                startup_log("main window not found");
                "Main window missing.".to_string()
            })?;
            show_boot_message("D: main window created");

            let monitor = main_window.current_monitor().ok().flatten();
            let screen_size = monitor
                .as_ref()
                .map(|m| *m.size())
                .unwrap_or_else(|| PhysicalSize::new(1920, 1080));
            let scale_factor = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
            let size = compute_window_size(screen_size, scale_factor);
            let _ = main_window.set_size(Size::Physical(size));
            let _ = main_window.set_resizable(false);
            let _ = main_window.set_min_size(Some(Size::Physical(size)));
            let _ = main_window.set_max_size(Some(Size::Physical(size)));
            center_window(&main_window, size);
            align_window_height_for_game_ratio(&main_window);
            if let Ok(actual) = main_window.inner_size() {
                center_window(&main_window, actual);
            }
            if let Ok(hwnd) = main_window.hwnd() {
                disable_maximize_resize(hwnd);
            }
            let _ = main_window.show();
            startup_log("main window show called");

            if let Ok(projector_path) = app.path().resolve("projector.exe", BaseDirectory::Resource)
            {
                if std::fs::metadata(&projector_path).is_err() {
                    show_error_message(&format!(
                        "projector.exe not found. resolved: {}",
                        projector_path.display()
                    ));
                }
            } else {
                show_error_message("projector.exe resolve failed.");
            }

            let app_handle = app.handle().clone();
            let nav_handle = app.handle().clone();
            let login_builder = WebviewBuilder::new(
                "login",
                WebviewUrl::External(
                    "https://17roco.qq.com/login.html"
                        .parse()
                        .map_err(|_| "Invalid login URL.".to_string())?,
                ),
            )
            .on_navigation(move |url| {
                if url.path().contains("login.html") {
                    let _ = start_login3_capture(
                        nav_handle.clone(),
                        nav_handle.state::<Mutex<AppState>>(),
                    );
                }
                resize_login_to_window(&nav_handle);
                schedule_login_layout(nav_handle.clone());
                true
            })
            .on_new_window(move |_url, _features| tauri::webview::NewWindowResponse::Allow);

            let scale = main_window.scale_factor().unwrap_or(1.0);
            let logical_w = ((size.width as f64) / scale).round() as i32;
            let logical_h = ((size.height as f64) / scale).round() as i32;
            let login_pos = LogicalPosition::new(0, UI_BAR_HEIGHT as i32);
            let login_size = LogicalSize::new(logical_w, (logical_h - UI_BAR_HEIGHT as i32).max(1));

            let login_webview = main_window
                .add_child(login_builder, login_pos, login_size)
                .map_err(|_| {
                    error!("failed to create login webview");
                    startup_log("failed to create login webview");
                    "Failed to create login webview.".to_string()
                })?;
            let toolbar_builder =
                WebviewBuilder::new("toolbar", WebviewUrl::App("toolbar.html".into()));
            let toolbar_pos = LogicalPosition::new(0, 0);
            let toolbar_size = LogicalSize::new(logical_w, UI_BAR_HEIGHT as i32);
            let toolbar_webview = main_window
                .add_child(toolbar_builder, toolbar_pos, toolbar_size)
                .map_err(|_| {
                    error!("failed to create toolbar webview");
                    startup_log("failed to create toolbar webview");
                    "Failed to create toolbar webview.".to_string()
                })?;

            startup_log("login webview created");
            startup_log("toolbar webview created");
            let _ = login_webview.with_webview(move |webview| {
                login3_capture::attach_webview2_capture(webview, app_handle.clone());
            });

            resize_login_to_window(&app.handle().clone());
            schedule_login_layout(app.handle().clone());
            let _ = login_webview.show();
            let _ = toolbar_webview.show();
            let app_handle_for_theme = app.handle().clone();
            let state_for_theme = app_handle_for_theme.state::<Mutex<AppState>>();
            let current_theme = with_state(&state_for_theme, |s| s.theme_mode);
            apply_theme_to_app(&app_handle_for_theme, current_theme);

            // Pre-create debug window hidden. Toolbar only controls show/hide.
            let setup_app_handle = app.handle().clone();
            startup_log("DEBUG_WINDOW_CREATE: Starting");
            // [日志点 3] Debug 窗口创建开始
            dbglog!(INFO, "Creating debug window...");
            let debug_window = tauri::WebviewWindowBuilder::new(
                &setup_app_handle,
                "debug",
                tauri::WebviewUrl::App("debug.html".into()),
            )
            .title("Debug Console")
            .inner_size(800.0, 600.0)
            .resizable(true)
            .maximizable(false)
            .visible(false)
            .build()
            .map_err(|e| {
                startup_log(&format!("DEBUG_WINDOW_CREATE: Err({:?})", e));
                // [日志点 4] Debug 窗口创建失败
                dbglog!(ERROR, "Failed to create debug window: {:?}", e);
                format!("Failed to create debug window: {}", e)
            })?;
            startup_log("DEBUG_WINDOW_CREATE: Ok");
            // [日志点 5] Debug 窗口创建成功
            dbglog!(INFO, "Debug window created successfully");
            debug::set_debug_window_state(false);

            // 全局退出标志（用于在退出时拒绝所有 debug 操作）
            static EXITING_GLOBAL: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);

            let debug_window_for_events = debug_window.clone();
            debug_window.on_window_event(move |event| {
                // 如果正在退出，不处理 debug 窗口事件
                if EXITING.load(std::sync::atomic::Ordering::SeqCst) {
                    startup_log("DEBUG_EVENT: REJECTED due to EXITING=true");
                    return;
                }

                // 重入保护
                static DEBUG_CLOSING: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);

                match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        // DW_CP1: 进入 CloseRequested
                        startup_log("DW_CP1: DEBUG_CLOSE_REQUESTED");

                        // 重入保护：如果已经在处理关闭，直接返回
                        if DEBUG_CLOSING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            startup_log("DW_CP_REENTRY: already closing, skipping");
                            return;
                        }

                        // DW_CP2: 调用 prevent_close
                        startup_log("DW_CP2: calling api.prevent_close()");
                        api.prevent_close();

                        // DW_CP3: 准备 hide
                        startup_log("DW_CP3: about to hide()");

                        // 直接 hide，不要在回调里做复杂操作
                        match debug_window_for_events.hide() {
                            Ok(_) => {
                                startup_log("DW_CP4: hide() = Ok");
                            }
                            Err(e) => {
                                startup_log(&format!("DW_CP4: hide() = Err({:?})", e));
                            }
                        }

                        // DW_CP5: 更新状态
                        startup_log("DW_CP5: updating state (minimal)");

                        // 只更新最基本的状态，不调用任何可能触发 tracing/emit 的函数
                        debug::set_debug_window_state(false);

                        // DW_CP6: 完成
                        startup_log("DW_CP6: DEBUG_CLOSE_HANDLED");

                        // 重置重入保护标志
                        DEBUG_CLOSING.store(false, std::sync::atomic::Ordering::SeqCst);
                    }
                    tauri::WindowEvent::Destroyed => {
                        startup_log("DEBUG_DESTROYED: start");
                        debug::set_debug_window_state(false);
                        startup_log("DEBUG_DESTROYED: end");
                    }
                    _ => {}
                }
            });

            // 初始化日志总线
            debug_log_bus::init(app.handle().clone());

            debug::init_debug(app.handle().clone());
            debug_info!("Application initialized successfully");

            Ok(())
        })
        .on_window_event(|window, event| {
            // 只处理主窗口的事件，忽略其他窗口（如debug窗口）
            if window.label() != "main" {
                return;
            }

            if let WindowEvent::CloseRequested { .. } = event {
                startup_log("MAIN_WINDOW_CLOSE: calling request_exit()");
                request_exit();
                // request_exit() 会在 100ms 内强制退出进程
                // 不需要任何其他操作
            } else if let WindowEvent::Resized(size) = event {
                track_last_size(*size);
                let state = window.state::<Mutex<AppState>>();
                if let Ok(guard) = state.lock() {
                    let should_resize_login = guard.projector.is_none();
                    drop(guard);
                    if should_resize_login {
                        resize_login_to_window(&window.app_handle());
                    }
                }
                resize_projector_to_window(&window.app_handle(), &state);
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_login_bounds,
            show_login_webview,
            hide_login_webview,
            get_theme_mode,
            set_theme_mode,
            start_login3_capture,
            stop_login3_capture,
            launch_projector,
            resize_projector,
            stop_projector,
            restart_projector,
            change_channel,
            reset_to_login,
            toggle_debug_window,
            debug_log,
            get_debug_stats,
            debug_get_recent_logs
        ])
        .run(context);

    if let Err(err) = app_result {
        error!("tauri run error: {err}");
        startup_log(&format!("tauri run error: {err}"));
    }
}
