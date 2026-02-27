use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
use windows::Win32::System::Memory::{
    VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateRemoteThread, OpenProcess, PROCESS_ALL_ACCESS,
};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

static WPE_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

pub fn set_wpe_window_state(open: bool) {
    WPE_WINDOW_OPEN.store(open, Ordering::Relaxed);
}

pub struct PacketInjector;

impl PacketInjector {
    pub fn new(pid: u32) -> Result<Self, String> {
        let dll_bytes = include_bytes!("../../wpe-dll/target/i686-pc-windows-msvc/release/wpe_dll.dll");
        let mut temp_path = std::env::temp_dir();
        temp_path.push(format!("wpe_dll_{}.dll", pid));
        std::fs::write(&temp_path, dll_bytes).map_err(|e| format!("Failed to write DLL: {}", e))?;

        unsafe {
            let h_process = OpenProcess(PROCESS_ALL_ACCESS, false, pid).map_err(|e| e.to_string())?;
            
            let mut path_utf16: Vec<u16> = temp_path.to_string_lossy().encode_utf16().collect();
            path_utf16.push(0);
            let alloc_size = path_utf16.len() * 2;

            let alloc_mem = VirtualAllocEx(
                h_process,
                None,
                alloc_size,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            );

            if alloc_mem.is_null() {
                let _ = CloseHandle(h_process);
                return Err("Failed to allocate memory in target process".to_string());
            }

            let mut bytes_written = 0;
            let success = WriteProcessMemory(
                h_process,
                alloc_mem,
                path_utf16.as_ptr() as *const c_void,
                alloc_size,
                Some(&mut bytes_written),
            );

            if success.is_err() {
                let _ = VirtualFreeEx(h_process, alloc_mem, 0, MEM_RELEASE);
                let _ = CloseHandle(h_process);
                return Err("Failed to write memory".to_string());
            }

            let kernel32 = GetModuleHandleW(windows::core::w!("kernel32.dll")).map_err(|e| e.to_string())?;
            let load_library_addr = GetProcAddress(kernel32, windows::core::s!("LoadLibraryW"));

            if load_library_addr.is_none() {
                let _ = VirtualFreeEx(h_process, alloc_mem, 0, MEM_RELEASE);
                let _ = CloseHandle(h_process);
                return Err("Failed to find LoadLibraryW".to_string());
            }

            let thread_handle = CreateRemoteThread(
                h_process,
                None,
                0,
                Some(std::mem::transmute(load_library_addr.unwrap())),
                Some(alloc_mem),
                0,
                None,
            ).map_err(|e| e.to_string())?;

            windows::Win32::System::Threading::WaitForSingleObject(thread_handle, windows::Win32::System::Threading::INFINITE);

            let _ = CloseHandle(thread_handle);
            let _ = CloseHandle(h_process);
        }

        Ok(PacketInjector)
    }
}

pub struct PacketInterceptor {
    running: Arc<AtomicBool>,
}

impl PacketInterceptor {
    pub fn new(pid: u32, app: AppHandle) -> Result<Self, String> {
        let running = Arc::new(AtomicBool::new(true));
        let pipe_name = format!(r"\\.\pipe\RococnightWPE_{}", pid);
        let pipe_name_w: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();
        let r = running.clone();

        std::thread::spawn(move || {
            use windows::Win32::System::Pipes::CreateNamedPipeW;
            use windows::Win32::System::Pipes::ConnectNamedPipe;
            use windows::Win32::System::Pipes::DisconnectNamedPipe;
            use windows::Win32::Storage::FileSystem::ReadFile;
            use windows::Win32::System::Pipes::{PIPE_TYPE_MESSAGE, PIPE_READMODE_MESSAGE, PIPE_WAIT};
            use windows::Win32::Storage::FileSystem::PIPE_ACCESS_INBOUND;
            
            unsafe {
                let pipe: windows::Win32::Foundation::HANDLE = CreateNamedPipeW(
                    windows::core::PCWSTR(pipe_name_w.as_ptr()),
                    PIPE_ACCESS_INBOUND,
                    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                    1,
                    65536,
                    65536,
                    0,
                    None,
                );

                if !pipe.is_invalid() {
                    let connected = ConnectNamedPipe(pipe, None).is_ok() || windows::Win32::Foundation::GetLastError() == windows::Win32::Foundation::ERROR_PIPE_CONNECTED;
                    
                    if connected {
                        let mut buf = [0u8; 65536];
                        while r.load(Ordering::SeqCst) {
                            let mut bytes_read = 0;
                            let success = ReadFile(
                                pipe,
                                Some(&mut buf),
                                Some(&mut bytes_read),
                                None
                            );
                            if success.is_ok() {
                                if bytes_read > 0 {
                                    let data = &buf[..bytes_read as usize];
                                    if let Ok(json_str) = std::str::from_utf8(data) {
                                        for line in json_str.lines() {
                                            if !line.is_empty() {
                                                let _ = app.emit("wpe_packet", line);
                                            }
                                        }
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                    }
                    let _ = DisconnectNamedPipe(pipe);
                    let _ = CloseHandle(pipe);
                }
            }
        });

        Ok(Self { running })
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub fn toggle_wpe_window(app: AppHandle) -> Result<bool, String> {
    crate::request_context::wrap_command("toggle_wpe_window", 200, || {
        if crate::EXITING.load(Ordering::SeqCst) {
            return Err("Cannot toggle WPE window while exiting".to_string());
        }

        static TOGGLE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        let lock = TOGGLE_LOCK.get_or_init(|| std::sync::Mutex::new(()));
        let _guard = match lock.try_lock() {
            Ok(g) => g,
            Err(_) => {
                return Err("Toggle already in progress".to_string());
            }
        };

        let window = app
            .get_webview_window("wpe")
            .ok_or_else(|| "WPE window is not initialized.".to_string())?;

        let is_visible = window.is_visible().unwrap_or(false);
        let new_state = !is_visible;

        if new_state {
            window.show().map_err(|e| format!("Failed to show WPE window: {:?}", e))?;
            let _ = window.set_focus();
            set_wpe_window_state(true);
        } else {
            window.hide().map_err(|e| format!("Failed to hide WPE window: {:?}", e))?;
            set_wpe_window_state(false);
        }

        Ok(new_state)
    })
}
