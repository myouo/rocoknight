#[cfg(target_os = "windows")]
use std::ffi::OsStr;
use std::fs;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
#[cfg(not(target_os = "windows"))]
use std::process::{Command, Stdio};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
use tracing::{error, info};
use url::Url;

use crate::state::ProjectorProcess;

pub fn resolve_projector_path(app: &AppHandle) -> Result<PathBuf, String> {
    let resolved = app
        .path()
        .resolve("projector.exe", BaseDirectory::Resource)
        .map_err(|_| "Failed to resolve resource directory.".to_string())?;
    if fs::metadata(&resolved).is_ok() {
        info!("projector path resolved: {}", resolved.display());
        return Ok(resolved);
    }

    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|_| "Failed to get resource directory.".to_string())?;
    let fallback = resource_dir.join("projector.exe");
    if fs::metadata(&fallback).is_ok() {
        info!("projector path resolved (fallback): {}", fallback.display());
        return Ok(fallback);
    }

    if let Ok(mut exe) = std::env::current_exe() {
        exe.pop();
        let candidates = [
            exe.join("resources").join("projector.exe"),
            exe.join("..").join("resources").join("projector.exe"),
            exe.join("..")
                .join("..")
                .join("resources")
                .join("projector.exe"),
            exe.join("..")
                .join("..")
                .join("debug")
                .join("resources")
                .join("projector.exe"),
            exe.join("..")
                .join("..")
                .join("release")
                .join("resources")
                .join("projector.exe"),
        ];
        for candidate in candidates {
            if fs::metadata(&candidate).is_ok() {
                info!(
                    "projector path resolved (exe fallback): {}",
                    candidate.display()
                );
                return Ok(candidate);
            }
        }
    }

    Err(format!(
        "Failed to locate projector.exe. Checked: {}, {}, and dev resources.",
        resolved.display(),
        fallback.display()
    ))
}

#[cfg(target_os = "windows")]
pub fn launch_projector(path: &PathBuf, swf_url: &str) -> Result<ProjectorProcess, String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CreateProcessW, CREATE_NO_WINDOW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
        STARTF_USESHOWWINDOW, STARTUPINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    info!(
        "launching projector: {} {}",
        path.display(),
        sanitize_url_for_log(swf_url)
    );

    let app_w: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let cmd = format!("\"{}\" {}", path.display(), swf_url);
    let mut cmd_w: Vec<u16> = OsStr::new(&cmd)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut si = STARTUPINFOW::default();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    si.dwFlags = STARTF_USESHOWWINDOW;
    si.wShowWindow = SW_HIDE.0 as u16;

    let mut pi = PROCESS_INFORMATION::default();
    let launch_result = unsafe {
        CreateProcessW(
            PCWSTR(app_w.as_ptr()),
            Some(PWSTR(cmd_w.as_mut_ptr())),
            None,
            None,
            false,
            PROCESS_CREATION_FLAGS(CREATE_NO_WINDOW.0),
            None,
            PCWSTR::null(),
            &si,
            &mut pi,
        )
    };
    if let Err(err) = launch_result {
        error!("launch projector failed: CreateProcessW: {err}");
        return Err("Failed to launch projector.".to_string());
    }

    unsafe {
        let _ = CloseHandle(pi.hThread);
    }

    Ok(ProjectorProcess {
        handle: pi.hProcess,
        pid: pi.dwProcessId,
    })
}

#[cfg(not(target_os = "windows"))]
pub fn launch_projector(path: &PathBuf, swf_url: &str) -> Result<ProjectorProcess, String> {
    info!(
        "launching projector: {} {}",
        path.display(),
        sanitize_url_for_log(swf_url)
    );
    let mut child = Command::new(path)
        .arg(swf_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            error!("launch projector failed: {err}");
            "Failed to launch projector.".to_string()
        })?;
    let pid = child.id();
    Ok(ProjectorProcess { child, pid })
}

#[cfg(target_os = "windows")]
pub fn stop_projector(process: &mut ProjectorProcess) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::TerminateProcess;
    unsafe {
        let _ = TerminateProcess(process.handle, 1);
        let _ = CloseHandle(process.handle);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn stop_projector(process: &mut ProjectorProcess) {
    let _ = process.child.kill();
    let _ = process.child.wait();
}

fn sanitize_url_for_log(url: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return "<invalid-url>".to_string();
    };
    let origin = parsed.origin().ascii_serialization();
    let path = parsed.path();
    let mut keys: Vec<String> = Vec::new();
    for (k, _v) in parsed.query_pairs() {
        keys.push(k.to_string());
    }
    keys.sort();
    keys.dedup();
    if keys.is_empty() {
        format!("{origin}{path}")
    } else {
        format!("{origin}{path} ?keys={}", keys.join(","))
    }
}
