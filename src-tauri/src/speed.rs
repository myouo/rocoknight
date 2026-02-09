//! Speed-hack support: shared memory for the speed multiplier and DLL injection.

#[cfg(target_os = "windows")]
mod win {
    use std::ffi::c_void;
    use std::path::Path;

    use tracing::{info, warn};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
        MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
    };
    use windows::Win32::System::Memory::{
        VirtualAllocEx, VirtualFreeEx, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
    };
    use windows::Win32::System::Threading::{
        CreateRemoteThread, OpenProcess, WaitForSingleObject, PROCESS_CREATE_THREAD,
        PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
    };

    #[repr(C)]
    pub struct SpeedConfig {
        pub multiplier: f64,
        pub enabled: u32,
        pub _pad: [u8; 52],
    }

    const SHMEM_NAME: &str = "rocoknight-speed";

    pub struct SpeedShmem {
        handle: HANDLE,
        ptr: *mut SpeedConfig,
    }

    unsafe impl Send for SpeedShmem {}
    unsafe impl Sync for SpeedShmem {}

    impl SpeedShmem {
        pub fn create() -> Result<Self, String> {
            let name_wide: Vec<u16> = SHMEM_NAME
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            unsafe {
                let handle = CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    None,
                    PAGE_READWRITE,
                    0,
                    std::mem::size_of::<SpeedConfig>() as u32,
                    PCWSTR(name_wide.as_ptr()),
                )
                .map_err(|e| format!("CreateFileMappingW failed: {e}"))?;
                let view = MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, 0);
                if view.Value.is_null() {
                    let _ = CloseHandle(handle);
                    return Err("MapViewOfFile returned null".into());
                }
                let ptr = view.Value as *mut SpeedConfig;
                (*ptr).multiplier = 1.0;
                (*ptr).enabled = 1;
                info!("[speed] shared memory created");
                Ok(Self { handle, ptr })
            }
        }

        pub fn set_multiplier(&self, multiplier: f64) {
            unsafe {
                (*self.ptr).multiplier = multiplier;
            }
        }

        pub fn get_multiplier(&self) -> f64 {
            unsafe { (*self.ptr).multiplier }
        }
    }

    impl Drop for SpeedShmem {
        fn drop(&mut self) {
            unsafe {
                if !self.ptr.is_null() {
                    let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: self.ptr as *mut c_void,
                    });
                    self.ptr = std::ptr::null_mut();
                }
                if !self.handle.is_invalid() {
                    let _ = CloseHandle(self.handle);
                }
            }
        }
    }

    pub fn inject_dll(pid: u32, dll_path: &Path) -> Result<(), String> {
        let dll_path_str = dll_path
            .to_str()
            .ok_or_else(|| "DLL path not UTF-8".to_string())?;
        let dll_wide: Vec<u16> = dll_path_str
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let dll_bytes = dll_wide.len() * std::mem::size_of::<u16>();
        info!("[speed] injecting DLL into pid {}", pid);
        unsafe {
            let process = OpenProcess(
                PROCESS_CREATE_THREAD
                    | PROCESS_VM_OPERATION
                    | PROCESS_VM_WRITE
                    | PROCESS_VM_READ
                    | PROCESS_QUERY_INFORMATION,
                false,
                pid,
            )
            .map_err(|e| format!("OpenProcess: {e}"))?;

            let remote_mem = VirtualAllocEx(
                process,
                None,
                dll_bytes,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_EXECUTE_READWRITE,
            );
            if remote_mem.is_null() {
                let _ = CloseHandle(process);
                return Err("VirtualAllocEx failed".into());
            }

            let mut written: usize = 0;
            let ok = WriteProcessMemory(
                process,
                remote_mem,
                dll_wide.as_ptr() as *const c_void,
                dll_bytes,
                Some(&mut written),
            );
            if ok.is_err() || written != dll_bytes {
                let _ = VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
                let _ = CloseHandle(process);
                return Err("WriteProcessMemory failed".into());
            }

            let kernel32 = GetModuleHandleW(windows::core::w!("kernel32.dll"))
                .map_err(|e| format!("GetModuleHandleW: {e}"))?;
            let load_library_addr = GetProcAddress(kernel32, windows::core::s!("LoadLibraryW"))
                .ok_or_else(|| "GetProcAddress null".to_string())?;

            let thread = CreateRemoteThread(
                process,
                None,
                0,
                Some(std::mem::transmute(load_library_addr)),
                Some(remote_mem),
                0,
                None,
            )
            .map_err(|e| format!("CreateRemoteThread: {e}"))?;

            let wait = WaitForSingleObject(thread, 10_000);
            if wait != WAIT_OBJECT_0 {
                warn!("[speed] remote thread timeout");
            }

            let _ = VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
            let _ = CloseHandle(thread);
            let _ = CloseHandle(process);
        }
        info!("[speed] DLL injected successfully");
        Ok(())
    }

    pub fn resolve_speed_dll(
        app: &tauri::AppHandle,
        is_32bit: bool,
    ) -> Result<std::path::PathBuf, String> {
        use tauri::path::BaseDirectory;
        use tauri::Manager;
        let filename = if is_32bit {
            "speed_hook_32.dll"
        } else {
            "speed_hook_64.dll"
        };
        if let Ok(resolved) = app
            .path()
            .resolve(&format!("resources/{}", filename), BaseDirectory::Resource)
        {
            if std::fs::metadata(&resolved).is_ok() {
                info!("[speed] DLL resolved: {}", resolved.display());
                return Ok(resolved);
            }
        }
        Err(format!("Could not locate {}", filename))
    }

    pub fn is_process_32bit(pid: u32) -> Result<bool, String> {
        use windows::Win32::System::Threading::IsWow64Process;
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid)
                .map_err(|e| format!("OpenProcess: {e}"))?;
            let mut is_wow64 = windows::core::BOOL::default();
            IsWow64Process(process, &mut is_wow64).map_err(|e| format!("IsWow64Process: {e}"))?;
            let _ = CloseHandle(process);
            if cfg!(target_pointer_width = "64") {
                Ok(is_wow64.as_bool())
            } else {
                Ok(true)
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub use win::*;

#[cfg(not(target_os = "windows"))]
mod non_win {
    use std::path::Path;
    pub struct SpeedShmem;
    impl SpeedShmem {
        pub fn create() -> Result<Self, String> {
            Err("Windows only".into())
        }
        pub fn set_multiplier(&self, _m: f64) {}
        pub fn get_multiplier(&self) -> f64 {
            1.0
        }
    }
    pub fn inject_dll(_pid: u32, _dll_path: &Path) -> Result<(), String> {
        Err("Windows only".into())
    }
    pub fn resolve_speed_dll(
        _app: &tauri::AppHandle,
        _is_32bit: bool,
    ) -> Result<std::path::PathBuf, String> {
        Err("Windows only".into())
    }
    pub fn is_process_32bit(_pid: u32) -> Result<bool, String> {
        Ok(true)
    }
}

#[cfg(not(target_os = "windows"))]
pub use non_win::*;
