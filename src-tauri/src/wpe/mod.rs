pub mod injector;
pub mod interceptor;
pub mod packet;
pub mod windivert;

pub use injector::PacketInjector;
pub use interceptor::PacketInterceptor;
pub use packet::{GamePacket, PacketAction, PacketHandler};

#[derive(Debug, thiserror::Error)]
pub enum WpeError {
    #[error("WinDivert error: {0}")]
    WinDivert(String),

    #[error("Packet parse error: {0}")]
    PacketParse(String),

    #[error("Packet build error: {0}")]
    PacketBuild(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not running")]
    NotRunning,
}

pub type Result<T> = std::result::Result<T, WpeError>;
