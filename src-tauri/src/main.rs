#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod embed_win32;
mod launcher;
mod login3_capture;
mod projector;
mod state;
mod wpe;
mod debug;

use std::io::Write;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri::webview::WebviewBuilder;
use tauri_utils::config::WebviewUrl;
use tauri::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Size, State};
use tauri::path::BaseDirectory;
use log::LevelFilter;
use tracing::{error, info};

use crate::embed_win32::{disable_maximize_resize, parent_client_size, set_dpi_awareness};
use crate::launcher::{resize_login_to_window, resize_projector_to_window, schedule_login_layout, stop_projector as stop_projector_state};
use crate::state::{emit_status, AppState, AppStatus, ThemeMode};

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

static STARTUP_LOG: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();

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
      if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
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
  webview.show().map_err(|_| "Failed to show login view.".to_string())?;
  Ok(())
}

#[tauri::command]
fn hide_login_webview(app: AppHandle) -> Result<(), String> {
  let webview = app
    .get_webview("login")
    .ok_or_else(|| "Login WebView not found.".to_string())?;
  webview.hide().map_err(|_| "Failed to hide login view.".to_string())?;
  Ok(())
}

#[tauri::command]
fn get_theme_mode(state: State<Mutex<AppState>>) -> String {
  with_state(&state, |s| s.theme_mode.as_str().to_string())
}

#[tauri::command]
fn set_theme_mode(app: AppHandle, state: State<Mutex<AppState>>, theme: String) -> Result<String, String> {
  let mode = parse_theme_mode(&theme).ok_or_else(|| "Invalid theme. Use 'dark' or 'light'.".to_string())?;
  with_state(&state, |s| {
    s.theme_mode = mode;
  });
  apply_theme_to_app(&app, mode);
  Ok(mode.as_str().to_string())
}

#[tauri::command]
fn start_login3_capture(app: AppHandle, state: State<Mutex<AppState>>) -> Result<(), String> {
  startup_log("start_login3_capture");
  login3_capture::start(app, state)
}

#[tauri::command]
fn stop_login3_capture(app: AppHandle, state: State<Mutex<AppState>>) {
  login3_capture::stop(app, state);
}

#[tauri::command]
fn launch_projector(app: AppHandle, state: State<Mutex<AppState>>, rect: Rect) -> Result<(), String> {
  let _ = rect;
  crate::launcher::launch_projector_auto(&app, &state)
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
  stop_projector_command(&state);
  emit_status(&app, &state.lock().expect("state lock"));
}

#[tauri::command]
fn restart_projector(app: AppHandle, state: State<Mutex<AppState>>, rect: Rect) -> Result<(), String> {
  stop_projector_command(&state);
  launch_projector(app, state, rect)
}

#[tauri::command]
fn change_channel(app: AppHandle, state: State<Mutex<AppState>>) -> Result<(), String> {
  let (has_projector, has_swf) = with_state(&state, |s| (s.projector.is_some(), s.swf_url.is_some()));
  if !has_projector {
    info!("[RocoKnight][channel] projector not running; fallback to relogin");
    return reset_to_login(app, state);
  }
  if !has_swf {
    return Err("Missing swf url.".to_string());
  }
  info!("[RocoKnight][channel] change_channel invoked");
  crate::launcher::launch_projector_auto(&app, &state)?;
  info!("[RocoKnight][channel] change_channel done");
  Ok(())
}

#[tauri::command]
fn reset_to_login(app: AppHandle, state: State<Mutex<AppState>>) -> Result<(), String> {
  info!("[RocoKnight][reset] command invoked");
  stop_projector_command(&state);
  login3_capture::stop_timer_only(&state);
  with_state(&state, |s| {
    s.status = AppStatus::Login;
    s.message = None;
    s.swf_url = None;
  });
  if let Some(main) = app.get_webview("main") {
    let _ = main.show();
  }
  let login = app
    .get_webview("login")
    .ok_or_else(|| "Login WebView not found.".to_string())?;
  login
    .show()
    .map_err(|_| "Failed to show login webview.".to_string())?;
  let url = "https://17roco.qq.com/login.html"
    .parse()
    .map_err(|_| "Invalid login URL.".to_string())?;
  login
    .navigate(url)
    .map_err(|_| "Failed to navigate login webview.".to_string())?;
  info!("[RocoKnight][reset] login webview shown and navigated");
  resize_login_to_window(&app);
  schedule_login_layout(app.clone());
  emit_status(&app, &state.lock().expect("state lock"));
  info!("[RocoKnight][reset] status emitted");
  Ok(())
}

#[tauri::command]
fn toggle_debug_window(app: AppHandle) -> Result<(), String> {
  if let Some(window) = app.get_webview_window("debug") {
    // 窗口已存在，切换显示/隐藏状态
    if window.is_visible().unwrap_or(false) {
      window.hide().map_err(|e| format!("Failed to hide debug window: {}", e))?;
      debug::set_debug_window_state(false);
    } else {
      window.show().map_err(|e| format!("Failed to show debug window: {}", e))?;
      debug::set_debug_window_state(true);
    }
    return Ok(());
  }

  // 窗口不存在，创建新窗口
  let window = tauri::WebviewWindowBuilder::new(
    &app,
    "debug",
    tauri::WebviewUrl::App("debug.html".into())
  )
  .title("Debug Console")
  .inner_size(800.0, 600.0)
  .resizable(true)
  .maximizable(false)
  .build()
  .map_err(|e| format!("Failed to create debug window: {}", e))?;

  // 标记debug窗口已打开
  debug::set_debug_window_state(true);

  // 监听窗口事件
  window.on_window_event(move |event| {
    match event {
      tauri::WindowEvent::CloseRequested { api, .. } => {
        // 阻止窗口关闭，改为隐藏
        api.prevent_close();
        let _ = event.window().hide();
        debug::set_debug_window_state(false);
      }
      _ => {}
    }
  });

  Ok(())
}

#[tauri::command]
fn debug_log(app: AppHandle, level: String, message: String) {
  let _ = app.emit("debug_log", serde_json::json!({
    "level": level,
    "message": message
  }));
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

  let filter = tracing_subscriber::EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| "info".into());

  tracing_subscriber::fmt()
    .with_writer(non_blocking)
    .with_env_filter(filter)
    .with_ansi(false)
    .try_init()
    .ok();

  std::panic::set_hook(Box::new(|info| {
    error!("panic: {info}");
    startup_log(&format!("panic: {info}"));
  }));

  info!("logging initialized: {}", log_path.display());
  Ok(log_path)
}

fn main() {
  let _ = set_dpi_awareness();
  init_startup_log();
  show_boot_message("A: main entered");

  let context = tauri::generate_context!();
  show_boot_message("B: tauri context loaded");

  let app_result = tauri::Builder::default()
    .manage(Mutex::new(AppState::new()))
    .setup(|app| {
      show_boot_message("C: setup entered");
      app
        .handle()
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

      let main_window = app
        .get_window("main")
        .ok_or_else(|| {
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

      if let Ok(projector_path) = app
        .path()
        .resolve("projector.exe", BaseDirectory::Resource)
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
            let _ = start_login3_capture(nav_handle.clone(), nav_handle.state::<Mutex<AppState>>());
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
      let toolbar_builder = WebviewBuilder::new("toolbar", WebviewUrl::App("toolbar.html".into()));
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

      debug::init_debug(app.handle().clone());
      debug_info!("Application initialized successfully");

      Ok(())
    })
    .on_window_event(|window, event| {
      if let WindowEvent::CloseRequested { .. } = event {
        let state = window.state::<Mutex<AppState>>();
        stop_projector_command(&state);
        info!("window close requested");
        startup_log("window close requested");
      } else if let WindowEvent::Resized(size) = event {
        track_last_size(*size);
        let state = window.state::<Mutex<AppState>>();
        let should_resize_login = with_state(&state, |s| s.projector.is_none());
        if should_resize_login {
          resize_login_to_window(&window.app_handle());
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
      debug_log
    ])
    .run(context);

  if let Err(err) = app_result {
    error!("tauri run error: {err}");
    startup_log(&format!("tauri run error: {err}"));
  }
}
