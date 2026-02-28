mod config;
mod error;
mod logging;
mod runtime;
pub mod services;

pub use error::AppError;

pub fn run() -> Result<(), AppError> {
    run_combined()
}

pub fn run_combined() -> Result<(), AppError> {
    logging::init()?;
    let config = config::AppConfig::from_env()?;
    log_bootstrap("combined", &config);
    runtime::run_combined(config)
}

pub fn run_service() -> Result<(), AppError> {
    logging::init()?;
    let config = config::AppConfig::from_env()?;
    log_bootstrap("service", &config);
    runtime::run_service(config)
}

pub fn run_api() -> Result<(), AppError> {
    logging::init()?;
    let config = config::AppConfig::from_env_for_api()?;
    log_bootstrap("api", &config);
    runtime::run_api(config)
}

fn log_bootstrap(mode: &str, config: &config::AppConfig) {
    tracing::info!(
        run_mode = mode,
        keba_ip = %config.keba_ip,
        keba_source = ?config.keba_source,
        keba_udp_port = config.keba_udp_port,
        keba_modbus_port = config.keba_modbus_port,
        keba_modbus_unit_id = config.keba_modbus_unit_id,
        keba_modbus_energy_factor_wh = config.keba_modbus_energy_factor_wh,
        keba_debug_data_file = ?config.keba_debug_data_file,
        results_output_file = ?config.results_output_file,
        poll_interval_ms = config.poll_interval_ms,
        db_path = %config.db_path,
        http_bind = %config.http_bind,
        debounce_samples = config.debounce_samples,
        status_log_interval_seconds = config.status_log_interval_seconds,
        status_station_count = config.status_stations.len(),
        "application bootstrap initialized"
    );
}
