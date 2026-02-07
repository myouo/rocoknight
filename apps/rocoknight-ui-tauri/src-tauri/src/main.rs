#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rocoknight_core::{CoreConfig, ProcessHandle, ProcessManager};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tauri::Emitter;

type SharedConfig = Arc<Mutex<CoreConfig>>;

#[derive(Default)]
struct LoginState {
    in_progress: Mutex<bool>,
}

#[derive(serde::Serialize)]
struct StatusPayload {
    status: &'static str,
}

#[derive(serde::Serialize)]
struct ErrorPayload {
    message: String,
}

#[tauri::command]
fn get_config(state: tauri::State<SharedConfig>) -> CoreConfig {
    state.lock().unwrap().clone()
}

#[tauri::command]
fn set_config(state: tauri::State<SharedConfig>, cfg: CoreConfig) {
    *state.lock().unwrap() = cfg;
}

#[tauri::command]
fn launch(state: tauri::State<SharedConfig>, manager: tauri::State<ProcessManager>) -> Result<ProcessHandle, String> {
    let cfg = state.lock().unwrap().clone();
    manager.launch_projector(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
fn stop(manager: tauri::State<ProcessManager>, handle: ProcessHandle) -> Result<(), String> {
    manager.stop(&handle).map_err(|e| e.to_string())
}

#[tauri::command]
fn is_running(manager: tauri::State<ProcessManager>, handle: ProcessHandle) -> bool {
    manager.is_running(&handle)
}

#[tauri::command]
async fn login_and_launch(
    app: AppHandle,
    state: tauri::State<'_, SharedConfig>,
    manager: tauri::State<'_, ProcessManager>,
    login_state: tauri::State<'_, LoginState>,
) -> Result<(), String> {
    {
        let mut guard = login_state.in_progress.lock().unwrap();
        if *guard {
            return Err("login already in progress".to_string());
        }
        *guard = true;
    }

    let cfg = state.lock().unwrap().clone();
    let projector = match cfg.launcher.projector_path.clone() {
        Some(path) => path,
        None => {
            let mut guard = login_state.in_progress.lock().unwrap();
            *guard = false;
            return Err("projector_path is required".to_string());
        }
    };

    let _ = app.emit(
        "login_status",
        StatusPayload {
            status: "WaitingForLogin",
        },
    );

    if let Some(existing) = app.get_webview_window("login") {
        let _ = existing.close();
    }

    let login_url = "https://17roco.qq.com/login.html";
    let window = WebviewWindowBuilder::new(&app, "login", WebviewUrl::External(login_url.parse().unwrap()))
        .title("RocoKnight Login")
        .inner_size(900.0, 720.0)
        .resizable(true)
        .build()
        .map_err(|e| {
            let mut guard = login_state.in_progress.lock().unwrap();
            *guard = false;
            e.to_string()
        })?;

    let (close_tx, mut close_rx) = tokio::sync::watch::channel::<bool>(false);
    let finished = Arc::new(AtomicBool::new(false));
    let finished_close = finished.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { .. } = event {
            if !finished_close.load(Ordering::SeqCst) {
                let _ = close_tx.send(true);
            }
        }
    });

    let app_clone = app.clone();
    let manager = manager.inner().clone();
    let login_state = login_state.inner().clone();
    let finished_task = finished.clone();

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(200));
        let timeout = Duration::from_secs(180);
        let start = tokio::time::Instant::now();

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if start.elapsed() > timeout {
                        finished_task.store(true, Ordering::SeqCst);
                        let _ = window.close();
                        let _ = app_clone.emit("login_error", ErrorPayload {
                            message: "登录超时，请重试".to_string(),
                        });
                        let _ = app_clone.emit("login_status", StatusPayload { status: "Idle" });
                        break;
                    }

                    if let Ok(current) = window.url() {
                        if let Some(matched_url) = match_main_swf(&current) {
                            let _ = app_clone.emit("login_status", StatusPayload { status: "Launching" });
                            finished_task.store(true, Ordering::SeqCst);
                            let _ = window.close();

                            let launch_result = manager.launch_projector_with_url(projector.clone(), matched_url);
                            match launch_result {
                                Ok(_) => {
                                    let _ = app_clone.emit("login_status", StatusPayload { status: "Running" });
                                }
                                Err(_) => {
                                    let _ = app_clone.emit("login_error", ErrorPayload {
                                        message: "启动失败，请检查 Projector 路径".to_string(),
                                    });
                                    let _ = app_clone.emit("login_status", StatusPayload { status: "Idle" });
                                }
                            }
                            break;
                        }
                    }
                }
                changed = close_rx.changed() => {
                    if changed.is_ok() && *close_rx.borrow() {
                        finished_task.store(true, Ordering::SeqCst);
                        let _ = app_clone.emit("login_error", ErrorPayload {
                            message: "登录窗口已关闭".to_string(),
                        });
                        let _ = app_clone.emit("login_status", StatusPayload { status: "Idle" });
                        break;
                    }
                }
            }
        }

        let mut guard = login_state.in_progress.lock().unwrap();
        *guard = false;
    });

    Ok(())
}

fn match_main_swf(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    if host != "res.17roco.qq.com" {
        return None;
    }
    if !url.path().contains("main.swf") {
        return None;
    }
    let query = url.query().unwrap_or("");
    if query.is_empty() {
        return None;
    }
    Some(url.as_str().to_string())
}

fn main() {
    rocoknight_core::logging::init_logging();

    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(CoreConfig::default())))
        .manage(ProcessManager::new())
        .manage(LoginState::default())
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config,
            launch,
            stop,
            is_running,
            login_and_launch
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
