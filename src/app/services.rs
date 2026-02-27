use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use thiserror::Error;

use crate::adapters::db;
use crate::adapters::db::DbError;
use crate::domain::models::{LogEventRecord, NewLogEventRecord, NewSessionRecord, SessionRecord};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("database lock poisoned")]
    DbLockPoisoned,
    #[error("database operation failed: {0}")]
    Database(#[from] DbError),
}

pub trait SessionQueryHandler {
    fn get_latest_session(&self) -> Result<Option<SessionRecord>, ServiceError>;
    fn get_latest_session_since(
        &self,
        since_inclusive: &str,
    ) -> Result<Option<SessionRecord>, ServiceError>;
    fn list_sessions(&self, limit: u32, offset: u32) -> Result<Vec<SessionRecord>, ServiceError>;
    fn get_schema_version(&self) -> Result<u32, ServiceError>;
    fn count_sessions(&self) -> Result<i64, ServiceError>;
    fn count_log_events(&self) -> Result<i64, ServiceError>;
    fn list_recent_log_events(&self, limit: u32) -> Result<Vec<LogEventRecord>, ServiceError>;
}

pub trait SessionCommandHandler {
    fn insert_session(&self, new_session: &NewSessionRecord) -> Result<String, ServiceError>;
    fn insert_log_event(&self, new_log_event: &NewLogEventRecord) -> Result<String, ServiceError>;
    fn link_session_log_events(
        &self,
        session_id: &str,
        log_event_ids: &[String],
    ) -> Result<(), ServiceError>;
}

#[derive(Clone)]
pub struct SqliteSessionService {
    connection: Arc<Mutex<Connection>>,
}

impl SqliteSessionService {
    pub fn new(connection: Arc<Mutex<Connection>>) -> Self {
        Self { connection }
    }

    fn with_connection<T>(
        &self,
        op: impl FnOnce(&Connection) -> Result<T, DbError>,
    ) -> Result<T, ServiceError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ServiceError::DbLockPoisoned)?;
        op(&connection).map_err(ServiceError::from)
    }
}

impl SessionQueryHandler for SqliteSessionService {
    fn get_latest_session(&self) -> Result<Option<SessionRecord>, ServiceError> {
        self.with_connection(db::get_latest_session)
    }

    fn get_latest_session_since(
        &self,
        since_inclusive: &str,
    ) -> Result<Option<SessionRecord>, ServiceError> {
        self.with_connection(|connection| db::get_latest_session_since(connection, since_inclusive))
    }

    fn list_sessions(&self, limit: u32, offset: u32) -> Result<Vec<SessionRecord>, ServiceError> {
        self.with_connection(|connection| db::list_sessions(connection, limit, offset))
    }

    fn get_schema_version(&self) -> Result<u32, ServiceError> {
        self.with_connection(db::schema_version)
    }

    fn count_sessions(&self) -> Result<i64, ServiceError> {
        self.with_connection(db::count_sessions)
    }

    fn count_log_events(&self) -> Result<i64, ServiceError> {
        self.with_connection(db::count_log_events)
    }

    fn list_recent_log_events(&self, limit: u32) -> Result<Vec<LogEventRecord>, ServiceError> {
        self.with_connection(|connection| db::list_recent_log_events(connection, limit))
    }
}

impl SessionCommandHandler for SqliteSessionService {
    fn insert_session(&self, new_session: &NewSessionRecord) -> Result<String, ServiceError> {
        self.with_connection(|connection| db::insert_session(connection, new_session))
    }

    fn insert_log_event(&self, new_log_event: &NewLogEventRecord) -> Result<String, ServiceError> {
        self.with_connection(|connection| db::insert_log_event(connection, new_log_event))
    }

    fn link_session_log_events(
        &self,
        session_id: &str,
        log_event_ids: &[String],
    ) -> Result<(), ServiceError> {
        self.with_connection(|connection| {
            db::link_session_log_events(connection, session_id, log_event_ids)
        })
    }
}
