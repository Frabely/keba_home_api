use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

use actix_web::{App, HttpServer, web};
use chrono::{SecondsFormat, Utc};
use rusqlite::Connection;
use serde_json::Value;
use thiserror::Error;

use crate::adapters::api::{ApiState, configure_routes};
use crate::adapters::db::{DbError, NewSessionRecord, insert_session};
use crate::adapters::keba_udp::{KebaClient, KebaClientError, KebaUdpClient};
use crate::app::config::AppConfig;
use crate::app::error::AppError;
use crate::domain::keba_payload::{ParseError, parse_report2, parse_report3};
use crate::domain::session_energy::{EnergySnapshot, compute_session_kwh};
use crate::domain::session_state::{Clock, SessionStateMachine, SessionTransition, TimestampMs};

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
    #[error("failed to fetch report 3: {0}")]
    FetchReport3(#[source] KebaClientError),
    #[error("failed to parse report 3: {0}")]
    ParseReport3(#[source] ParseError),
    #[error("failed to compute session energy: {0}")]
    ComputeEnergy(#[source] crate::domain::session_energy::EnergyComputationError),
    #[error("database lock poisoned")]
    DbLockPoisoned,
    #[error("database write failed: {0}")]
    Database(#[source] DbError),
}

pub struct SessionPoller<C, Cl> {
    client: C,
    clock: Cl,
    connection: Arc<Mutex<Connection>>,
    machine: SessionStateMachine,
    start_snapshot: Option<EnergySnapshot>,
    last_seconds: Option<u64>,
}

impl<C, Cl> SessionPoller<C, Cl>
where
    C: KebaClient,
    Cl: Clock,
{
    pub fn new(
        client: C,
        clock: Cl,
        connection: Arc<Mutex<Connection>>,
        debounce_samples: usize,
    ) -> Self {
        Self {
            client,
            clock,
            connection,
            machine: SessionStateMachine::new(debounce_samples),
            start_snapshot: None,
            last_seconds: None,
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

        match self.machine.observe(report2.plugged, &self.clock) {
            Some(SessionTransition::Plugged { plugged_at }) => {
                self.handle_plugged(plugged_at);
            }
            Some(SessionTransition::Unplugged {
                plugged_at,
                unplugged_at,
            }) => self.handle_unplugged(plugged_at, unplugged_at, report2_raw)?,
            None => {}
        }

        Ok(())
    }

    fn handle_plugged(&mut self, plugged_at: TimestampMs) {
        let report3_raw = match self.client.get_report3() {
            Ok(value) => value,
            Err(error) => {
                self.start_snapshot = None;
                tracing::warn!(error = %error, "failed to fetch report 3 on plugged transition");
                return;
            }
        };

        let report3 = match parse_report3(&report3_raw) {
            Ok(report3) => report3,
            Err(error) => {
                self.start_snapshot = None;
                tracing::warn!(error = %error, "failed to parse report 3 on plugged transition");
                return;
            }
        };

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
        let report3_raw = self
            .client
            .get_report3()
            .map_err(PollerError::FetchReport3)?;
        let report3 = parse_report3(&report3_raw).map_err(PollerError::ParseReport3)?;

        let end_snapshot = EnergySnapshot {
            present_session_kwh: report3.present_session_kwh,
            total_kwh: report3.total_kwh,
        };

        let energy = compute_session_kwh(self.start_snapshot.as_ref(), &end_snapshot)
            .map_err(PollerError::ComputeEnergy)?;

        let new_session = NewSessionRecord {
            plugged_at: timestamp_to_iso8601(plugged_at),
            unplugged_at: timestamp_to_iso8601(unplugged_at),
            kwh: energy.kwh,
            created_at: timestamp_to_iso8601(unplugged_at),
            raw_report2: Some(report2_raw.to_string()),
            raw_report3: Some(report3_raw.to_string()),
        };

        self.start_snapshot = None;

        let connection = self
            .connection
            .lock()
            .map_err(|_| PollerError::DbLockPoisoned)?;

        let session_id =
            insert_session(&connection, &new_session).map_err(PollerError::Database)?;

        tracing::info!(
            session_id,
            plugged_at = %new_session.plugged_at,
            unplugged_at = %new_session.unplugged_at,
            kwh = new_session.kwh,
            "charging session persisted"
        );

        Ok(())
    }
}

pub fn start_poller<C, Cl>(
    mut poller: SessionPoller<C, Cl>,
    poll_interval: Duration,
    stop_flag: Arc<AtomicBool>,
) -> JoinHandle<()>
where
    C: KebaClient,
    Cl: Clock + Send + 'static,
{
    std::thread::spawn(move || {
        while !stop_flag.load(Ordering::Relaxed) {
            if let Err(error) = poller.tick() {
                tracing::warn!(error = %error, "poll cycle failed");
            }
            std::thread::sleep(poll_interval);
        }
    })
}

pub fn run(config: AppConfig) -> Result<(), AppError> {
    let mut connection =
        crate::adapters::db::open_connection(&config.db_path).map_err(AppError::database_init)?;
    crate::adapters::db::run_migrations(&mut connection).map_err(AppError::database_init)?;

    let shared_connection = Arc::new(Mutex::new(connection));
    let api_state = ApiState {
        connection: Arc::clone(&shared_connection),
    };

    let keba_client =
        KebaUdpClient::new(&config.keba_ip, config.keba_udp_port).map_err(AppError::runtime)?;
    let poller = SessionPoller::new(
        keba_client,
        SystemClock,
        Arc::clone(&shared_connection),
        config.debounce_samples,
    );
    let stop_flag = Arc::new(AtomicBool::new(false));
    let poller_handle = start_poller(
        poller,
        Duration::from_millis(config.poll_interval_ms),
        Arc::clone(&stop_flag),
    );

    tracing::info!(bind = %config.http_bind, "http server starting");

    let server_result = actix_web::rt::System::new().block_on(async move {
        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(api_state.clone()))
                .configure(configure_routes)
        })
        .bind(&config.http_bind)?
        .run()
        .await
    });

    stop_flag.store(true, Ordering::Relaxed);
    let join_result = poller_handle.join();

    if join_result.is_err() {
        return Err(AppError::runtime("poller thread panicked"));
    }

    server_result.map_err(AppError::runtime)
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

    use crate::adapters::db::{get_latest_session, open_connection, run_migrations};
    use crate::adapters::keba_udp::KebaUdpClient;

    use super::{Clock, SessionPoller, TimestampMs};

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

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let path = dir.path().join(name);
        std::mem::forget(dir);
        path
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

        let db_path = temp_db_path("poller-runtime.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db should open");
        run_migrations(&mut connection).expect("migrations should succeed");
        let shared_connection = Arc::new(Mutex::new(connection));

        let client = KebaUdpClient::new("127.0.0.1", responder_port).expect("client should build");
        let clock = StepClock::new(vec![1_700_000_000_000, 1_700_000_060_000]);
        let mut poller = SessionPoller::new(client, clock, Arc::clone(&shared_connection), 2);

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
            assert_eq!(latest.kwh, 5.0);
            assert_eq!(latest.plugged_at, "2023-11-14T22:13:20.000Z");
            assert_eq!(latest.unplugged_at, "2023-11-14T22:14:20.000Z");
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
}
