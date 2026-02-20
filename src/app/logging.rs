use tracing_subscriber::{EnvFilter, fmt};

use crate::app::AppError;

pub fn init() -> Result<(), AppError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init()
        .map_err(AppError::logging_init)
}
