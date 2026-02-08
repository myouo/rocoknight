mod embed_win32;
mod launcher;
mod login3_capture;
mod projector;
mod state;

use std::io::Write;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager, WindowEvent};
use tauri::webview::WebviewBuilder;
use tauri_utils::config::WebviewUrl;
use tauri::{LogicalPosition, LogicalSize, PhysicalSize, Size, State};
use tauri::path::BaseDirectory;
use tracing::{error, info};

use crate::launcher::{resize_login_to_window, resize_projector_to_window, stop_projector as stop_projector_state};
use crate::state::{emit_status, AppState, AppStatus};

const FIXED_W: u32 = 1440;
const FIXED_H: u32 = 840;
static LAST_WINDOW_SIZE: OnceLock<Mutex<Option<PhysicalSize<u32>>>> = OnceLock::new();

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

fn set_error(app: &AppHandle, state: &State<Mutex<AppState>>, msg: String) {
  with_state(state, |s| {
    s.status = AppStatus::Error;
    s.message = Some(msg.clone());
  });
  error!("{msg}");
  emit_status(app, &state.lock().expect("state lock"));
}

static STARTUP_LOG: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();

fn init_startup_log() {
  #[cfg(target_os = "windows")]
  {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
      let path = std::path::PathBuf::from(local)
        .join("RocoKnight")
        .join("logs")
        .join("startup.log");
      if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
      }
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

fn enforce_fixed_size(window: &tauri::Window, size: PhysicalSize<u32>) {
  if size.width == 0 || size.height == 0 {
    return;
  }
  let lock = LAST_WINDOW_SIZE.get_or_init(|| Mutex::new(None));
  let mut guard = lock.lock().expect("window size lock");

  if let Some(prev) = *guard {
    if prev.width == size.width && prev.height == size.height {
      return;
    }
  }

  let target = PhysicalSize::new(FIXED_W, FIXED_H);
  if size.width != FIXED_W || size.height != FIXED_H {
    let _ = window.set_size(Size::Physical(target));
  }
  *guard = Some(target);
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
fn reset_to_login(app: AppHandle, state: State<Mutex<AppState>>) {
  stop_projector_command(&state);
  login3_capture::stop_timer_only(&state);
  with_state(&state, |s| {
    s.status = AppStatus::Login;
    s.message = None;
    s.swf_url = None;
  });
  emit_status(&app, &state.lock().expect("state lock"));
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
  init_startup_log();
  show_boot_message("A: main entered");

  let context = tauri::generate_context!();
  show_boot_message("B: tauri context loaded");

  let app_result = tauri::Builder::default()
    .manage(Mutex::new(AppState::new()))
    .setup(|app| {
      show_boot_message("C: setup entered");
      match init_logging(app) {
        Ok(path) => info!("log file at {}", path.display()),
        Err(msg) => eprintln!("logging init failed: {msg}"),
      }

      let main_window = app
        .get_window("main")
        .ok_or_else(|| {
          error!("main window not found");
          startup_log("main window not found");
          "Main window missing.".to_string()
        })?;
      show_boot_message("D: main window created");

      let _ = main_window.set_size(Size::Physical(PhysicalSize::new(FIXED_W, FIXED_H)));
      let _ = main_window.set_resizable(false);
      let _ = main_window.set_min_size(Some(Size::Physical(PhysicalSize::new(FIXED_W, FIXED_H))));
      let _ = main_window.set_max_size(Some(Size::Physical(PhysicalSize::new(FIXED_W, FIXED_H))));
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
      let builder = WebviewBuilder::new(
        "login",
        WebviewUrl::External(
          "https://17roco.qq.com/login.html"
            .parse()
            .map_err(|_| "Invalid login URL.".to_string())?,
        ),
      )
        .on_navigation(move |url| {
          if url.as_str().starts_with("about:blank") || url.path().contains("login.html") {
            let _ = start_login3_capture(nav_handle.clone(), nav_handle.state::<Mutex<AppState>>());
          }
          true
        })
        .on_new_window(move |_url, _features| tauri::webview::NewWindowResponse::Allow);

      let login_webview = main_window
        .add_child(builder, LogicalPosition::new(0, 0), LogicalSize::new(1, 1))
        .map_err(|_| {
          error!("failed to create login webview");
          startup_log("failed to create login webview");
          "Failed to create login webview.".to_string()
        })?;

      startup_log("login webview created");
      let _ = login_webview.with_webview(move |webview| {
        login3_capture::attach_webview2_capture(webview, app_handle.clone());
      });

      resize_login_to_window(&app.handle().clone());
      let _ = login_webview.show();

      if let Err(err) = start_login3_capture(app.handle().clone(), app.handle().state::<Mutex<AppState>>()) {
        error!("start_login3_capture failed: {err}");
        startup_log(&format!("start_login3_capture failed: {err}"));
      }

      Ok(())
    })
    .on_window_event(|window, event| {
      if let WindowEvent::CloseRequested { .. } = event {
        let state = window.state::<Mutex<AppState>>();
        stop_projector_command(&state);
        info!("window close requested");
        startup_log("window close requested");
      } else if let WindowEvent::Resized(size) = event {
        enforce_fixed_size(window, *size);
        resize_login_to_window(&window.app_handle());
        resize_projector_to_window(&window.app_handle(), &window.state::<Mutex<AppState>>());
      }
    })
    .invoke_handler(tauri::generate_handler![
      set_login_bounds,
      show_login_webview,
      hide_login_webview,
      start_login3_capture,
      stop_login3_capture,
      launch_projector,
      resize_projector,
      stop_projector,
      restart_projector,
      reset_to_login
    ])
    .run(context);

  if let Err(err) = app_result {
    eprintln!("tauri run error: {err}");
    error!("tauri run error: {err}");
    startup_log(&format!("tauri run error: {err}"));
  }
}
