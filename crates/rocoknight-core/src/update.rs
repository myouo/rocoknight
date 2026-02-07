use crate::error::{CoreError, CoreResult};

pub async fn check_for_updates(_endpoint: &str) -> CoreResult<()> {
    Err(CoreError::Network("update not implemented".to_string()))
}

