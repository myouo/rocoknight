use lazy_static::lazy_static;
use retour::GenericDetour;
use serde::Serialize;
use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use windows::core::{s, PCSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Networking::WinSock::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::SystemServices::{DLL_PROCESS_ATTACH, DLL_PROCESS_DETACH};

#[derive(Serialize)]
struct WpePacket {
    direction: &'static str,
    data_hex: String,
    len: usize,
    timestamp: u64,
}

lazy_static! {
    static ref PIPE_SENDER: Mutex<Option<std::fs::File>> = Mutex::new(None);
}

fn send_to_pipe(direction: &'static str, buf: *const u8, len: usize) {
    if len == 0 || buf.is_null() {
        return;
    }

    let data = unsafe { std::slice::from_raw_parts(buf, len) };
    let packet = WpePacket {
        direction,
        data_hex: hex::encode_upper(data),
        len,
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    };

    if let Ok(json) = serde_json::to_string(&packet) {
        let mut sender = PIPE_SENDER.lock().unwrap();
        if let Some(file) = sender.as_mut() {
            let _ = writeln!(file, "{}", json);
        }
    }
}

type SendFn = unsafe extern "system" fn(SOCKET, *const i8, i32, i32) -> i32;
type RecvFn = unsafe extern "system" fn(SOCKET, *mut i8, i32, i32) -> i32;
type SendToFn = unsafe extern "system" fn(SOCKET, *const i8, i32, i32, *const SOCKADDR, i32) -> i32;
type RecvFromFn = unsafe extern "system" fn(SOCKET, *mut i8, i32, i32, *mut SOCKADDR, *mut i32) -> i32;
type WSASendFn = unsafe extern "system" fn(SOCKET, *const WSABUF, u32, *mut u32, u32, *mut c_void, *mut c_void) -> i32;
type WSARecvFn = unsafe extern "system" fn(SOCKET, *const WSABUF, u32, *mut u32, *mut u32, *mut c_void, *mut c_void) -> i32;

static mut SEND_HOOK: Option<GenericDetour<SendFn>> = None;
static mut RECV_HOOK: Option<GenericDetour<RecvFn>> = None;
static mut SENDTO_HOOK: Option<GenericDetour<SendToFn>> = None;
static mut RECVFROM_HOOK: Option<GenericDetour<RecvFromFn>> = None;
static mut WSASEND_HOOK: Option<GenericDetour<WSASendFn>> = None;
static mut WSARECV_HOOK: Option<GenericDetour<WSARecvFn>> = None;

unsafe extern "system" fn hooked_send(s: SOCKET, buf: *const i8, len: i32, flags: i32) -> i32 {
    let ret = SEND_HOOK.as_ref().unwrap().call(s, buf, len, flags);
    if ret > 0 {
        send_to_pipe("Send", buf as *const u8, ret as usize);
    }
    ret
}

unsafe extern "system" fn hooked_recv(s: SOCKET, buf: *mut i8, len: i32, flags: i32) -> i32 {
    let ret = RECV_HOOK.as_ref().unwrap().call(s, buf, len, flags);
    if ret > 0 {
        send_to_pipe("Recv", buf as *mut u8, ret as usize);
    }
    ret
}

unsafe extern "system" fn hooked_sendto(
    s: SOCKET,
    buf: *const i8,
    len: i32,
    flags: i32,
    to: *const SOCKADDR,
    tolen: i32,
) -> i32 {
    let ret = SENDTO_HOOK.as_ref().unwrap().call(s, buf, len, flags, to, tolen);
    if ret > 0 {
        send_to_pipe("SendTo", buf as *const u8, ret as usize);
    }
    ret
}

unsafe extern "system" fn hooked_recvfrom(
    s: SOCKET,
    buf: *mut i8,
    len: i32,
    flags: i32,
    from: *mut SOCKADDR,
    fromlen: *mut i32,
) -> i32 {
    let ret = RECVFROM_HOOK.as_ref().unwrap().call(s, buf, len, flags, from, fromlen);
    if ret > 0 {
        send_to_pipe("RecvFrom", buf as *mut u8, ret as usize);
    }
    ret
}

unsafe extern "system" fn hooked_wsasend(
    s: SOCKET,
    buffers: *const WSABUF,
    buffer_count: u32,
    bytes_sent: *mut u32,
    flags: u32,
    overlapped: *mut c_void,
    completion_routine: *mut c_void,
) -> i32 {
    let ret = WSASEND_HOOK.as_ref().unwrap().call(
        s,
        buffers,
        buffer_count,
        bytes_sent,
        flags,
        overlapped,
        completion_routine,
    );
    if ret == 0 && !bytes_sent.is_null() && !buffers.is_null() {
        let sent = *bytes_sent;
        if sent > 0 {
            let buf = &*buffers;
            send_to_pipe("WSASend", buf.buf.as_ptr() as *const u8, sent as usize);
        }
    }
    ret
}

unsafe extern "system" fn hooked_wsarecv(
    s: SOCKET,
    buffers: *const WSABUF,
    buffer_count: u32,
    bytes_recvd: *mut u32,
    flags: *mut u32,
    overlapped: *mut c_void,
    completion_routine: *mut c_void,
) -> i32 {
    let ret = WSARECV_HOOK.as_ref().unwrap().call(
        s,
        buffers,
        buffer_count,
        bytes_recvd,
        flags,
        overlapped,
        completion_routine,
    );
    if ret == 0 && !bytes_recvd.is_null() && !buffers.is_null() {
        let recvd = *bytes_recvd;
        if recvd > 0 {
            let buf = &*buffers;
            send_to_pipe("WSARecv", buf.buf.as_ptr() as *const u8, recvd as usize);
        }
    }
    ret
}

fn init_hooks() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let module = GetModuleHandleA(s!("ws2_32.dll")).map_err(|e| e.to_string())?;

        if let Some(addr) = GetProcAddress(module, s!("send")) {
            let target: SendFn = std::mem::transmute(addr);
            let hook = GenericDetour::new(target, hooked_send)?;
            hook.enable()?;
            SEND_HOOK = Some(hook);
        }
        if let Some(addr) = GetProcAddress(module, s!("recv")) {
            let target: RecvFn = std::mem::transmute(addr);
            let hook = GenericDetour::new(target, hooked_recv)?;
            hook.enable()?;
            RECV_HOOK = Some(hook);
        }
        if let Some(addr) = GetProcAddress(module, s!("sendto")) {
            let target: SendToFn = std::mem::transmute(addr);
            let hook = GenericDetour::new(target, hooked_sendto)?;
            hook.enable()?;
            SENDTO_HOOK = Some(hook);
        }
        if let Some(addr) = GetProcAddress(module, s!("recvfrom")) {
            let target: RecvFromFn = std::mem::transmute(addr);
            let hook = GenericDetour::new(target, hooked_recvfrom)?;
            hook.enable()?;
            RECVFROM_HOOK = Some(hook);
        }
        if let Some(addr) = GetProcAddress(module, s!("WSASend")) {
            let target: WSASendFn = std::mem::transmute(addr);
            let hook = GenericDetour::new(target, hooked_wsasend)?;
            hook.enable()?;
            WSASEND_HOOK = Some(hook);
        }
        if let Some(addr) = GetProcAddress(module, s!("WSARecv")) {
            let target: WSARecvFn = std::mem::transmute(addr);
            let hook = GenericDetour::new(target, hooked_wsarecv)?;
            hook.enable()?;
            WSARECV_HOOK = Some(hook);
        }
    }
    Ok(())
}

fn open_pipe() {
    let pid = std::process::id();
    let pipe_name = format!(r"\\.\pipe\RococnightWPE_{}", pid);
    
    // Attempt to connect to the named pipe
    if let Ok(file) = OpenOptions::new().write(true).open(&pipe_name) {
        *PIPE_SENDER.lock().unwrap() = Some(file);
    }
}

#[no_mangle]
#[allow(non_snake_case)]
extern "system" fn DllMain(_module: HINSTANCE, call_reason: u32, _reserved: *mut c_void) -> BOOL {
    match call_reason {
        DLL_PROCESS_ATTACH => {
            std::thread::spawn(|| {
                open_pipe();
                let _ = init_hooks();
            });
            BOOL(1)
        }
        DLL_PROCESS_DETACH => {
            BOOL(1)
        }
        _ => BOOL(1),
    }
}
