use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("config error: {0}")]
    Config(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("unsupported platform")]
    UnsupportedPlatform,
}

