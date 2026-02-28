use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use std::{fs, path::Path};

use actix_web::{App, HttpServer, web};
use chrono::{SecondsFormat, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::adapters::api::{ApiState, configure_routes};
use crate::adapters::db::{DbError, NewLogEventRecord, NewSessionRecord};
use crate::adapters::keba_debug_file::KebaDebugFileClient;
use crate::adapters::keba_modbus::KebaModbusClient;
use crate::adapters::keba_udp::{KebaClient, KebaClientError, KebaUdpClient};
use crate::app::config::{AppConfig, KebaSource, StatusStationConfig};
use crate::app::error::AppError;
use crate::app::services::{ServiceError, SessionCommandHandler, SqliteSessionService};
use crate::domain::keba_payload::{ParseError, parse_report2, parse_report3};
use crate::domain::session_energy::{EnergySnapshot, compute_session_kwh};
use crate::domain::session_state::{Clock, SessionStateMachine, SessionTransition, TimestampMs};

const SESSION_PERSIST_MAX_RETRIES: usize = 3;
const SESSION_PERSIST_RETRY_BACKOFF_MS: u64 = 250;

#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> TimestampMs {
        TimestampMs(Utc::now().timestamp_millis())
    }
}

#[derive(Debug, Error)]
pub enum PollerError {
    #[error("failed to fetch report 2: {0}")]
    FetchReport2(#[source] KebaClientError),
    #[error("failed to parse report 2: {0}")]
    ParseReport2(#[source] ParseError),
    #[error("database lock poisoned")]
    DbLockPoisoned,
    #[error("database write failed: {0}")]
    Database(#[source] DbError),
    #[error("results file io failed: {0}")]
    ResultsIo(#[source] std::io::Error),
}

pub struct SessionPoller<Cl> {
    client: Box<dyn KebaClient>,
    clock: Cl,
    session_commands: SqliteSessionService,
    machine: SessionStateMachine,
    start_snapshot: Option<EnergySnapshot>,
    start_report2_raw: Option<String>,
    start_report3_raw: Option<String>,
    last_seconds: Option<u64>,
    source: String,
    poll_interval_ms: i64,
    debounce_samples: i64,
    station_id: Option<String>,
    error_count_during_session: i64,
    pending_session_log_event_ids: Vec<String>,
    results_output_file: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeConsoleStation {
    name: String,
    ip: String,
    port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionResultEntry {
    from: String,
    to: String,
    #[serde(rename = "durationMs")]
    duration_ms: i64,
    kwh: f64,
}

struct SessionCompletion {
    energy_kwh: f64,
    status: &'static str,
    finished_reason: &'static str,
    report2_end_raw: String,
    report3_end_raw: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionPollerConfig {
    pub source: String,
    pub poll_interval_ms: u64,
    pub station_id: Option<String>,
    pub results_output_file: Option<String>,
}

impl<Cl: Clock> SessionPoller<Cl> {
    pub fn new(
        client: Box<dyn KebaClient>,
        clock: Cl,
        session_commands: SqliteSessionService,
        debounce_samples: usize,
        config: SessionPollerConfig,
    ) -> Self {
        Self {
            client,
            clock,
            session_commands,
            machine: SessionStateMachine::new(debounce_samples),
            start_snapshot: None,
            start_report2_raw: None,
            start_report3_raw: None,
            last_seconds: None,
            source: config.source,
            poll_interval_ms: i64::try_from(config.poll_interval_ms).unwrap_or(i64::MAX),
            debounce_samples: i64::try_from(debounce_samples).unwrap_or(i64::MAX),
            station_id: config.station_id,
            error_count_during_session: 0,
            pending_session_log_event_ids: Vec::new(),
            results_output_file: config.results_output_file,
        }
    }

    pub fn tick(&mut self) -> Result<(), PollerError> {
        let report2_raw = self
            .client
            .get_report2()
            .map_err(PollerError::FetchReport2)?;
        let report2 = parse_report2(&report2_raw).map_err(PollerError::ParseReport2)?;

        if let (Some(previous), Some(current)) = (self.last_seconds, report2.seconds)
            && current < previous
        {
            tracing::warn!(
                previous_seconds = previous,
                current_seconds = current,
                "report2 seconds counter moved backwards"
            );
        }
        self.last_seconds = report2.seconds;

        let transition = if let Some(observed_at) = extract_observed_at(&report2_raw) {
            self.machine.observe_at(report2.plugged, observed_at)
        } else {
            self.machine.observe(report2.plugged, &self.clock)
        };

        match transition {
            Some(SessionTransition::Plugged { plugged_at }) => {
                self.handle_plugged(plugged_at, report2_raw.clone());
            }
            Some(SessionTransition::Unplugged {
                plugged_at,
                unplugged_at,
            }) => self.handle_unplugged(plugged_at, unplugged_at, report2_raw)?,
            None => {}
        }

        Ok(())
    }

    pub fn note_poll_error(&mut self, error: &PollerError) {
        let is_active_session = self.machine.active_session_started_at().is_some();
        if is_active_session {
            self.error_count_during_session += 1;
        }
        self.persist_log_event(
            "warn",
            poller_error_code(error),
            &error.to_string(),
            is_active_session,
            Some(json!({
                "activeSession": is_active_session,
                "errorCountDuringSession": self.error_count_during_session,
            })),
        );
    }

    fn persist_log_event(
        &mut self,
        level: &str,
        code: &str,
        message: &str,
        link_to_active_session: bool,
        details: Option<Value>,
    ) {
        let log_event = NewLogEventRecord {
            created_at: timestamp_to_iso8601(self.clock.now()),
            level: level.to_string(),
            code: code.to_string(),
            message: message.to_string(),
            source: self.source.clone(),
            station_id: self.station_id.clone(),
            details_json: details.map(|value| value.to_string()),
        };

        match self.session_commands.insert_log_event(&log_event) {
            Ok(log_event_id) if link_to_active_session => {
                self.pending_session_log_event_ids.push(log_event_id);
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(error = %error, "failed to persist log event");
            }
        }
    }

    fn handle_plugged(&mut self, plugged_at: TimestampMs, report2_raw: Value) {
        let report3_raw = match self.client.get_report3() {
            Ok(value) => value,
            Err(error) => {
                self.start_snapshot = None;
                self.error_count_during_session += 1;
                self.persist_log_event(
                    "warn",
                    "poll.fetch_report3_on_plugged",
                    &error.to_string(),
                    true,
                    None,
                );
                tracing::warn!(error = %error, "failed to fetch report 3 on plugged transition");
                return;
            }
        };

        let report3 = match parse_report3(&report3_raw) {
            Ok(report3) => report3,
            Err(error) => {
                self.start_snapshot = None;
                self.error_count_during_session += 1;
                self.persist_log_event(
                    "warn",
                    "poll.parse_report3_on_plugged",
                    &error.to_string(),
                    true,
                    None,
                );
                tracing::warn!(error = %error, "failed to parse report 3 on plugged transition");
                return;
            }
        };

        self.start_report2_raw = Some(report2_raw.to_string());
        self.start_report3_raw = Some(report3_raw.to_string());
        self.error_count_during_session = 0;
        self.pending_session_log_event_ids.clear();
        self.start_snapshot = Some(EnergySnapshot {
            present_session_kwh: report3.present_session_kwh,
            total_kwh: report3.total_kwh,
        });

        tracing::info!(
            plugged_at = %timestamp_to_iso8601(plugged_at),
            "charging session started"
        );
    }

    fn handle_unplugged(
        &mut self,
        plugged_at: TimestampMs,
        unplugged_at: TimestampMs,
        report2_raw: Value,
    ) -> Result<(), PollerError> {
        let report3_raw = match self.client.get_report3() {
            Ok(raw) => raw,
            Err(error) => {
                self.persist_log_event(
                    "warn",
                    "poll.fetch_report3_on_unplugged",
                    &error.to_string(),
                    true,
                    Some(json!({
                        "startedAt": timestamp_to_iso8601(plugged_at),
                        "finishedAt": timestamp_to_iso8601(unplugged_at),
                    })),
                );
                let new_session = self.build_session_record(
                    plugged_at,
                    unplugged_at,
                    SessionCompletion {
                        energy_kwh: 0.0,
                        status: "aborted",
                        finished_reason: "report3_fetch_failed",
                        report2_end_raw: report2_raw.to_string(),
                        report3_end_raw: None,
                    },
                );
                self.persist_session_and_finalize(&new_session)?;
                return Ok(());
            }
        };
        let report3 = match parse_report3(&report3_raw) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.persist_log_event(
                    "warn",
                    "poll.parse_report3_on_unplugged",
                    &error.to_string(),
                    true,
                    Some(json!({
                        "startedAt": timestamp_to_iso8601(plugged_at),
                        "finishedAt": timestamp_to_iso8601(unplugged_at),
                    })),
                );
                let new_session = self.build_session_record(
                    plugged_at,
                    unplugged_at,
                    SessionCompletion {
                        energy_kwh: 0.0,
                        status: "invalid",
                        finished_reason: "report3_parse_failed",
                        report2_end_raw: report2_raw.to_string(),
                        report3_end_raw: Some(report3_raw.to_string()),
                    },
                );
                self.persist_session_and_finalize(&new_session)?;
                return Ok(());
            }
        };

        let end_snapshot = EnergySnapshot {
            present_session_kwh: report3.present_session_kwh,
            total_kwh: report3.total_kwh,
        };

        let energy = compute_session_kwh(self.start_snapshot.as_ref(), &end_snapshot);
        let (energy_kwh, status, finished_reason) = match energy {
            Ok(energy) if energy.warnings.is_empty() => {
                (energy.kwh, "completed", "plug_state_transition")
            }
            Ok(energy) => {
                self.persist_log_event(
                    "warn",
                    "poll.energy_warning",
                    "energy clamped due to negative delta/value",
                    true,
                    Some(json!({
                        "warnings": format!("{:?}", energy.warnings),
                    })),
                );
                (energy.kwh, "invalid", "energy_clamped")
            }
            Err(error) => {
                self.persist_log_event(
                    "warn",
                    "poll.compute_energy_on_unplugged",
                    &error.to_string(),
                    true,
                    Some(json!({
                        "startedAt": timestamp_to_iso8601(plugged_at),
                        "finishedAt": timestamp_to_iso8601(unplugged_at),
                    })),
                );
                (0.0, "invalid", "energy_compute_failed")
            }
        };
        let new_session = self.build_session_record(
            plugged_at,
            unplugged_at,
            SessionCompletion {
                energy_kwh,
                status,
                finished_reason,
                report2_end_raw: report2_raw.to_string(),
                report3_end_raw: Some(report3_raw.to_string()),
            },
        );

        let session_id = self.persist_session_and_finalize(&new_session)?;

        tracing::info!(
            session_id,
            started_at = %new_session.started_at,
            finished_at = %new_session.finished_at,
            kwh = new_session.energy_kwh,
            "charging session persisted"
        );

        if let Some(path) = self.results_output_file.as_deref() {
            let duration_ms = (unplugged_at.0 - plugged_at.0).max(0);
            append_session_result(path, &new_session, duration_ms)
                .map_err(PollerError::ResultsIo)?;
        }

        Ok(())
    }

    fn build_session_record(
        &self,
        plugged_at: TimestampMs,
        unplugged_at: TimestampMs,
        completion: SessionCompletion,
    ) -> NewSessionRecord {
        NewSessionRecord {
            started_at: timestamp_to_iso8601(plugged_at),
            finished_at: timestamp_to_iso8601(unplugged_at),
            duration_ms: (unplugged_at.0 - plugged_at.0).max(0),
            energy_kwh: completion.energy_kwh,
            source: self.source.clone(),
            status: completion.status.to_string(),
            started_reason: "plug_state_transition".to_string(),
            finished_reason: completion.finished_reason.to_string(),
            poll_interval_ms: self.poll_interval_ms,
            debounce_samples: self.debounce_samples,
            error_count_during_session: self.error_count_during_session,
            station_id: self.station_id.clone(),
            created_at: timestamp_to_iso8601(unplugged_at),
            raw_report2_start: self.start_report2_raw.clone(),
            raw_report3_start: self.start_report3_raw.clone(),
            raw_report2_end: Some(completion.report2_end_raw),
            raw_report3_end: completion.report3_end_raw,
        }
    }

    fn persist_session_and_finalize(
        &mut self,
        new_session: &NewSessionRecord,
    ) -> Result<String, PollerError> {
        let mut insert_attempt = 0_usize;
        let session_id = loop {
            match self.session_commands.insert_session(new_session) {
                Ok(session_id) => break session_id,
                Err(error)
                    if is_retryable_db_contention(&error)
                        && insert_attempt < SESSION_PERSIST_MAX_RETRIES =>
                {
                    insert_attempt += 1;
                    let sleep_ms = SESSION_PERSIST_RETRY_BACKOFF_MS * insert_attempt as u64;
                    tracing::warn!(
                        attempt = insert_attempt,
                        max_attempts = SESSION_PERSIST_MAX_RETRIES,
                        sleep_ms,
                        error = %error,
                        "session insert hit db contention; retrying"
                    );
                    std::thread::sleep(Duration::from_millis(sleep_ms));
                }
                Err(error) => return Err(service_error_to_poller_error(error)),
            }
        };

        let mut link_attempt = 0_usize;
        loop {
            match self
                .session_commands
                .link_session_log_events(&session_id, &self.pending_session_log_event_ids)
            {
                Ok(()) => break,
                Err(error)
                    if is_retryable_db_contention(&error)
                        && link_attempt < SESSION_PERSIST_MAX_RETRIES =>
                {
                    link_attempt += 1;
                    let sleep_ms = SESSION_PERSIST_RETRY_BACKOFF_MS * link_attempt as u64;
                    tracing::warn!(
                        attempt = link_attempt,
                        max_attempts = SESSION_PERSIST_MAX_RETRIES,
                        sleep_ms,
                        error = %error,
                        session_id,
                        "session-log linking hit db contention; retrying"
                    );
                    std::thread::sleep(Duration::from_millis(sleep_ms));
                }
                Err(error) => return Err(service_error_to_poller_error(error)),
            }
        }

        self.start_snapshot = None;
        self.start_report2_raw = None;
        self.start_report3_raw = None;
        self.error_count_during_session = 0;
        self.pending_session_log_event_ids.clear();

        Ok(session_id)
    }
}

fn append_session_result(
    path: &str,
    session: &NewSessionRecord,
    duration_ms: i64,
) -> Result<(), std::io::Error> {
    let mut existing = if Path::new(path).exists() {
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            Vec::new()
        } else {
            serde_json::from_str::<Vec<SessionResultEntry>>(&content).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("failed to parse existing results json: {error}"),
                )
            })?
        }
    } else {
        Vec::new()
    };

    existing.push(SessionResultEntry {
        from: session.started_at.clone(),
        to: session.finished_at.clone(),
        duration_ms,
        kwh: session.energy_kwh,
    });

    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(&existing).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("failed to serialize results json: {error}"),
        )
    })?;
    fs::write(path, json)?;
    Ok(())
}

fn log_console_station_statuses(stations: &[RuntimeConsoleStation]) {
    for station in stations {
        match fetch_report_pair(station) {
            Ok((report2, report3)) => {
                let status = derive_console_status(&report2, &report3);
                println!(
                    "[{}] {} ({}) | {} | Stecker: {} | Laden: {} | E pres: {}",
                    Utc::now().to_rfc3339(),
                    station.name,
                    station.ip,
                    status,
                    bool_text(find_number(&report2, &["Plug"]).unwrap_or(0.0) != 0.0),
                    bool_text(find_number(&report3, &["P"]).unwrap_or(0.0) > 0.0),
                    session_energy_text(&report3)
                );
            }
            Err(error) => {
                println!(
                    "[{}] {} ({}) | FEHLER beim Statuspolling: {}",
                    Utc::now().to_rfc3339(),
                    station.name,
                    station.ip,
                    error
                );
            }
        }
    }
}

fn fetch_report_pair(station: &RuntimeConsoleStation) -> Result<(Value, Value), KebaClientError> {
    let client = KebaUdpClient::new(&station.ip, station.port)?;
    let report2 = client.get_report2()?;
    let report3 = client.get_report3()?;
    Ok((report2, report3))
}

fn derive_console_status(report2: &Value, report3: &Value) -> &'static str {
    let plugged = find_number(report2, &["Plug"]).unwrap_or(0.0) != 0.0;
    let enabled = find_number(report2, &["Enable sys"]).unwrap_or(0.0) == 1.0
        && find_number(report2, &["Enable user"]).unwrap_or(0.0) == 1.0
        && find_number(report2, &["Max curr"]).unwrap_or(0.0) > 0.0;
    let fault = find_number(report2, &["Error1"]).unwrap_or(0.0) != 0.0
        || find_number(report2, &["Error2"]).unwrap_or(0.0) != 0.0;
    let charging = find_number(report3, &["P"]).unwrap_or(0.0) > 0.0;

    if fault {
        "Fehler"
    } else if !plugged {
        "Nicht angesteckt"
    } else if charging {
        "Laedt"
    } else if !enabled {
        "Angesteckt, gesperrt/deaktiviert"
    } else {
        "Angesteckt, wartet/bereit"
    }
}

fn session_energy_text(report3: &Value) -> String {
    let e_pres = find_number(report3, &["E pres"]).map(|raw| raw / 10_000.0);
    let energy_kwh = e_pres.or_else(|| find_number(report3, &["Energy (present session)"]));
    match energy_kwh {
        Some(kwh) => format!("{kwh:.3} kWh"),
        None => "n/a".to_string(),
    }
}

fn find_number(payload: &Value, aliases: &[&str]) -> Option<f64> {
    let object = payload.as_object()?;

    for alias in aliases {
        if let Some(value) = object.get(*alias)
            && let Some(parsed) = parse_number(value)
        {
            return Some(parsed);
        }
    }

    let normalized_aliases: Vec<String> =
        aliases.iter().map(|alias| normalize_key(alias)).collect();
    object.iter().find_map(|(key, value)| {
        let normalized_key = normalize_key(key);
        if normalized_aliases
            .iter()
            .any(|alias| alias == &normalized_key)
        {
            parse_number(value)
        } else {
            None
        }
    })
}

fn normalize_key(input: &str) -> String {
    input
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_number_from_text(text),
        _ => None,
    }
}

fn parse_number_from_text(text: &str) -> Option<f64> {
    let cleaned = text.trim().replace(',', ".");
    let token = cleaned
        .split(|char: char| !char.is_ascii_digit() && char != '.' && char != '-')
        .find(|part| !part.is_empty())?;
    token.parse::<f64>().ok()
}

fn bool_text(value: bool) -> &'static str {
    if value { "ja" } else { "nein" }
}

fn extract_observed_at(report2_raw: &Value) -> Option<TimestampMs> {
    let object = report2_raw.as_object()?;
    let ts_ms = object.get("__tsMs")?.as_i64()?;
    Some(TimestampMs(ts_ms))
}

fn start_poller<Cl>(
    mut poller: SessionPoller<Cl>,
    poll_interval: Duration,
    status_log_interval: Duration,
    status_stations: Vec<RuntimeConsoleStation>,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    Cl: Clock + Send + 'static,
{
    std::thread::spawn(move || {
        let mut next_status_log = Instant::now();
        while !stop_flag.load(Ordering::Relaxed) {
            if let Err(error) = poller.tick() {
                poller.note_poll_error(&error);
                tracing::warn!(error = %error, "poll cycle failed");
            }
            if next_status_log.elapsed() >= status_log_interval {
                log_console_station_statuses(&status_stations);
                next_status_log = Instant::now();
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
    };

    let status_stations = build_status_stations(&config);
    let mut poller = build_poller(&config, session_service)?;

    if config.keba_source == KebaSource::DebugFile {
        return run_debug_replay_loop(&mut poller, config.poll_interval_ms);
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let poller_handle = start_poller(
        poller,
        Duration::from_millis(config.poll_interval_ms),
        Duration::from_secs(config.status_log_interval_seconds),
        status_stations,
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
    let status_stations = build_status_stations(&config);
    let mut poller = build_poller(&config, session_service)?;

    if config.keba_source == KebaSource::DebugFile {
        return run_debug_replay_loop(&mut poller, config.poll_interval_ms);
    }

    let poller_handle = start_poller(
        poller,
        Duration::from_millis(config.poll_interval_ms),
        Duration::from_secs(config.status_log_interval_seconds),
        status_stations,
        Arc::new(AtomicBool::new(false)),
    );

    match poller_handle.join() {
        Ok(()) => Ok(()),
        Err(_) => Err(AppError::runtime("poller thread panicked")),
    }
}

pub fn run_api(config: AppConfig) -> Result<(), AppError> {
    let shared_connection = open_shared_connection_reader(&config.db_path)?;
    let session_service = SqliteSessionService::new(Arc::clone(&shared_connection));
    let api_state = ApiState {
        session_queries: session_service,
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
    let version = crate::adapters::db::schema_version(&connection).map_err(AppError::database_init)?;
    if version == 0 {
        return Err(AppError::database_init(
            "database schema is not initialized; start writer service first",
        ));
    }
    Ok(Arc::new(Mutex::new(connection)))
}

fn build_status_stations(config: &AppConfig) -> Vec<RuntimeConsoleStation> {
    if config.keba_source == KebaSource::DebugFile {
        return Vec::new();
    }

    config
        .status_stations
        .iter()
        .map(|station: &StatusStationConfig| RuntimeConsoleStation {
            name: station.name.clone(),
            ip: station.ip.clone(),
            port: station.port,
        })
        .collect()
}

fn build_poller(
    config: &AppConfig,
    session_service: SqliteSessionService,
) -> Result<SessionPoller<SystemClock>, AppError> {
    let keba_client = build_keba_client(config)?;
    Ok(SessionPoller::new(
        keba_client,
        SystemClock,
        session_service,
        config.debounce_samples,
        SessionPollerConfig {
            source: keba_source_label(config.keba_source).to_string(),
            poll_interval_ms: config.poll_interval_ms,
            station_id: config.station_id.clone(),
            results_output_file: config.results_output_file.clone(),
        },
    ))
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
    poller: &mut SessionPoller<SystemClock>,
    poll_interval_ms: u64,
) -> Result<(), AppError> {
    loop {
        match poller.tick() {
            Ok(()) => std::thread::sleep(Duration::from_millis(poll_interval_ms)),
            Err(error) if is_debug_replay_finished(&error) => {
                tracing::info!("debug replay finished");
                return Ok(());
            }
            Err(error) => {
                poller.note_poll_error(&error);
                tracing::warn!(error = %error, "poll cycle failed");
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

fn service_error_to_poller_error(error: crate::app::services::ServiceError) -> PollerError {
    match error {
        crate::app::services::ServiceError::DbLockPoisoned => PollerError::DbLockPoisoned,
        crate::app::services::ServiceError::Database(db_error) => PollerError::Database(db_error),
    }
}

fn is_retryable_db_contention(error: &ServiceError) -> bool {
    match error {
        ServiceError::DbLockPoisoned => false,
        ServiceError::Database(DbError::Sqlite(rusqlite::Error::SqliteFailure(db_error, _))) => {
            db_error.code == rusqlite::ErrorCode::DatabaseBusy
                || db_error.code == rusqlite::ErrorCode::DatabaseLocked
        }
        _ => false,
    }
}

fn is_debug_replay_finished(error: &PollerError) -> bool {
    match error {
        PollerError::FetchReport2(KebaClientError::Io(io)) => {
            io.kind() == std::io::ErrorKind::UnexpectedEof
        }
        _ => false,
    }
}

fn poller_error_code(error: &PollerError) -> &'static str {
    match error {
        PollerError::FetchReport2(_) => "poll.fetch_report2",
        PollerError::ParseReport2(_) => "poll.parse_report2",
        PollerError::DbLockPoisoned => "poll.db_lock_poisoned",
        PollerError::Database(_) => "poll.database",
        PollerError::ResultsIo(_) => "poll.results_io",
    }
}

fn keba_source_label(source: KebaSource) -> &'static str {
    match source {
        KebaSource::Udp => "udp",
        KebaSource::Modbus => "modbus",
        KebaSource::DebugFile => "debug_file",
    }
}

fn timestamp_to_iso8601(timestamp: TimestampMs) -> String {
    let datetime = chrono::DateTime::<Utc>::from_timestamp_millis(timestamp.0)
        .unwrap_or_else(|| chrono::DateTime::<Utc>::from(std::time::UNIX_EPOCH));
    datetime.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::net::UdpSocket;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use crate::adapters::db::{
        DbError, count_log_events, count_session_log_events, get_latest_session, list_sessions,
    };
    use crate::adapters::keba_debug_file::KebaDebugFileClient;
    use crate::adapters::keba_udp::KebaUdpClient;
    use crate::app::services::{ServiceError, SqliteSessionService};
    use crate::test_support::open_test_connection;

    use super::{Clock, SessionPoller, SessionPollerConfig, TimestampMs};

    struct StepClock {
        values: Vec<i64>,
        index: Cell<usize>,
    }

    impl StepClock {
        fn new(values: Vec<i64>) -> Self {
            Self {
                values,
                index: Cell::new(0),
            }
        }
    }

    impl Clock for StepClock {
        fn now(&self) -> TimestampMs {
            let index = self.index.get();
            self.index.set(index + 1);
            TimestampMs(*self.values.get(index).unwrap_or(&0))
        }
    }

    #[test]
    fn persists_session_from_simulated_udp_responder() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let report2_frames = [
                r#"{"Plug":0,"Seconds":10}"#,
                r#"{"Plug":0,"Seconds":11}"#,
                r#"{"Plug":7,"Seconds":12}"#,
                r#"{"Plug":7,"Seconds":13}"#,
                r#"{"Plug":0,"Seconds":14}"#,
                r#"{"Plug":0,"Seconds":15}"#,
            ];
            let report3_frames = [
                r#"{"E pres":2000,"Total energy":100000}"#,
                r#"{"E pres":7000,"Total energy":105000}"#,
            ];
            let mut report2_idx = 0_usize;
            let mut report3_idx = 0_usize;
            let mut buffer = [0_u8; 256];

            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();

                if cmd == "shutdown-test-responder" {
                    break;
                }

                let payload = match cmd.as_str() {
                    "report 2" => {
                        let value = report2_frames[report2_idx.min(report2_frames.len() - 1)];
                        report2_idx += 1;
                        value
                    }
                    "report 3" => {
                        let value = report3_frames[report3_idx.min(report3_frames.len() - 1)];
                        report3_idx += 1;
                        value
                    }
                    _ => r#"{"error":"unknown command"}"#,
                };

                responder
                    .send_to(payload.as_bytes(), from)
                    .expect("responder send should succeed");
            }
        });

        let connection = open_test_connection("poller-runtime.sqlite");
        let shared_connection = Arc::new(Mutex::new(connection));

        let client =
            Box::new(KebaUdpClient::new("127.0.0.1", responder_port).expect("client should build"));
        let clock = StepClock::new(vec![1_700_000_000_000, 1_700_000_060_000]);
        let mut poller = SessionPoller::new(
            client,
            clock,
            SqliteSessionService::new(Arc::clone(&shared_connection)),
            2,
            SessionPollerConfig {
                source: "debug_file".to_string(),
                poll_interval_ms: 1000,
                station_id: None,
                results_output_file: None,
            },
        );

        for _ in 0..6 {
            poller.tick().expect("poll tick should succeed");
        }

        {
            let locked = shared_connection
                .lock()
                .expect("database lock should be available");
            let latest = get_latest_session(&locked)
                .expect("db query should succeed")
                .expect("session should exist");
            assert_eq!(latest.energy_kwh, 5.0);
            assert_eq!(latest.started_at, "2023-11-14T22:13:20.000Z");
            assert_eq!(latest.finished_at, "2023-11-14T22:14:20.000Z");
        }

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[test]
    fn debug_file_client_with_intermittent_failures_still_persists_session() {
        let connection = open_test_connection("poller-debug-file.sqlite");
        let shared_connection = Arc::new(Mutex::new(connection));

        let fixture = format!(
            "{}/testdata/debug/poller_recovery.json",
            env!("CARGO_MANIFEST_DIR").replace("\\", "/")
        );
        let client = Box::new(
            KebaDebugFileClient::from_file(&fixture).expect("debug file client should build"),
        );
        let clock = StepClock::new(vec![1_700_000_000_000, 1_700_000_060_000]);
        let mut poller = SessionPoller::new(
            client,
            clock,
            SqliteSessionService::new(Arc::clone(&shared_connection)),
            2,
            SessionPollerConfig {
                source: "debug_file".to_string(),
                poll_interval_ms: 1000,
                station_id: None,
                results_output_file: None,
            },
        );

        for _ in 0..8 {
            let _ = poller.tick();
        }

        let locked = shared_connection
            .lock()
            .expect("database lock should be available");
        let latest = get_latest_session(&locked)
            .expect("db query should succeed")
            .expect("session should exist");
        assert!(latest.energy_kwh > 0.0);
        assert!(
            count_log_events(&locked).expect("log count query should succeed") >= 1,
            "expected at least one persisted log event"
        );
        assert!(
            count_session_log_events(&locked, &latest.id)
                .expect("session-log count query should succeed")
                >= 1,
            "expected at least one linked session log event"
        );
    }

    #[test]
    fn writes_multiple_completed_sessions_to_results_json() {
        let connection = open_test_connection("poller-results-json.sqlite");
        let shared_connection = Arc::new(Mutex::new(connection));

        let results_path = std::path::Path::new("./target/testdb/results.json").to_path_buf();
        let fixture = format!(
            "{}/testdata/debug/happy_loop.json",
            env!("CARGO_MANIFEST_DIR").replace("\\", "/")
        );
        let client = Box::new(
            KebaDebugFileClient::from_file(&fixture).expect("debug file client should build"),
        );
        let clock = StepClock::new(vec![
            1_700_000_000_000,
            1_700_000_060_000,
            1_700_000_120_000,
            1_700_000_180_000,
        ]);
        let mut poller = SessionPoller::new(
            client,
            clock,
            SqliteSessionService::new(Arc::clone(&shared_connection)),
            1,
            SessionPollerConfig {
                source: "debug_file".to_string(),
                poll_interval_ms: 1000,
                station_id: None,
                results_output_file: Some(results_path.to_string_lossy().to_string()),
            },
        );

        for _ in 0..8 {
            let _ = poller.tick();
        }

        let content = std::fs::read_to_string(&results_path).expect("results json should exist");
        let entries: Vec<serde_json::Value> =
            serde_json::from_str(&content).expect("results json should parse");
        assert!(entries.len() >= 2);
        assert!(
            entries
                .iter()
                .all(|entry| entry["kwh"].as_f64().unwrap_or(0.0) >= 0.0)
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry["durationMs"].as_i64().unwrap_or(-1) >= 0)
        );
    }

    #[test]
    fn persists_aborted_session_when_report3_fetch_fails_on_unplugged() {
        let connection = open_test_connection("poller-aborted-unplugged.sqlite");
        let shared_connection = Arc::new(Mutex::new(connection));

        let fixture = format!(
            "{}/testdata/debug/aborted_report3_fetch_on_unplugged.json",
            env!("CARGO_MANIFEST_DIR").replace("\\", "/")
        );
        let client = Box::new(
            KebaDebugFileClient::from_file(&fixture).expect("debug file client should build"),
        );
        let clock = StepClock::new(vec![1_700_000_000_000, 1_700_000_060_000]);
        let mut poller = SessionPoller::new(
            client,
            clock,
            SqliteSessionService::new(Arc::clone(&shared_connection)),
            2,
            SessionPollerConfig {
                source: "debug_file".to_string(),
                poll_interval_ms: 1000,
                station_id: None,
                results_output_file: None,
            },
        );

        for _ in 0..6 {
            let _ = poller.tick();
        }

        let locked = shared_connection
            .lock()
            .expect("database lock should be available");
        let latest = get_latest_session(&locked)
            .expect("db query should succeed")
            .expect("session should exist");
        assert_eq!(latest.status, "aborted");
        assert_eq!(latest.finished_reason, "report3_fetch_failed");
        assert_eq!(latest.energy_kwh, 0.0);
    }

    #[test]
    fn persists_invalid_session_when_energy_cannot_be_computed() {
        let connection = open_test_connection("poller-invalid-energy.sqlite");
        let shared_connection = Arc::new(Mutex::new(connection));

        let fixture = format!(
            "{}/testdata/debug/invalid_energy_source_switch.json",
            env!("CARGO_MANIFEST_DIR").replace("\\", "/")
        );
        let client = Box::new(
            KebaDebugFileClient::from_file(&fixture).expect("debug file client should build"),
        );
        let clock = StepClock::new(vec![1_700_000_000_000, 1_700_000_060_000]);
        let mut poller = SessionPoller::new(
            client,
            clock,
            SqliteSessionService::new(Arc::clone(&shared_connection)),
            2,
            SessionPollerConfig {
                source: "debug_file".to_string(),
                poll_interval_ms: 1000,
                station_id: None,
                results_output_file: None,
            },
        );

        for _ in 0..6 {
            let _ = poller.tick();
        }

        let locked = shared_connection
            .lock()
            .expect("database lock should be available");
        let latest = get_latest_session(&locked)
            .expect("db query should succeed")
            .expect("session should exist");
        assert_eq!(latest.status, "invalid");
        assert_eq!(latest.finished_reason, "energy_compute_failed");
        assert_eq!(latest.energy_kwh, 0.0);
    }

    #[test]
    fn debounce_flap_at_start_does_not_create_session() {
        let connection = open_test_connection("poller-flap-start.sqlite");
        let shared_connection = Arc::new(Mutex::new(connection));

        let fixture = format!(
            "{}/testdata/debug/flap_start_no_session.json",
            env!("CARGO_MANIFEST_DIR").replace("\\", "/")
        );
        let client = Box::new(
            KebaDebugFileClient::from_file(&fixture).expect("debug file client should build"),
        );
        let clock = StepClock::new(vec![1_700_000_000_000, 1_700_000_060_000]);
        let mut poller = SessionPoller::new(
            client,
            clock,
            SqliteSessionService::new(Arc::clone(&shared_connection)),
            2,
            SessionPollerConfig {
                source: "debug_file".to_string(),
                poll_interval_ms: 1000,
                station_id: None,
                results_output_file: None,
            },
        );

        for _ in 0..16 {
            let _ = poller.tick();
        }

        let locked = shared_connection
            .lock()
            .expect("database lock should be available");
        let sessions = list_sessions(&locked, 10, 0).expect("db query should succeed");
        assert_eq!(sessions.len(), 0);
    }

    #[test]
    fn debounce_flap_at_end_creates_single_session_once_stable() {
        let connection = open_test_connection("poller-flap-end.sqlite");
        let shared_connection = Arc::new(Mutex::new(connection));

        let fixture = format!(
            "{}/testdata/debug/flap_end_single_session.json",
            env!("CARGO_MANIFEST_DIR").replace("\\", "/")
        );
        let client = Box::new(
            KebaDebugFileClient::from_file(&fixture).expect("debug file client should build"),
        );
        let clock = StepClock::new(vec![1_700_000_000_000, 1_700_000_060_000]);
        let mut poller = SessionPoller::new(
            client,
            clock,
            SqliteSessionService::new(Arc::clone(&shared_connection)),
            2,
            SessionPollerConfig {
                source: "debug_file".to_string(),
                poll_interval_ms: 1000,
                station_id: None,
                results_output_file: None,
            },
        );

        for _ in 0..18 {
            let _ = poller.tick();
        }

        let locked = shared_connection
            .lock()
            .expect("database lock should be available");
        let sessions = list_sessions(&locked, 10, 0).expect("db query should succeed");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].started_at, "2026-02-27T14:42:00.000Z");
        assert_eq!(sessions[0].finished_at, "2026-02-27T14:46:00.000Z");
        assert_eq!(sessions[0].energy_kwh, 4.0);
    }

    #[test]
    fn retries_only_for_sqlite_busy_or_locked_errors() {
        let busy_error = ServiceError::Database(DbError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy,
                extended_code: 5,
            },
            Some("database is locked".to_string()),
        )));
        let locked_error =
            ServiceError::Database(DbError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ErrorCode::DatabaseLocked,
                    extended_code: 6,
                },
                Some("database table is locked".to_string()),
            )));
        let other_error =
            ServiceError::Database(DbError::Sqlite(rusqlite::Error::ExecuteReturnedResults));

        assert!(super::is_retryable_db_contention(&busy_error));
        assert!(super::is_retryable_db_contention(&locked_error));
        assert!(!super::is_retryable_db_contention(&other_error));
        assert!(!super::is_retryable_db_contention(
            &ServiceError::DbLockPoisoned
        ));
    }
}
