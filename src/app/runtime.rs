use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::JoinHandle;
use std::time::Duration;

use actix_web::{App, HttpServer, web};
use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde_json::Value;
use thiserror::Error;

use crate::adapters::api::{ApiState, Report100Station, configure_routes};
use crate::adapters::keba_debug_file::KebaDebugFileClient;
use crate::adapters::keba_modbus::KebaModbusClient;
use crate::adapters::keba_udp::{KebaClient, KebaClientError, KebaUdpClient};
use crate::app::config::{AppConfig, KebaSource};
use crate::app::error::AppError;
use crate::app::services::{SessionCommandHandler, SqliteSessionService};
use crate::domain::keba_payload::{ParseError, parse_report2};
use crate::domain::models::NewUnplugLogRecord;
use crate::domain::session_state::{SessionStateMachine, SessionTransition, TimestampMs};

#[derive(Debug, Error)]
pub enum PollerError {
    #[error("failed to fetch report 2: {0}")]
    FetchReport2(#[source] KebaClientError),
    #[error("failed to parse report 2: {0}")]
    ParseReport2(#[source] ParseError),
}

pub struct PlugStatusPoller {
    client: Box<dyn KebaClient>,
    session_commands: SqliteSessionService,
    station_label: String,
    state_machine: SessionStateMachine,
    consecutive_poll_errors: u32,
}

impl PlugStatusPoller {
    pub fn new(
        client: Box<dyn KebaClient>,
        session_commands: SqliteSessionService,
        station_label: String,
        debounce_samples: usize,
    ) -> Self {
        Self {
            client,
            session_commands,
            station_label,
            state_machine: SessionStateMachine::new(debounce_samples),
            consecutive_poll_errors: 0,
        }
    }

    pub fn tick(&mut self) -> Result<(), PollerError> {
        let report2_raw = self.client.get_report2().map_err(PollerError::FetchReport2)?;
        let report2 = parse_report2(&report2_raw).map_err(PollerError::ParseReport2)?;
        if self.consecutive_poll_errors > 0 {
            tracing::info!(
                consecutive_errors = self.consecutive_poll_errors,
                "poller recovered after consecutive errors"
            );
            self.consecutive_poll_errors = 0;
        }

        let observed_at = TimestampMs(Utc::now().timestamp_millis());
        if let Some(transition) = self.state_machine.observe_at(report2.plugged, observed_at) {
            match transition {
                SessionTransition::Plugged { .. } => self.log_status_change(true),
                SessionTransition::Unplugged { .. } => self.log_status_change(false),
            }
        }

        Ok(())
    }

    pub fn note_poll_error(&mut self, error: &PollerError) {
        self.consecutive_poll_errors += 1;
        if self.consecutive_poll_errors == 1 {
            tracing::warn!(error = %error, "poller tick failed");
            return;
        }
        if self.consecutive_poll_errors.is_multiple_of(10) {
            tracing::warn!(
                error = %error,
                consecutive_errors = self.consecutive_poll_errors,
                "poller tick still failing"
            );
        }
    }

    fn log_status_change(&self, plugged: bool) {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let status = if plugged { "Angesteckt" } else { "Abgesteckt" };

        if plugged {
            tracing::info!(
                "Zeitstempel: {} | Ladestation: {} | Status: {}",
                timestamp,
                self.station_label,
                status
            );
            return;
        }

        let disconnected_at_ms = Utc::now().timestamp_millis();
        let details = self.fetch_unplug_details(disconnected_at_ms);
        tracing::info!(
            "Zeitstempel: {} | Ladestation: {} | Status: {} | Start: {} | Ende: {} | kWh: {} | CardId: {}",
            timestamp,
            self.station_label,
            status,
            details.started,
            details.ended,
            details.kwh,
            details.card_id
        );
        let event = NewUnplugLogRecord {
            timestamp,
            station: self.station_label.clone(),
            started: details.started,
            ended: details.ended,
            kwh: details.kwh,
            card_id: details.card_id,
        };
        if let Err(error) = self.session_commands.insert_unplug_log_event(&event) {
            tracing::warn!(error = %error, "failed to persist unplug log event");
        }
    }

    fn fetch_unplug_details(&self, disconnected_at_ms: i64) -> UnplugLogDetails {
        let report100 = match self.client.get_report100() {
            Ok(payload) => payload,
            Err(_) => return UnplugLogDetails::na(),
        };
        let object100 = match report100.as_object() {
            Some(object) => object,
            None => return UnplugLogDetails::na(),
        };
        let ended_100 = find_value(object100, &["ended", "Ended"]).and_then(parse_f64);
        let selected_payload = if matches!(
            ended_100,
            Some(0.0)
        ) {
            match self.client.get_report101() {
                Ok(payload) => payload,
                Err(_) => return UnplugLogDetails::na(),
            }
        } else {
            report100
        };
        let selected = match selected_payload.as_object() {
            Some(object) => object,
            None => return UnplugLogDetails::na(),
        };

        let kwh = find_value(
            selected,
            &["E Pres", "E pres", "Energy Session", "energy_present_session"],
        )
        .and_then(parse_f64)
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "n/a".to_string());
        let card_id = find_value(
            selected,
            &[
                "RFID",
                "RFID tag",
                "RFID Tag",
                "CardId",
                "Card ID",
                "card_id",
            ],
        )
        .map(stringify_value)
        .unwrap_or_else(|| "n/a".to_string());

        let started = parse_session_timestamp_ms_from_object(
            selected,
            &["started", "Started", "start", "session_start", "Session Start"],
            disconnected_at_ms,
        )
        .map(format_ts)
        .unwrap_or_else(|| "n/a".to_string());
        let ended = parse_session_timestamp_ms_from_object(
            selected,
            &["ended", "Ended", "end", "session_end", "Session End"],
            disconnected_at_ms,
        )
        .map(format_ts)
        .unwrap_or_else(|| "n/a".to_string());

        UnplugLogDetails {
            started,
            ended,
            kwh,
            card_id,
        }
    }
}

struct UnplugLogDetails {
    started: String,
    ended: String,
    kwh: String,
    card_id: String,
}

impl UnplugLogDetails {
    fn na() -> Self {
        Self {
            started: "n/a".to_string(),
            ended: "n/a".to_string(),
            kwh: "n/a".to_string(),
            card_id: "n/a".to_string(),
        }
    }
}

fn format_ts(value_ms: i64) -> String {
    match Utc.timestamp_millis_opt(value_ms).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "n/a".to_string(),
    }
}

fn find_value<'a>(object: &'a serde_json::Map<String, Value>, aliases: &[&str]) -> Option<&'a Value> {
    aliases.iter().find_map(|alias| object.get(*alias))
}

fn parse_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().replace(',', ".").parse::<f64>().ok(),
        _ => None,
    }
}

fn stringify_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn parse_absolute_timestamp_ms(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => {
            if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(text) {
                return Some(parsed.timestamp_millis());
            }
            if let Ok(parsed) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.3f") {
                return Some(Utc.from_utc_datetime(&parsed).timestamp_millis());
            }
            text.trim().parse::<i64>().ok()
        }
        _ => None,
    }
}

fn parse_session_timestamp_ms_from_object(
    object: &serde_json::Map<String, Value>,
    aliases: &[&str],
    now_ms: i64,
) -> Option<i64> {
    let sec_from_report = find_value(object, &["Sec", "sec", "Seconds", "seconds"]).and_then(parse_f64);
    let value = find_value(object, aliases)?;
    if let Some(sec_now) = sec_from_report
        && let Some(raw_seconds) = parse_f64(value)
        && (0.0..1_000_000_000_000.0).contains(&raw_seconds)
    {
        let ts = (now_ms as f64) - ((sec_now - raw_seconds) * 1000.0);
        return Some(ts.round() as i64);
    }

    parse_absolute_timestamp_ms(value)
}

fn start_poller(
    mut poller: PlugStatusPoller,
    poll_interval: Duration,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            if let Err(error) = poller.tick() {
                poller.note_poll_error(&error);
            }
            std::thread::sleep(poll_interval);
        }
    })
}

pub fn run_combined(config: AppConfig) -> Result<(), AppError> {
    let shared_connection = open_shared_connection_writer(&config.db_path)?;
    let session_service = SqliteSessionService::new(Arc::clone(&shared_connection));
    let api_state = ApiState {
        session_queries: session_service.clone(),
        report100_stations: build_report100_stations(&config),
    };

    let poller = build_poller(&config, session_service.clone())?;
    let stop_flag = Arc::new(AtomicBool::new(false));
    let poller_handle = start_poller(
        poller,
        Duration::from_millis(config.poll_interval_ms),
        Arc::clone(&stop_flag),
    );

    let server_result = run_http_server(&config.http_bind, api_state);
    stop_flag.store(true, Ordering::Relaxed);
    let join_result = poller_handle.join();
    if join_result.is_err() {
        return Err(AppError::runtime("poller thread panicked"));
    }

    server_result
}

pub fn run_service(config: AppConfig) -> Result<(), AppError> {
    let shared_connection = open_shared_connection_writer(&config.db_path)?;
    let session_service = SqliteSessionService::new(Arc::clone(&shared_connection));
    let mut poller = build_poller(&config, session_service)?;

    if config.keba_source == KebaSource::DebugFile {
        return run_debug_replay_loop(&mut poller, config.poll_interval_ms);
    }

    loop {
        if let Err(error) = poller.tick() {
            poller.note_poll_error(&error);
        }
        std::thread::sleep(Duration::from_millis(config.poll_interval_ms));
    }
}

pub fn run_api(config: AppConfig) -> Result<(), AppError> {
    let shared_connection = open_shared_connection_reader(&config.db_path)?;
    let session_service = crate::app::services::SqliteSessionService::new(Arc::clone(&shared_connection));
    let api_state = ApiState {
        session_queries: session_service,
        report100_stations: build_report100_stations(&config),
    };

    run_http_server(&config.http_bind, api_state)
}

fn open_shared_connection_writer(db_path: &str) -> Result<Arc<Mutex<Connection>>, AppError> {
    let mut connection =
        crate::adapters::db::open_connection(db_path).map_err(AppError::database_init)?;
    crate::adapters::db::run_migrations(&mut connection).map_err(AppError::database_init)?;
    Ok(Arc::new(Mutex::new(connection)))
}

fn open_shared_connection_reader(db_path: &str) -> Result<Arc<Mutex<Connection>>, AppError> {
    let connection =
        crate::adapters::db::open_read_only_connection(db_path).map_err(AppError::database_init)?;
    let version =
        crate::adapters::db::schema_version(&connection).map_err(AppError::database_init)?;
    if version == 0 {
        return Err(AppError::database_init(
            "database schema is not initialized; start writer service first",
        ));
    }
    Ok(Arc::new(Mutex::new(connection)))
}

fn build_poller(
    config: &AppConfig,
    session_service: SqliteSessionService,
) -> Result<PlugStatusPoller, AppError> {
    let keba_client = build_keba_client(config)?;
    Ok(PlugStatusPoller::new(
        keba_client,
        session_service,
        station_label(config.station_id.as_deref()),
        config.debounce_samples,
    ))
}

fn station_label(station_id: Option<&str>) -> String {
    let Some(raw) = station_id else {
        return "Unbekannt".to_string();
    };
    let normalized = raw
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();

    if normalized.contains("carport") {
        "Carport".to_string()
    } else if normalized.contains("eingang") || normalized.contains("entrance") {
        "Eingang".to_string()
    } else {
        raw.to_string()
    }
}

fn build_report100_stations(config: &AppConfig) -> Vec<Report100Station> {
    let mut mapped = Vec::new();
    for station in &config.status_stations {
        let normalized = station
            .name
            .chars()
            .filter(|char| char.is_ascii_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>();
        let logical_name = if normalized.contains("carport") {
            Some("carport")
        } else if normalized.contains("entrance") || normalized.contains("eingang") {
            Some("entrance")
        } else {
            None
        };
        if let Some(logical_name) = logical_name
            && !mapped.iter().any(|entry: &Report100Station| entry.logical_name == logical_name)
        {
            mapped.push(Report100Station {
                logical_name: logical_name.to_string(),
                ip: station.ip.clone(),
                port: station.port,
            });
        }
    }
    mapped
}

fn build_keba_client(config: &AppConfig) -> Result<Box<dyn KebaClient>, AppError> {
    let keba_client: Box<dyn KebaClient> = match config.keba_source {
        KebaSource::Udp => Box::new(
            KebaUdpClient::new(&config.keba_ip, config.keba_udp_port).map_err(AppError::runtime)?,
        ),
        KebaSource::Modbus => Box::new(
            KebaModbusClient::new(
                &config.keba_ip,
                config.keba_modbus_port,
                config.keba_modbus_unit_id,
                config.keba_modbus_energy_factor_wh,
            )
            .map_err(AppError::runtime)?,
        ),
        KebaSource::DebugFile => Box::new(
            KebaDebugFileClient::from_file(
                config
                    .keba_debug_data_file
                    .as_deref()
                    .ok_or_else(|| AppError::config("KEBA_DEBUG_DATA_FILE is required"))?,
            )
            .map_err(AppError::runtime)?,
        ),
    };
    Ok(keba_client)
}

fn run_debug_replay_loop(poller: &mut PlugStatusPoller, poll_interval_ms: u64) -> Result<(), AppError> {
    loop {
        match poller.tick() {
            Ok(()) => std::thread::sleep(Duration::from_millis(poll_interval_ms)),
            Err(PollerError::FetchReport2(KebaClientError::Io(io)))
                if io.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                tracing::info!("debug replay finished");
                return Ok(());
            }
            Err(error) => {
                poller.note_poll_error(&error);
                std::thread::sleep(Duration::from_millis(poll_interval_ms));
            }
        }
    }
}

fn run_http_server(http_bind: &str, api_state: ApiState) -> Result<(), AppError> {
    tracing::info!(bind = %http_bind, "http server starting");
    let server_result = actix_web::rt::System::new().block_on(async move {
        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(api_state.clone()))
                .configure(configure_routes)
        })
        .bind(http_bind)?
        .run()
        .await
    });
    server_result.map_err(AppError::runtime)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::PlugStatusPoller;
    use crate::adapters::keba_udp::{KebaClient, KebaClientError};
    use crate::app::services::SqliteSessionService;
    use crate::test_support::open_test_connection;

    struct FakeKebaClient {
        report2_payloads: Mutex<VecDeque<serde_json::Value>>,
        report3_payloads: Mutex<VecDeque<serde_json::Value>>,
    }

    impl FakeKebaClient {
        fn new(report2_payloads: Vec<serde_json::Value>, report3_payloads: Vec<serde_json::Value>) -> Self {
            Self {
                report2_payloads: Mutex::new(VecDeque::from(report2_payloads)),
                report3_payloads: Mutex::new(VecDeque::from(report3_payloads)),
            }
        }
    }

    impl KebaClient for FakeKebaClient {
        fn get_report2(&self) -> Result<serde_json::Value, KebaClientError> {
            self.report2_payloads
                .lock()
                .expect("report2 queue lock should be available")
                .pop_front()
                .ok_or_else(|| {
                    KebaClientError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "no further report2 payloads",
                    ))
                })
        }

        fn get_report3(&self) -> Result<serde_json::Value, KebaClientError> {
            self.report3_payloads
                .lock()
                .expect("report3 queue lock should be available")
                .pop_front()
                .ok_or_else(|| {
                    KebaClientError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "no further report3 payloads",
                    ))
                })
        }

        fn get_report100(&self) -> Result<serde_json::Value, KebaClientError> {
            Err(KebaClientError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "not needed in test",
            )))
        }

        fn get_report101(&self) -> Result<serde_json::Value, KebaClientError> {
            Err(KebaClientError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "not needed in test",
            )))
        }
    }

    #[test]
    fn unplug_transition_persists_unplug_event_only() {
        let connection = Arc::new(Mutex::new(open_test_connection("runtime-session-persist")));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));

        let fake_client = FakeKebaClient::new(
            vec![
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 1}),
                json!({"Plug": 1}),
                json!({"Plug": 1}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
            ],
            vec![
                json!({"Energy (present session)": 0.0, "Energy (total)": 10.0}),
                json!({"Energy (present session)": 7.2, "Energy (total)": 17.2}),
            ],
        );

        let mut poller = PlugStatusPoller::new(
            Box::new(fake_client),
            session_service.clone(),
            "Carport".to_string(),
            3,
        );

        for _ in 0..8 {
            poller.tick().expect("pre-unplug ticks should succeed");
        }
        {
            let db = connection.lock().expect("connection lock should be available");
            let unplug_count: i64 = db
                .query_row("SELECT COUNT(*) FROM unplug_log_events", [], |row| row.get(0))
                .expect("unplug count query should succeed");
            assert_eq!(unplug_count, 0);
        }
        poller.tick().expect("debounced unplug tick should succeed");

        let db = connection.lock().expect("connection lock should be available");
        let unplug_count: i64 = db
            .query_row("SELECT COUNT(*) FROM unplug_log_events", [], |row| row.get(0))
            .expect("unplug count query should succeed");
        let session_count: i64 = db
            .query_row("SELECT COUNT(*) FROM charging_sessions", [], |row| row.get(0))
            .expect("session count query should succeed");

        assert_eq!(unplug_count, 1);
        assert_eq!(session_count, 0);
    }

    #[test]
    fn flapping_state_does_not_persist_unplug_event() {
        let connection = Arc::new(Mutex::new(open_test_connection("runtime-session-flap")));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));

        let fake_client = FakeKebaClient::new(
            vec![
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 1}),
                json!({"Plug": 0}),
                json!({"Plug": 1}),
                json!({"Plug": 0}),
                json!({"Plug": 1}),
                json!({"Plug": 0}),
            ],
            vec![],
        );

        let mut poller = PlugStatusPoller::new(
            Box::new(fake_client),
            session_service,
            "Carport".to_string(),
            3,
        );

        for _ in 0..9 {
            poller.tick().expect("flapping tick should succeed");
        }

        let db = connection.lock().expect("connection lock should be available");
        let unplug_count: i64 = db
            .query_row("SELECT COUNT(*) FROM unplug_log_events", [], |row| row.get(0))
            .expect("unplug count query should succeed");
        assert_eq!(unplug_count, 0);
    }
}

