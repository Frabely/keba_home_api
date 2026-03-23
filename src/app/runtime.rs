use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::http::{Method, header};
use actix_web::{App, HttpServer, web};
use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde_json::Value;
use thiserror::Error;

use crate::adapters::api::{ApiState, Report100Station, configure_routes};
use crate::adapters::keba_debug_file::KebaDebugFileClient;
use crate::adapters::keba_modbus::KebaModbusClient;
use crate::adapters::keba_udp::{KebaClient, KebaClientError, KebaUdpClient};
use crate::app::config::{AppConfig, CorsAllowedOrigins, KebaSource};
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

const POLLER_WARN_AFTER_CONSECUTIVE_ERRORS: u32 = 3;

#[cfg(not(test))]
const UNPLUG_DETAILS_RETRY_ATTEMPTS: usize = 6;
#[cfg(test)]
const UNPLUG_DETAILS_RETRY_ATTEMPTS: usize = 1;
const UNPLUG_DETAILS_RETRY_INTERVAL: Duration = Duration::from_secs(2);

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
        let report2_raw = self
            .client
            .get_report2()
            .map_err(PollerError::FetchReport2)?;
        let report2 = parse_report2(&report2_raw).map_err(PollerError::ParseReport2)?;
        if self.consecutive_poll_errors > 0 {
            tracing::info!(
                consecutive_errors = self.consecutive_poll_errors,
                "poller recovered after consecutive errors"
            );
            self.consecutive_poll_errors = 0;
        }

        let observed_at = TimestampMs(Utc::now().timestamp_millis());
        let stable_before = self.state_machine.stable_plugged();
        if let Some(transition) = self.state_machine.observe_at(report2.plugged, observed_at) {
            match transition {
                SessionTransition::Plugged { .. } => self.log_status_change(true),
                SessionTransition::Unplugged { .. } => self.log_status_change(false),
            }
        }
        if stable_before.is_none()
            && let Some(initial_plugged) = self.state_machine.stable_plugged()
        {
            tracing::info!(
                station = %self.station_label,
                plugged = initial_plugged,
                "initial plug state established"
            );
        }

        Ok(())
    }

    pub fn note_poll_error(&mut self, error: &PollerError) {
        self.consecutive_poll_errors += 1;
        if self.consecutive_poll_errors < POLLER_WARN_AFTER_CONSECUTIVE_ERRORS {
            tracing::debug!(
                error = %error,
                consecutive_errors = self.consecutive_poll_errors,
                "poller tick transient failure"
            );
            return;
        }
        if self.consecutive_poll_errors == POLLER_WARN_AFTER_CONSECUTIVE_ERRORS
            || self.consecutive_poll_errors.is_multiple_of(10)
        {
            tracing::warn!(
                error = %error,
                consecutive_errors = self.consecutive_poll_errors,
                "poller tick still failing"
            );
        }
    }

    fn log_status_change(&self, plugged: bool) {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M").to_string();
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
            "Zeitstempel: {} | Ladestation: {} | Status: {} | Start: {} | Ende: {} | Wh: {} | CardId: {}",
            timestamp,
            self.station_label,
            status,
            details.started,
            details.ended,
            details.wh,
            details.card_id
        );
        let event = NewUnplugLogRecord {
            timestamp,
            station: self.station_label.clone(),
            started: details.started,
            ended: details.ended,
            wh: details.wh,
            card_id: details.card_id,
        };
        if let Err(error) = self.session_commands.insert_unplug_log_event(&event) {
            tracing::warn!(error = %error, "failed to persist unplug log event");
        }
    }

    fn fetch_unplug_details(&self, disconnected_at_ms: i64) -> UnplugLogDetails {
        for attempt in 1..=UNPLUG_DETAILS_RETRY_ATTEMPTS {
            let details = self.fetch_unplug_details_once(disconnected_at_ms);
            if details.is_complete() {
                if attempt > 1 {
                    tracing::info!(attempt, "resolved unplug details after retry");
                }
                return details;
            }

            if attempt < UNPLUG_DETAILS_RETRY_ATTEMPTS {
                tracing::debug!(
                    attempt,
                    "unplug details incomplete, retrying report 1xx scan"
                );
                std::thread::sleep(UNPLUG_DETAILS_RETRY_INTERVAL);
            }
        }

        tracing::warn!(
            retry_attempts = UNPLUG_DETAILS_RETRY_ATTEMPTS,
            "unplug details remained incomplete after retries"
        );
        UnplugLogDetails::na()
    }

    fn fetch_unplug_details_once(&self, disconnected_at_ms: i64) -> UnplugLogDetails {
        const REPORT_SEARCH_START: u16 = 100;
        const REPORT_SEARCH_END: u16 = 130;

        for report_id in REPORT_SEARCH_START..=REPORT_SEARCH_END {
            let payload = match self.client.get_report(report_id) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::debug!(
                        report_id,
                        error = %error,
                        "failed to fetch report while resolving unplug details"
                    );
                    continue;
                }
            };

            let Some(object) = payload.as_object() else {
                tracing::debug!(report_id, "report payload is not a JSON object");
                continue;
            };

            let details = extract_unplug_details_from_report(object, disconnected_at_ms);
            if details.is_complete() {
                return details;
            }
        }

        UnplugLogDetails::na()
    }
}

struct UnplugLogDetails {
    started: String,
    ended: String,
    wh: String,
    card_id: String,
}

impl UnplugLogDetails {
    fn na() -> Self {
        Self {
            started: "n/a".to_string(),
            ended: "n/a".to_string(),
            wh: "n/a".to_string(),
            card_id: "n/a".to_string(),
        }
    }

    fn is_complete(&self) -> bool {
        self.started != "n/a" && self.ended != "n/a" && self.wh != "n/a"
    }
}

fn extract_unplug_details_from_report(
    report: &serde_json::Map<String, Value>,
    disconnected_at_ms: i64,
) -> UnplugLogDetails {
    let wh_value = parse_session_wh_from_report(report);

    let started_ms = parse_session_timestamp_ms_from_object(
        report,
        &[
            "started[s]",
            "Started[s]",
            "started",
            "Started",
            "start",
            "session_start",
            "Session Start",
        ],
        disconnected_at_ms,
    );
    let ended_ms = parse_session_timestamp_ms_from_object(
        report,
        &[
            "ended[s]",
            "Ended[s]",
            "ended",
            "Ended",
            "end",
            "session_end",
            "Session End",
        ],
        disconnected_at_ms,
    );

    let card_id = find_value(
        report,
        &[
            "RFID", "RFID tag", "RFID Tag", "CardId", "Card ID", "card_id",
        ],
    )
    .map(stringify_value)
    .unwrap_or_else(|| "n/a".to_string());

    if let (Some(started), Some(ended), Some(wh)) = (started_ms, ended_ms, wh_value)
        && started > 0
        && ended >= started
        && wh >= 0.0
    {
        return UnplugLogDetails {
            started: format_ts(started),
            ended: format_ts(ended),
            wh: format!("{wh:.1}"),
            card_id,
        };
    }

    UnplugLogDetails::na()
}

fn format_ts(value_ms: i64) -> String {
    match Utc.timestamp_millis_opt(value_ms).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "n/a".to_string(),
    }
}

fn find_value<'a>(
    object: &'a serde_json::Map<String, Value>,
    aliases: &[&str],
) -> Option<&'a Value> {
    aliases.iter().find_map(|alias| object.get(*alias))
}

fn parse_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_numeric_text(text),
        _ => None,
    }
}

fn parse_numeric_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.contains(',') && !trimmed.contains('.') {
        let mut parts = trimmed.split(',');
        let head = parts.next()?;
        let tail = parts.next()?;
        let has_more_parts = parts.next().is_some();
        let head_digits = head.trim_start_matches('-');
        let comma_looks_like_thousands_separator = !has_more_parts
            && head_digits.len() >= 3
            && head_digits.chars().all(|char| char.is_ascii_digit())
            && tail.len() == 3
            && tail.chars().all(|char| char.is_ascii_digit());
        if comma_looks_like_thousands_separator {
            return trimmed.replace(',', "").parse::<f64>().ok();
        }
    }

    trimmed.replace(',', ".").parse::<f64>().ok()
}

fn parse_session_wh_from_report(report: &serde_json::Map<String, Value>) -> Option<f64> {
    if let Some(wh) =
        find_value(report, &["Energy Session", "energy_present_session"]).and_then(parse_f64)
    {
        return Some(wh);
    }

    find_value(report, &["E Pres", "E pres"])
        .and_then(parse_f64)
        .map(|value| value / 10.0)
}

fn stringify_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        _ => value.to_string(),
    }
}

fn parse_absolute_timestamp_ms(value: &Value) -> Option<i64> {
    let Value::String(text) = value else {
        return None;
    };
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(parsed.timestamp_millis());
    }
    if let Ok(parsed) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.3f") {
        return Some(Utc.from_utc_datetime(&parsed).timestamp_millis());
    }
    None
}

fn parse_session_timestamp_ms_from_object(
    object: &serde_json::Map<String, Value>,
    aliases: &[&str],
    now_ms: i64,
) -> Option<i64> {
    let sec_from_report =
        find_value(object, &["Sec", "sec", "Seconds", "seconds"]).and_then(parse_f64);
    let value = find_value(object, aliases)?;
    if let Some(raw_numeric) = parse_f64(value)
        && raw_numeric <= 0.0
    {
        return None;
    }
    if let Some(absolute_ms) = parse_absolute_timestamp_ms(value)
        && is_plausible_absolute_timestamp_ms(absolute_ms, now_ms)
    {
        return Some(absolute_ms);
    }
    if let Some(raw_numeric) = parse_f64(value)
        && let Some(numeric_timestamp_ms) = parse_numeric_timestamp_ms(raw_numeric, now_ms)
    {
        return Some(numeric_timestamp_ms);
    }
    if let Some(sec_now) = sec_from_report
        && let Some(raw_seconds) = parse_f64(value)
        && (0.0..1_000_000_000_000.0).contains(&raw_seconds)
    {
        let ts = (now_ms as f64) - ((sec_now - raw_seconds) * 1000.0);
        let ts_ms = ts.round() as i64;
        if is_plausible_absolute_timestamp_ms(ts_ms, now_ms) {
            return Some(ts_ms);
        }
    }

    None
}

fn is_plausible_absolute_timestamp_ms(timestamp_ms: i64, now_ms: i64) -> bool {
    const MIN_PLAUSIBLE_TIMESTAMP_MS: i64 = 946_684_800_000; // 2000-01-01T00:00:00Z
    const MAX_FUTURE_DRIFT_MS: i64 = 86_400_000; // +24h
    timestamp_ms >= MIN_PLAUSIBLE_TIMESTAMP_MS && timestamp_ms <= now_ms + MAX_FUTURE_DRIFT_MS
}

fn parse_numeric_timestamp_ms(raw_value: f64, now_ms: i64) -> Option<i64> {
    if !raw_value.is_finite() || raw_value < 0.0 {
        return None;
    }
    const MIN_PLAUSIBLE_TIMESTAMP_SECONDS: f64 = 946_684_800.0; // 2000-01-01T00:00:00Z
    const MAX_FUTURE_DRIFT_SECONDS: f64 = 86_400.0; // +24h
    let now_seconds = (now_ms as f64) / 1000.0;
    if (MIN_PLAUSIBLE_TIMESTAMP_SECONDS..=now_seconds + MAX_FUTURE_DRIFT_SECONDS)
        .contains(&raw_value)
    {
        return Some((raw_value * 1000.0).round() as i64);
    }
    let raw_ms = raw_value.round() as i64;
    if is_plausible_absolute_timestamp_ms(raw_ms, now_ms) {
        return Some(raw_ms);
    }
    None
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
        report100_stations: build_report100_stations(&config),
        session_query_service: Some(session_service.clone()),
    };

    let poller = build_poller(&config, session_service.clone())?;
    let stop_flag = Arc::new(AtomicBool::new(false));
    let poller_handle = start_poller(
        poller,
        Duration::from_millis(config.poll_interval_ms),
        Arc::clone(&stop_flag),
    );

    let server_result = run_http_server(&config.http_bind, &config.cors_allowed_origins, api_state);
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
    let connection = open_shared_connection_writer(&config.db_path)?;
    let session_query_service = SqliteSessionService::new(connection);
    let api_state = ApiState {
        report100_stations: build_report100_stations(&config),
        session_query_service: Some(session_query_service),
    };

    run_http_server(&config.http_bind, &config.cors_allowed_origins, api_state)
}

fn open_shared_connection_writer(db_path: &str) -> Result<Arc<Mutex<Connection>>, AppError> {
    let mut connection =
        crate::adapters::db::open_connection(db_path).map_err(AppError::database_init)?;
    crate::adapters::db::run_migrations(&mut connection).map_err(AppError::database_init)?;
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
            && !mapped
                .iter()
                .any(|entry: &Report100Station| entry.logical_name == logical_name)
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

fn run_debug_replay_loop(
    poller: &mut PlugStatusPoller,
    poll_interval_ms: u64,
) -> Result<(), AppError> {
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

fn build_cors(cors_allowed_origins: &CorsAllowedOrigins) -> Cors {
    let cors = Cors::default()
        .allowed_methods([Method::GET, Method::OPTIONS])
        .allowed_headers([header::ACCEPT, header::AUTHORIZATION, header::CONTENT_TYPE])
        .max_age(3600);

    match cors_allowed_origins {
        CorsAllowedOrigins::Any => cors.allow_any_origin(),
        CorsAllowedOrigins::Exact(origins) => origins
            .iter()
            .fold(cors, |cors, origin| cors.allowed_origin(origin)),
    }
}

fn run_http_server(
    http_bind: &str,
    cors_allowed_origins: &CorsAllowedOrigins,
    api_state: ApiState,
) -> Result<(), AppError> {
    tracing::info!(bind = %http_bind, "http server starting");
    let cors_allowed_origins = cors_allowed_origins.clone();
    let server_result = actix_web::rt::System::new().block_on(async move {
        HttpServer::new(move || {
            App::new()
                .wrap(build_cors(&cors_allowed_origins))
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
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use actix_web::http::{Method, StatusCode, header};
    use actix_web::{App, HttpResponse, web};
    use serde_json::json;

    use super::PlugStatusPoller;
    use crate::adapters::keba_debug_file::KebaDebugFileClient;
    use crate::adapters::keba_udp::{KebaClient, KebaClientError};
    use crate::app::config::CorsAllowedOrigins;
    use crate::app::services::SqliteSessionService;
    use crate::test_support::open_test_connection;

    struct FakeKebaClient {
        report2_payloads: Mutex<VecDeque<serde_json::Value>>,
        report3_payloads: Mutex<VecDeque<serde_json::Value>>,
        report_1xx_payloads: HashMap<u16, serde_json::Value>,
    }

    impl FakeKebaClient {
        fn new(
            report2_payloads: Vec<serde_json::Value>,
            report3_payloads: Vec<serde_json::Value>,
        ) -> Self {
            Self {
                report2_payloads: Mutex::new(VecDeque::from(report2_payloads)),
                report3_payloads: Mutex::new(VecDeque::from(report3_payloads)),
                report_1xx_payloads: HashMap::new(),
            }
        }

        fn with_1xx_reports(mut self, reports: Vec<(u16, serde_json::Value)>) -> Self {
            self.report_1xx_payloads = reports.into_iter().collect();
            self
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
            self.get_report(100)
        }

        fn get_report101(&self) -> Result<serde_json::Value, KebaClientError> {
            self.get_report(101)
        }

        fn get_report(&self, report_id: u16) -> Result<serde_json::Value, KebaClientError> {
            self.report_1xx_payloads
                .get(&report_id)
                .cloned()
                .ok_or_else(|| {
                    KebaClientError::Io(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        format!("report {report_id} not available in FakeKebaClient"),
                    ))
                })
        }
    }

    #[test]
    fn extract_unplug_details_converts_e_pres_thousands_string_to_wh() {
        let report = json!({
            "E Pres": "285,000",
            "started": "2026-03-04 17:01:00.000",
            "ended": "2026-03-05 05:29:00.000",
            "RFID tag": "C1"
        });
        let report_obj = report
            .as_object()
            .expect("report fixture should be an object");
        let disconnected_at_ms = chrono::DateTime::parse_from_rfc3339("2026-03-05T05:29:00Z")
            .expect("timestamp fixture should parse")
            .timestamp_millis();

        let details = super::extract_unplug_details_from_report(report_obj, disconnected_at_ms);

        assert_eq!(details.wh, "28500.0");
        assert_eq!(details.card_id, "C1");
    }

    #[actix_web::test]
    async fn cors_allows_default_localhost_origin_and_preflight_headers() {
        let app = actix_web::test::init_service(
            App::new()
                .wrap(super::build_cors(&CorsAllowedOrigins::Exact(vec![
                    "http://localhost:3000".to_string(),
                    "https://invessiv.de".to_string(),
                ])))
                .route(
                    "/health",
                    web::get().to(|| async { HttpResponse::Ok().finish() }),
                ),
        )
        .await;

        let req = actix_web::test::TestRequest::default()
            .method(Method::OPTIONS)
            .uri("/health")
            .insert_header((header::ORIGIN, "http://localhost:3000"))
            .insert_header((header::ACCESS_CONTROL_REQUEST_METHOD, "GET"))
            .insert_header((header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type"))
            .to_request();
        let resp = actix_web::test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&header::HeaderValue::from_static("http://localhost:3000"))
        );
        let allow_methods = resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .expect("preflight response should include allow-methods")
            .to_str()
            .expect("allow-methods should be valid ascii");
        assert!(allow_methods.contains("GET"));
        assert!(allow_methods.contains("OPTIONS"));
        assert!(
            resp.headers()
                .get(header::ACCESS_CONTROL_ALLOW_HEADERS)
                .expect("preflight response should include allow-headers")
                .to_str()
                .expect("allow-headers should be valid ascii")
                .contains("content-type")
        );
    }

    #[actix_web::test]
    async fn cors_allows_default_invessiv_origin() {
        let app = actix_web::test::init_service(
            App::new()
                .wrap(super::build_cors(&CorsAllowedOrigins::Exact(vec![
                    "http://localhost:3000".to_string(),
                    "https://invessiv.de".to_string(),
                ])))
                .route(
                    "/health",
                    web::get().to(|| async { HttpResponse::Ok().finish() }),
                ),
        )
        .await;

        let req = actix_web::test::TestRequest::default()
            .method(Method::OPTIONS)
            .uri("/health")
            .insert_header((header::ORIGIN, "https://invessiv.de"))
            .insert_header((header::ACCESS_CONTROL_REQUEST_METHOD, "GET"))
            .to_request();
        let resp = actix_web::test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&header::HeaderValue::from_static("https://invessiv.de"))
        );
    }

    #[actix_web::test]
    async fn cors_rejects_origin_outside_explicit_allow_list() {
        let app = actix_web::test::init_service(
            App::new()
                .wrap(super::build_cors(&CorsAllowedOrigins::Exact(vec![
                    "https://app.example.com".to_string(),
                ])))
                .route(
                    "/health",
                    web::get().to(|| async { HttpResponse::Ok().finish() }),
                ),
        )
        .await;

        let req = actix_web::test::TestRequest::default()
            .method(Method::OPTIONS)
            .uri("/health")
            .insert_header((header::ORIGIN, "https://blocked.example.com"))
            .insert_header((header::ACCESS_CONTROL_REQUEST_METHOD, "GET"))
            .to_request();
        let resp = actix_web::test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn extract_unplug_details_converts_small_e_pres_to_wh() {
        let report = json!({
            "E Pres": 285,
            "started": "2026-03-04 17:01:00.000",
            "ended": "2026-03-05 05:29:00.000",
            "RFID tag": "C2"
        });
        let report_obj = report
            .as_object()
            .expect("report fixture should be an object");
        let disconnected_at_ms = chrono::DateTime::parse_from_rfc3339("2026-03-05T05:29:00Z")
            .expect("timestamp fixture should parse")
            .timestamp_millis();

        let details = super::extract_unplug_details_from_report(report_obj, disconnected_at_ms);

        assert_eq!(details.wh, "28.5");
        assert_eq!(details.card_id, "C2");
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
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
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
            let db = connection
                .lock()
                .expect("connection lock should be available");
            let unplug_count: i64 = db
                .query_row("SELECT COUNT(*) FROM unplug_log_events", [], |row| {
                    row.get(0)
                })
                .expect("unplug count query should succeed");
            assert_eq!(unplug_count, 0);
        }
        poller.tick().expect("debounced unplug tick should succeed");

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let unplug_count: i64 = db
            .query_row("SELECT COUNT(*) FROM unplug_log_events", [], |row| {
                row.get(0)
            })
            .expect("unplug count query should succeed");

        assert_eq!(unplug_count, 1);
    }

    #[test]
    fn startup_with_vehicle_already_connected_still_persists_unplug_event() {
        let connection = Arc::new(Mutex::new(open_test_connection(
            "runtime-startup-plugged-unplug",
        )));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));

        let fake_client = FakeKebaClient::new(
            vec![
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 3}),
                json!({"Plug": 3}),
            ],
            vec![],
        )
        .with_1xx_reports(vec![
            (
                100,
                json!({"E Pres": 81159, "Sec": 197769, "started[s]": 191012, "ended[s]": 197682, "RFID tag": "BOOT1"}),
            ),
        ]);

        let mut poller = PlugStatusPoller::new(
            Box::new(fake_client),
            session_service,
            "Carport".to_string(),
            2,
        );

        for _ in 0..4 {
            poller.tick().expect("poll tick should succeed");
        }

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let row: (String, String) = db
            .query_row(
                "SELECT Wh, CardId FROM unplug_log_events ORDER BY Timestamp DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("inserted unplug event should be readable");

        assert_eq!(row.0, "8115.9");
        assert_eq!(row.1, "BOOT1");
    }

    #[test]
    fn unplug_transition_uses_first_complete_report_1xx_payload_for_db_values() {
        let connection = Arc::new(Mutex::new(open_test_connection(
            "runtime-unplug-report-1xx",
        )));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));

        let fake_client = FakeKebaClient::new(
            vec![
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
            ],
            vec![
                json!({"Energy (present session)": 0.0, "Energy (total)": 10.0}),
                json!({"Energy (present session)": 7.2, "Energy (total)": 17.2}),
            ],
        )
        .with_1xx_reports(vec![
            (
                100,
                json!({"E Pres": 42000, "started": "2026-03-01 16:40:19.000", "ended": 0, "RFID tag": "R100"}),
            ),
            (
                101,
                json!({"E Pres": 0, "started": "2026-03-01 16:40:19.000", "ended": "2026-03-02 04:01:59.000", "RFID tag": "R101"}),
            ),
            (
                102,
                json!({"E Pres": 12300, "started": null, "ended": "2026-03-02 04:01:59.000", "RFID tag": "R102"}),
            ),
            (
                103,
                json!({"E Pres": 65400, "started": "2026-03-01 16:40:19.000", "ended": "2026-03-02 04:01:59.000", "RFID tag": "R103"}),
            ),
        ]);

        let mut poller = PlugStatusPoller::new(
            Box::new(fake_client),
            session_service,
            "Carport".to_string(),
            3,
        );

        for _ in 0..9 {
            poller.tick().expect("poll tick should succeed");
        }

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let row: (String, String, String, String) = db
            .query_row(
                "SELECT Started, Ended, Wh, CardId FROM unplug_log_events ORDER BY Timestamp DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("inserted unplug event should be readable");

        assert_eq!(row.0, "2026-03-01 16:40");
        assert_eq!(row.1, "2026-03-02 04:01");
        assert_eq!(row.2, "0.0");
        assert_eq!(row.3, "R101");
    }

    #[test]
    fn unplug_transition_skips_report_with_zero_ended_seconds_and_uses_next_report() {
        let connection = Arc::new(Mutex::new(open_test_connection(
            "runtime-unplug-report-1xx-ended-seconds-zero",
        )));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));

        let fake_client = FakeKebaClient::new(
            vec![
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
            ],
            vec![
                json!({"Energy (present session)": 0.0, "Energy (total)": 10.0}),
                json!({"Energy (present session)": 7.2, "Energy (total)": 17.2}),
            ],
        )
        .with_1xx_reports(vec![
            (
                100,
                json!({"E Pres": 71077, "Sec": 195395, "started[s]": 191012, "ended[s]": 0, "started": "191012000", "ended": "0", "RFID tag": "R100"}),
            ),
            (
                101,
                json!({"E Pres": 65400, "Sec": 195395, "started[s]": 182170, "ended[s]": 184901, "RFID tag": "R101"}),
            ),
        ]);

        let mut poller = PlugStatusPoller::new(
            Box::new(fake_client),
            session_service,
            "Carport".to_string(),
            3,
        );

        for _ in 0..9 {
            poller.tick().expect("poll tick should succeed");
        }

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let row: (String, String, String, String) = db
            .query_row(
                "SELECT Started, Ended, Wh, CardId FROM unplug_log_events ORDER BY Timestamp DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("inserted unplug event should be readable");

        assert_eq!(row.2, "6540.0");
        assert_eq!(row.3, "R101");
    }

    #[test]
    fn unplug_transition_requires_complete_timestamps_even_for_zero_kwh() {
        let connection = Arc::new(Mutex::new(open_test_connection(
            "runtime-unplug-report-1xx-zero-fallback",
        )));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));

        let fake_client = FakeKebaClient::new(
            vec![
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
            ],
            vec![],
        )
        .with_1xx_reports(vec![(
            100,
            json!({"E Pres": 0, "ended[s]": 0, "RFID tag": "RZERO"}),
        )]);

        let mut poller = PlugStatusPoller::new(
            Box::new(fake_client),
            session_service,
            "Carport".to_string(),
            3,
        );

        for _ in 0..9 {
            poller.tick().expect("poll tick should succeed");
        }

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let row: (String, String, String, String) = db
            .query_row(
                "SELECT Started, Ended, Wh, CardId FROM unplug_log_events ORDER BY Timestamp DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("inserted unplug event should be readable");

        assert_eq!(row.0, "n/a");
        assert_eq!(row.1, "n/a");
        assert_eq!(row.2, "n/a");
        assert_eq!(row.3, "n/a");
    }

    #[test]
    fn unplug_transition_accepts_complete_zero_kwh_report_without_iterating_further() {
        let connection = Arc::new(Mutex::new(open_test_connection(
            "runtime-unplug-report-1xx-zero-complete",
        )));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));

        let fake_client = FakeKebaClient::new(
            vec![
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 7}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
                json!({"Plug": 0}),
            ],
            vec![],
        )
        .with_1xx_reports(vec![
            (
                100,
                json!({"E Pres": 0, "started": "2026-03-04 15:41:00.000", "ended": "2026-03-05 07:29:00.000", "RFID tag": "RZERO"}),
            ),
            (
                101,
                json!({"E Pres": 65400, "started": "2026-03-04 15:40:00.000", "ended": "2026-03-05 07:30:00.000", "RFID tag": "R101"}),
            ),
        ]);

        let mut poller = PlugStatusPoller::new(
            Box::new(fake_client),
            session_service,
            "Carport".to_string(),
            3,
        );

        for _ in 0..9 {
            poller.tick().expect("poll tick should succeed");
        }

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let row: (String, String, String, String) = db
            .query_row(
                "SELECT Started, Ended, Wh, CardId FROM unplug_log_events ORDER BY Timestamp DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("inserted unplug event should be readable");

        assert_eq!(row.0, "2026-03-04 15:41");
        assert_eq!(row.1, "2026-03-05 07:29");
        assert_eq!(row.2, "0.0");
        assert_eq!(row.3, "RZERO");
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
                json!({"Plug": 7}),
                json!({"Plug": 0}),
                json!({"Plug": 7}),
                json!({"Plug": 0}),
                json!({"Plug": 7}),
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

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let unplug_count: i64 = db
            .query_row("SELECT COUNT(*) FROM unplug_log_events", [], |row| {
                row.get(0)
            })
            .expect("unplug count query should succeed");
        assert_eq!(unplug_count, 0);
    }

    #[test]
    fn debug_replay_two_minutes_writes_three_unplug_events() {
        let connection = Arc::new(Mutex::new(open_test_connection(
            "runtime-debug-two-minutes",
        )));
        let session_service = SqliteSessionService::new(Arc::clone(&connection));
        let fixture_path = format!(
            "{}/testdata/debug/two_minutes_three_unplugs.json",
            env!("CARGO_MANIFEST_DIR").replace('\\', "/")
        );
        let debug_client =
            KebaDebugFileClient::from_file(&fixture_path).expect("debug fixture should load");

        let mut poller = PlugStatusPoller::new(
            Box::new(debug_client),
            session_service,
            "Carport".to_string(),
            3,
        );

        let poll_interval_ms = 100_u64;
        assert!(poll_interval_ms < 20_000);
        super::run_debug_replay_loop(&mut poller, poll_interval_ms)
            .expect("debug replay should finish");

        let db = connection
            .lock()
            .expect("connection lock should be available");
        let unplug_count: i64 = db
            .query_row("SELECT COUNT(*) FROM unplug_log_events", [], |row| {
                row.get(0)
            })
            .expect("unplug count query should succeed");
        assert_eq!(unplug_count, 3);
    }
}
