use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{error, info, warn};

use crate::wpe::windivert::WinDivertHandle;
use crate::wpe::{GamePacket, PacketAction, PacketHandler, WpeError};

pub struct PacketInterceptor {
    pid: u32,
    running: Arc<AtomicBool>,
    handlers: Arc<Mutex<Vec<Arc<dyn PacketHandler>>>>,
}

impl PacketInterceptor {
    pub fn new(pid: u32) -> Result<Arc<Self>, WpeError> {
        info!("[WPE] Creating packet interceptor for PID {}", pid);

        let interceptor = Arc::new(Self {
            pid,
            running: Arc::new(AtomicBool::new(true)),
            handlers: Arc::new(Mutex::new(Vec::new())),
        });

        let interceptor_clone = interceptor.clone();
        thread::spawn(move || {
            if let Err(e) = interceptor_clone.run() {
                error!("[WPE] Interceptor thread error: {}", e);
            }
        });

        Ok(interceptor)
    }

    pub fn register_handler(&self, handler: Arc<dyn PacketHandler>) {
        let mut handlers = self.handlers.lock().expect("handlers lock");
        handlers.push(handler);
        info!("[WPE] Registered packet handler");
    }

    pub fn stop(&self) {
        info!("[WPE] Stopping packet interceptor");
        self.running.store(false, Ordering::Relaxed);
    }

    fn run(&self) -> Result<(), WpeError> {
        info!("[WPE] Interceptor thread started for PID {}", self.pid);

        let handle = WinDivertHandle::open(self.pid)?;

        while self.running.load(Ordering::Relaxed) {
            match handle.recv() {
                Ok(data) => {
                    if let Err(e) = self.process_packet(&data) {
                        warn!("[WPE] Failed to process packet: {}", e);
                    }
                }
                Err(WpeError::NotRunning) => {
                    break;
                }
                Err(e) => {
                    warn!("[WPE] Recv error: {}", e);
                    thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }

        info!("[WPE] Interceptor thread stopped");
        Ok(())
    }

    fn process_packet(&self, data: &[u8]) -> Result<(), WpeError> {
        let packet = GamePacket::parse(data)?;

        let handlers = self.handlers.lock().expect("handlers lock");
        for handler in handlers.iter() {
            match handler.handle_outbound(&packet) {
                PacketAction::Forward => continue,
                PacketAction::Modified(modified) => {
                    info!("[WPE] Packet modified by handler");
                    return Ok(());
                }
                PacketAction::Drop => {
                    info!("[WPE] Packet dropped by handler");
                    return Ok(());
                }
                PacketAction::Inject(inject) => {
                    info!("[WPE] Handler requested packet injection");
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

impl Drop for PacketInterceptor {
    fn drop(&mut self) {
        self.stop();
    }
}
