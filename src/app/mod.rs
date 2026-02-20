mod config;
mod error;
mod logging;
mod runtime;

pub use error::AppError;

pub fn run() -> Result<(), AppError> {
    logging::init()?;

    let config = config::AppConfig::from_env()?;

    tracing::info!(
        keba_ip = %config.keba_ip,
        keba_source = ?config.keba_source,
        keba_udp_port = config.keba_udp_port,
        keba_modbus_port = config.keba_modbus_port,
        keba_modbus_unit_id = config.keba_modbus_unit_id,
        keba_modbus_energy_factor_wh = config.keba_modbus_energy_factor_wh,
        poll_interval_ms = config.poll_interval_ms,
        db_path = %config.db_path,
        http_bind = %config.http_bind,
        debounce_samples = config.debounce_samples,
        "application bootstrap initialized"
    );

    runtime::run(config)
}
