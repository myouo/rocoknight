use std::sync::Arc;
use tracing::info;

use crate::wpe::{GamePacket, WpeError};
use crate::wpe::windivert::WinDivertHandle;

pub struct PacketInjector {
    handle: Arc<WinDivertHandle>,
}

impl PacketInjector {
    pub fn new(pid: u32) -> Result<Self, WpeError> {
        info!("[WPE] Creating packet injector for PID {}", pid);
        let handle = WinDivertHandle::open(pid)?;
        Ok(Self {
            handle: Arc::new(handle),
        })
    }

    pub fn inject(&self, packet: GamePacket) -> Result<(), WpeError> {
        let data = packet.build()?;
        info!("[WPE] Injecting packet: {} bytes", data.len());
        self.handle.send(&data)?;
        Ok(())
    }
}
