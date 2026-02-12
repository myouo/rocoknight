use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use tauri::PhysicalSize;
use tauri::{AppHandle, Manager, State};
use windows::Win32::Foundation::HWND;

// 全局退出标志，用于控制调度线程停止
static SHOULD_EXIT_SCHEDULES: AtomicBool = AtomicBool::new(false);

use crate::embed_win32::{
    attach_child, bring_to_top, detach_child, find_window_by_pid, hide_window, move_child,
    parent_client_size,
};
use crate::projector::{resolve_projector_path, stop_projector as kill_projector};
use crate::state::{emit_status, AppState, AppStatus, ProjectorHandle};
use crate::wpe::{PacketInjector, PacketInterceptor};
use tracing::info;

const LOGIN_ZOOM: f64 = 1.17;
const UI_BAR_HEIGHT: i32 = 36;

fn extract_qq_from_url(url: &str) -> Option<u64> {
    url::Url::parse(url).ok().and_then(|parsed| {
        parsed
            .query_pairs()
            .find(|(key, _)| key == "qq" || key == "uin")
            .and_then(|(_, value)| value.parse::<u64>().ok())
    })
}

fn main_window(app: &AppHandle) -> Result<tauri::Window, String> {
    app.get_window("main")
        .ok_or_else(|| "Main window not found.".to_string())
}

fn main_hwnd(app: &AppHandle) -> Result<HWND, String> {
    let window = main_window(app)?;
    window
        .hwnd()
        .map_err(|_| "Failed to get main window handle.".to_string())
}

fn main_window_size_physical(app: &AppHandle) -> Result<PhysicalSize<u32>, String> {
    let window = main_window(app)?;
    let size = window
        .inner_size()
        .map_err(|_| "Failed to get window size.".to_string())?;
    Ok(size)
}

fn main_window_scale(app: &AppHandle) -> f64 {
    if let Ok(window) = main_window(app) {
        if let Ok(scale) = window.scale_factor() {
            return scale;
        }
    }
    1.0
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
    emit_status(app, &state.lock().expect("state lock"));
}

pub fn stop_projector(state: &State<Mutex<AppState>>) {
    with_state(state, |s| {
        if let Some(mut projector) = s.projector.take() {
            detach_child(
                HWND(projector.hwnd as *mut std::ffi::c_void),
                projector.original_style,
            );
            kill_projector(&mut projector.process);
        }

        if let Some(interceptor) = s.wpe_interceptor.take() {
            info!("[WPE] Stopping interceptor");
            interceptor.stop();
        }

        s.status = AppStatus::Login;
        s.message = None;
        s.last_projector_rect = None;
        s.qq_num = None;
    });
}

pub fn launch_projector_auto(
    app: &AppHandle,
    state: &State<Mutex<AppState>>,
) -> Result<(), String> {
    tracing::info!("launch_projector_auto started");

    // 阶段 1：验证状态
    let (swf_url, existing) = {
        let _stage = crate::request_context::StageTimer::new("validate_state");
        let result = with_state(state, |s| (s.swf_url.clone(), s.projector.is_some()));
        tracing::info!(
            has_swf_url = result.0.is_some(),
            has_existing_projector = result.1,
            "state validated"
        );
        result
    };

    if existing {
        tracing::info!("stopping existing projector");
        stop_projector(state);
    }

    let swf_url = match swf_url {
        Some(url) => {
            tracing::info!(url_len = url.len(), "swf url available");
            url
        }
        None => {
            let msg = "Missing main.swf URL.".to_string();
            tracing::error!("missing swf url");
            set_error(app, state, msg.clone());
            return Err(msg);
        }
    };

    // 阶段 2：解析投影器路径
    let projector_path = {
        let _stage = crate::request_context::StageTimer::new("resolve_path");
        match resolve_projector_path(app) {
            Ok(path) => {
                tracing::info!(path = %path.display(), "projector path resolved");
                path
            }
            Err(msg) => {
                tracing::error!(error = %msg, "failed to resolve projector path");
                set_error(app, state, msg.clone());
                return Err(msg);
            }
        }
    };

    // 阶段 3：启动进程
    let (process, pid) = {
        let _stage = crate::request_context::StageTimer::new("launch_process");
        match crate::projector::launch_projector(&projector_path, &swf_url) {
            Ok(process) => {
                let pid = process.pid;
                tracing::info!(pid = pid, "process launched");
                (process, pid)
            }
            Err(msg) => {
                tracing::error!(error = %msg, "failed to launch process");
                set_error(app, state, msg.clone());
                return Err(msg);
            }
        }
    };

    // 阶段 4：查找窗口
    let child_hwnd = {
        let _stage = crate::request_context::StageTimer::new("find_window");
        match find_window_by_pid(pid, 6000) {
            Ok(hwnd) => {
                tracing::info!(hwnd = hwnd.0 as usize, "window found");
                hwnd
            }
            Err(msg) => {
                tracing::error!(error = %msg, pid = pid, "failed to find window");
                set_error(app, state, msg.clone());
                return Err(msg);
            }
        }
    };

    // 阶段 5：嵌入窗口
    let original_style = {
        let _stage = crate::request_context::StageTimer::new("attach_window");

        hide_window(child_hwnd);

        let main_hwnd = match main_hwnd(app) {
            Ok(hwnd) => hwnd,
            Err(msg) => {
                tracing::error!(error = %msg, "failed to get main window handle");
                set_error(app, state, msg.clone());
                return Err(msg);
            }
        };

        match attach_child(child_hwnd, main_hwnd) {
            Ok(style) => {
                tracing::info!(
                    child_hwnd = child_hwnd.0 as usize,
                    parent_hwnd = main_hwnd.0 as usize,
                    "window attached"
                );
                style
            }
            Err(msg) => {
                tracing::error!(error = %msg, "failed to attach window");
                set_error(app, state, msg.clone());
                return Err(msg);
            }
        }
    };

    // 阶段 6：调整窗口大小
    {
        let _stage = crate::request_context::StageTimer::new("resize_window");

        if let Some((w, h)) = parent_client_size(main_hwnd(app).unwrap()) {
            let scale = main_window_scale(app);
            let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
            let usable_h = (h - bar_h).max(1);
            move_child(child_hwnd, 0, bar_h, w, usable_h);
            tracing::info!(width = w, height = usable_h, "window resized");
        } else {
            let size = main_window_size_physical(app)?;
            let scale = main_window_scale(app);
            let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
            let usable_h = (size.height as i32 - bar_h).max(1);
            move_child(child_hwnd, 0, bar_h, size.width as i32, usable_h);
            tracing::info!(
                width = size.width,
                height = usable_h,
                "window resized (fallback)"
            );
        }

        bring_to_top(child_hwnd);
        schedule_projector_fit(app.clone());
    }

    // 阶段 7：初始化 WPE
    let qq_num = extract_qq_from_url(&swf_url).unwrap_or(0);
    tracing::info!(qq_num = qq_num, "qq number extracted");

    let _interceptor = {
        let _stage = crate::request_context::StageTimer::new("init_wpe");

        let _injector = match PacketInjector::new(pid) {
            Ok(inj) => {
                tracing::info!("packet injector created");
                Arc::new(inj)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to create packet injector");
                return Err(format!("Failed to create packet injector: {}", e));
            }
        };

        match PacketInterceptor::new(pid) {
            Ok(int) => {
                tracing::info!("packet interceptor created");
                int
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to create packet interceptor");
                return Err(format!("Failed to create packet interceptor: {}", e));
            }
        }
    };

    // 阶段 8：更新状态
    {
        let _stage = crate::request_context::StageTimer::new("update_state");

        with_state(state, |s| {
            tracing::info!(
              old_status = ?s.status,
              new_status = ?AppStatus::Running,
              "state transition"
            );

            s.projector = Some(ProjectorHandle {
                process,
                hwnd: child_hwnd.0 as isize,
                original_style,
            });
            s.status = AppStatus::Running;
            s.message = None;
            s.last_projector_rect = None;
            s.qq_num = Some(qq_num);
            s.wpe_interceptor = Some(_interceptor);
        });

        emit_status(app, &state.lock().expect("state lock"));
    }

    // 阶段 9：隐藏登录窗口
    {
        let _stage = crate::request_context::StageTimer::new("hide_login");

        if let Some(login) = app.get_webview("login") {
            let _ = login.hide();
            tracing::info!("login webview hidden");
        }
        if let Some(main) = app.get_webview("main") {
            let _ = main.hide();
            tracing::info!("main webview hidden");
        }
    }

    tracing::info!("launch_projector_auto completed successfully");
    Ok(())
}

fn schedule_projector_fit(app: AppHandle) {
    std::thread::spawn(move || {
        let delays_ms = [50u64, 150, 300, 600, 1200, 2000];
        for delay in delays_ms {
            // 检查退出标志
            if SHOULD_EXIT_SCHEDULES.load(Ordering::Relaxed) {
                break;
            }

            std::thread::sleep(Duration::from_millis(delay));

            // sleep 后再次检查
            if SHOULD_EXIT_SCHEDULES.load(Ordering::Relaxed) {
                break;
            }

            let app_clone = app.clone();
            let app_for_task = app_clone.clone();
            let _ = app_clone.run_on_main_thread(move || {
                let state = app_for_task.state::<Mutex<AppState>>();
                resize_projector_to_window(&app_for_task, &state);
            });
        }
    });
}

pub fn resize_projector_to_window(app: &AppHandle, state: &State<Mutex<AppState>>) {
    let (projector, last_rect) = with_state(state, |s| {
        (s.projector.as_ref().map(|p| p.hwnd), s.last_projector_rect)
    });
    let Some(hwnd) = projector else {
        return;
    };

    let rect = if let Ok(parent) = main_hwnd(app) {
        if let Some((w, h)) = parent_client_size(parent) {
            let scale = main_window_scale(app);
            let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
            let usable_h = (h - bar_h).max(1);
            Some((0, bar_h, w, usable_h))
        } else {
            None
        }
    } else {
        None
    }
    .or_else(|| {
        main_window_size_physical(app).ok().map(|size| {
            let scale = main_window_scale(app);
            let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
            let usable_h = (size.height as i32 - bar_h).max(1);
            (0, bar_h, size.width as i32, usable_h)
        })
    });

    let Some((x, y, w, h)) = rect else {
        return;
    };
    if Some((x, y, w, h)) == last_rect {
        return;
    }

    move_child(HWND(hwnd as *mut std::ffi::c_void), x, y, w, h);
    bring_to_top(HWND(hwnd as *mut std::ffi::c_void));
    with_state(state, |s| {
        s.last_projector_rect = Some((x, y, w, h));
    });
}

pub fn resize_login_to_window(app: &AppHandle) {
    if let Ok(window) = main_window(app) {
        if let Ok(size) = window.inner_size() {
            let scale = window.scale_factor().unwrap_or(1.0);
            let w = ((size.width as f64) / scale).round() as i32;
            let h = ((size.height as f64) / scale).round() as i32;
            if let Some(login) = app.get_webview("login") {
                let usable_h = (h - UI_BAR_HEIGHT).max(1);
                let _ = login.set_position(tauri::LogicalPosition::new(0, UI_BAR_HEIGHT));
                let _ = login.set_size(tauri::LogicalSize::new(w, usable_h));
                let _ = login.set_zoom(LOGIN_ZOOM);
            }
            if let Some(toolbar) = app.get_webview("toolbar") {
                let _ = toolbar.set_position(tauri::LogicalPosition::new(0, 0));
                let _ = toolbar.set_size(tauri::LogicalSize::new(w, UI_BAR_HEIGHT));
            }
        }
    }
}

pub fn schedule_login_layout(app: AppHandle) {
    std::thread::spawn(move || {
        let delays_ms = [50u64, 150, 300, 600];
        for delay in delays_ms {
            // 检查退出标志
            if SHOULD_EXIT_SCHEDULES.load(Ordering::Relaxed) {
                break;
            }

            std::thread::sleep(Duration::from_millis(delay));

            // sleep 后再次检查
            if SHOULD_EXIT_SCHEDULES.load(Ordering::Relaxed) {
                break;
            }

            let app_clone = app.clone();
            let app_for_cb = app_clone.clone();
            let _ = app_clone.run_on_main_thread(move || {
                resize_login_to_window(&app_for_cb);
            });
        }
    });
}

/// 停止所有调度线程
pub fn stop_schedule_threads() {
    tracing::info!("[Launcher] Stopping schedule threads");
    SHOULD_EXIT_SCHEDULES.store(true, Ordering::SeqCst);
}
