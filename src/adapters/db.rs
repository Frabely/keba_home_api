use rusqlite::{Connection, OpenFlags, params};
use thiserror::Error;

pub use crate::domain::models::{
    LogEventRecord, NewLogEventRecord, NewSessionRecord, SessionRecord,
};
use uuid::Uuid;

pub const LATEST_SCHEMA_VERSION: u32 = 5;

const MIGRATIONS: &[(u32, &str)] = &[
    (
        1,
        r#"
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugged_at TEXT NOT NULL,
    unplugged_at TEXT NOT NULL,
    kwh REAL NOT NULL,
    created_at TEXT NOT NULL,
    raw_report2 TEXT,
    raw_report3 TEXT
);

CREATE INDEX IF NOT EXISTS idx_sessions_created_at_desc
ON sessions (created_at DESC);
"#,
    ),
    (
        2,
        r#"
CREATE TABLE IF NOT EXISTS charging_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    energy_kwh REAL NOT NULL,
    source TEXT NOT NULL,
    status TEXT NOT NULL,
    started_reason TEXT NOT NULL,
    finished_reason TEXT NOT NULL,
    poll_interval_ms INTEGER NOT NULL,
    debounce_samples INTEGER NOT NULL,
    error_count_during_session INTEGER NOT NULL,
    station_id TEXT,
    created_at TEXT NOT NULL,
    raw_report2_start TEXT,
    raw_report3_start TEXT,
    raw_report2_end TEXT,
    raw_report3_end TEXT
);

INSERT INTO charging_sessions (
    started_at,
    finished_at,
    duration_ms,
    energy_kwh,
    source,
    status,
    started_reason,
    finished_reason,
    poll_interval_ms,
    debounce_samples,
    error_count_during_session,
    station_id,
    created_at,
    raw_report2_start,
    raw_report3_start,
    raw_report2_end,
    raw_report3_end
)
SELECT
    plugged_at,
    unplugged_at,
    MAX((CAST(strftime('%s', unplugged_at) AS INTEGER) - CAST(strftime('%s', plugged_at) AS INTEGER)) * 1000, 0),
    kwh,
    'udp',
    'completed',
    'plug_state_transition',
    'plug_state_transition',
    1000,
    2,
    0,
    NULL,
    created_at,
    NULL,
    NULL,
    raw_report2,
    raw_report3
FROM sessions;

DROP TABLE sessions;

CREATE INDEX IF NOT EXISTS idx_charging_sessions_created_at_desc
ON charging_sessions (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_charging_sessions_station_created_at_desc
ON charging_sessions (station_id, created_at DESC);
"#,
    ),
    (
        3,
        r#"
CREATE TABLE IF NOT EXISTS log_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    level TEXT NOT NULL,
    code TEXT NOT NULL,
    message TEXT NOT NULL,
    source TEXT NOT NULL,
    station_id TEXT,
    details_json TEXT
);

CREATE INDEX IF NOT EXISTS idx_log_events_created_at_desc
ON log_events (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_log_events_code_created_at_desc
ON log_events (code, created_at DESC);

CREATE TABLE IF NOT EXISTS charging_session_log_events (
    session_id INTEGER NOT NULL,
    log_event_id INTEGER NOT NULL,
    PRIMARY KEY (session_id, log_event_id),
    FOREIGN KEY (session_id) REFERENCES charging_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY (log_event_id) REFERENCES log_events(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_charging_session_log_events_log_event
ON charging_session_log_events (log_event_id);
"#,
    ),
    (
        4,
        r#"
CREATE TABLE IF NOT EXISTS charging_sessions_v2 (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    energy_kwh REAL NOT NULL,
    source TEXT NOT NULL,
    status TEXT NOT NULL,
    started_reason TEXT NOT NULL,
    finished_reason TEXT NOT NULL,
    poll_interval_ms INTEGER NOT NULL,
    debounce_samples INTEGER NOT NULL,
    error_count_during_session INTEGER NOT NULL,
    station_id TEXT,
    created_at TEXT NOT NULL,
    raw_report2_start TEXT,
    raw_report3_start TEXT,
    raw_report2_end TEXT,
    raw_report3_end TEXT
);

INSERT INTO charging_sessions_v2 (
    id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
    poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
    raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
)
SELECT
    'legacy-session-' || id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
    poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
    raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
FROM charging_sessions;

CREATE TABLE IF NOT EXISTS log_events_v2 (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    level TEXT NOT NULL,
    code TEXT NOT NULL,
    message TEXT NOT NULL,
    source TEXT NOT NULL,
    station_id TEXT,
    details_json TEXT
);

INSERT INTO log_events_v2 (id, created_at, level, code, message, source, station_id, details_json)
SELECT
    'legacy-log-' || id, created_at, level, code, message, source, station_id, details_json
FROM log_events;

CREATE TABLE IF NOT EXISTS charging_session_log_events_v2 (
    session_id TEXT NOT NULL,
    log_event_id TEXT NOT NULL,
    PRIMARY KEY (session_id, log_event_id),
    FOREIGN KEY (session_id) REFERENCES charging_sessions_v2(id) ON DELETE CASCADE,
    FOREIGN KEY (log_event_id) REFERENCES log_events_v2(id) ON DELETE CASCADE
);

INSERT INTO charging_session_log_events_v2 (session_id, log_event_id)
SELECT
    'legacy-session-' || session_id,
    'legacy-log-' || log_event_id
FROM charging_session_log_events;

DROP TABLE charging_session_log_events;
DROP TABLE log_events;
DROP TABLE charging_sessions;

ALTER TABLE charging_sessions_v2 RENAME TO charging_sessions;
ALTER TABLE log_events_v2 RENAME TO log_events;
ALTER TABLE charging_session_log_events_v2 RENAME TO charging_session_log_events;

CREATE INDEX IF NOT EXISTS idx_charging_sessions_created_at_desc
ON charging_sessions (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_charging_sessions_station_created_at_desc
ON charging_sessions (station_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_log_events_created_at_desc
ON log_events (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_log_events_code_created_at_desc
ON log_events (code, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_charging_session_log_events_log_event
ON charging_session_log_events (log_event_id);
"#,
    ),
    (
        5,
        r#"
CREATE TABLE IF NOT EXISTS charging_sessions_v3 (
    id TEXT PRIMARY KEY,
    started_at TEXT,
    finished_at TEXT NOT NULL,
    duration_ms INTEGER NOT NULL,
    energy_kwh REAL NOT NULL,
    source TEXT NOT NULL,
    status TEXT NOT NULL,
    started_reason TEXT NOT NULL,
    finished_reason TEXT NOT NULL,
    poll_interval_ms INTEGER NOT NULL,
    debounce_samples INTEGER NOT NULL,
    error_count_during_session INTEGER NOT NULL,
    station_id TEXT,
    created_at TEXT NOT NULL,
    raw_report2_start TEXT,
    raw_report3_start TEXT,
    raw_report2_end TEXT,
    raw_report3_end TEXT
);

INSERT INTO charging_sessions_v3 (
    id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
    poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
    raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
)
SELECT
    id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
    poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
    raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
FROM charging_sessions;

DROP TABLE charging_sessions;
ALTER TABLE charging_sessions_v3 RENAME TO charging_sessions;

CREATE INDEX IF NOT EXISTS idx_charging_sessions_created_at_desc
ON charging_sessions (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_charging_sessions_station_created_at_desc
ON charging_sessions (station_id, created_at DESC);
"#,
    ),
];

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unsupported schema version {current}; latest supported is {latest}")]
    UnsupportedSchemaVersion { current: u32, latest: u32 },
}

pub fn open_connection(path: &str) -> Result<Connection, DbError> {
    let connection = Connection::open(path).map_err(DbError::from)?;
    configure_writer_connection_pragmas(&connection)?;
    Ok(connection)
}

pub fn open_read_only_connection(path: &str) -> Result<Connection, DbError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(DbError::from)?;
    configure_reader_connection_pragmas(&connection)?;
    Ok(connection)
}

fn configure_writer_connection_pragmas(connection: &Connection) -> Result<(), DbError> {
    connection
        .execute_batch(
            r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
"#,
        )
        .map_err(DbError::from)?;
    Ok(())
}

fn configure_reader_connection_pragmas(connection: &Connection) -> Result<(), DbError> {
    connection
        .execute_batch(
            r#"
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA query_only = ON;
"#,
        )
        .map_err(DbError::from)?;
    Ok(())
}

pub fn run_migrations(connection: &mut Connection) -> Result<(), DbError> {
    let current_version = schema_version(connection)?;

    if current_version > LATEST_SCHEMA_VERSION {
        return Err(DbError::UnsupportedSchemaVersion {
            current: current_version,
            latest: LATEST_SCHEMA_VERSION,
        });
    }

    let transaction = connection.transaction()?;

    for (version, sql) in MIGRATIONS {
        if *version > current_version {
            transaction.execute_batch(sql)?;
            transaction.pragma_update(None, "user_version", version)?;
        }
    }

    transaction.commit()?;

    Ok(())
}

pub fn schema_version(connection: &Connection) -> Result<u32, DbError> {
    let version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version)
}

pub fn insert_session(
    connection: &Connection,
    new_session: &NewSessionRecord,
) -> Result<String, DbError> {
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO charging_sessions (
            id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
            poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
            raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            id,
            new_session.started_at,
            new_session.finished_at,
            new_session.duration_ms,
            new_session.energy_kwh,
            new_session.source,
            new_session.status,
            new_session.started_reason,
            new_session.finished_reason,
            new_session.poll_interval_ms,
            new_session.debounce_samples,
            new_session.error_count_during_session,
            new_session.station_id,
            new_session.created_at,
            new_session.raw_report2_start,
            new_session.raw_report3_start,
            new_session.raw_report2_end,
            new_session.raw_report3_end,
        ],
    )?;

    Ok(id)
}

pub fn insert_log_event(
    connection: &Connection,
    new_log_event: &NewLogEventRecord,
) -> Result<String, DbError> {
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO log_events (
            id, created_at, level, code, message, source, station_id, details_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            new_log_event.created_at,
            new_log_event.level,
            new_log_event.code,
            new_log_event.message,
            new_log_event.source,
            new_log_event.station_id,
            new_log_event.details_json,
        ],
    )?;

    Ok(id)
}

pub fn link_session_log_events(
    connection: &Connection,
    session_id: &str,
    log_event_ids: &[String],
) -> Result<(), DbError> {
    if log_event_ids.is_empty() {
        return Ok(());
    }

    let mut statement = connection.prepare(
        "INSERT OR IGNORE INTO charging_session_log_events (session_id, log_event_id) VALUES (?1, ?2)",
    )?;
    for log_event_id in log_event_ids {
        statement.execute(params![session_id, log_event_id])?;
    }
    Ok(())
}

pub fn count_log_events(connection: &Connection) -> Result<i64, DbError> {
    let count = connection.query_row("SELECT COUNT(*) FROM log_events", [], |row| row.get(0))?;
    Ok(count)
}

pub fn count_sessions(connection: &Connection) -> Result<i64, DbError> {
    let count = connection.query_row("SELECT COUNT(*) FROM charging_sessions", [], |row| {
        row.get(0)
    })?;
    Ok(count)
}

pub fn count_session_log_events(connection: &Connection, session_id: &str) -> Result<i64, DbError> {
    let count = connection.query_row(
        "SELECT COUNT(*) FROM charging_session_log_events WHERE session_id = ?1",
        params![session_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn list_recent_log_events(
    connection: &Connection,
    limit: u32,
) -> Result<Vec<LogEventRecord>, DbError> {
    let mut statement = connection.prepare(
        "SELECT id, created_at, level, code, message, source, station_id, details_json
         FROM log_events
         ORDER BY created_at DESC, id DESC
         LIMIT ?1",
    )?;

    let rows = statement.query_map(params![i64::from(limit)], |row| {
        Ok(LogEventRecord {
            id: row.get(0)?,
            created_at: row.get(1)?,
            level: row.get(2)?,
            code: row.get(3)?,
            message: row.get(4)?,
            source: row.get(5)?,
            station_id: row.get(6)?,
            details_json: row.get(7)?,
        })
    })?;

    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }

    Ok(events)
}

pub fn get_latest_session(connection: &Connection) -> Result<Option<SessionRecord>, DbError> {
    let mut statement = connection.prepare(
        "SELECT id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
                poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
                raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
         FROM charging_sessions
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )?;

    let mut rows = statement.query([])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(SessionRecord {
            id: row.get(0)?,
            started_at: row.get(1)?,
            finished_at: row.get(2)?,
            duration_ms: row.get(3)?,
            energy_kwh: row.get(4)?,
            source: row.get(5)?,
            status: row.get(6)?,
            started_reason: row.get(7)?,
            finished_reason: row.get(8)?,
            poll_interval_ms: row.get(9)?,
            debounce_samples: row.get(10)?,
            error_count_during_session: row.get(11)?,
            station_id: row.get(12)?,
            created_at: row.get(13)?,
            raw_report2_start: row.get(14)?,
            raw_report3_start: row.get(15)?,
            raw_report2_end: row.get(16)?,
            raw_report3_end: row.get(17)?,
        }));
    }

    Ok(None)
}

pub fn get_latest_session_since(
    connection: &Connection,
    since_inclusive: &str,
) -> Result<Option<SessionRecord>, DbError> {
    let mut statement = connection.prepare(
        "SELECT id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
                poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
                raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
         FROM charging_sessions
         WHERE created_at >= ?1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )?;

    let mut rows = statement.query(params![since_inclusive])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(SessionRecord {
            id: row.get(0)?,
            started_at: row.get(1)?,
            finished_at: row.get(2)?,
            duration_ms: row.get(3)?,
            energy_kwh: row.get(4)?,
            source: row.get(5)?,
            status: row.get(6)?,
            started_reason: row.get(7)?,
            finished_reason: row.get(8)?,
            poll_interval_ms: row.get(9)?,
            debounce_samples: row.get(10)?,
            error_count_during_session: row.get(11)?,
            station_id: row.get(12)?,
            created_at: row.get(13)?,
            raw_report2_start: row.get(14)?,
            raw_report3_start: row.get(15)?,
            raw_report2_end: row.get(16)?,
            raw_report3_end: row.get(17)?,
        }));
    }

    Ok(None)
}

pub fn list_sessions(
    connection: &Connection,
    limit: u32,
    offset: u32,
) -> Result<Vec<SessionRecord>, DbError> {
    let mut statement = connection.prepare(
        "SELECT id, started_at, finished_at, duration_ms, energy_kwh, source, status, started_reason, finished_reason,
                poll_interval_ms, debounce_samples, error_count_during_session, station_id, created_at,
                raw_report2_start, raw_report3_start, raw_report2_end, raw_report3_end
         FROM charging_sessions
         ORDER BY created_at DESC, id DESC
         LIMIT ?1 OFFSET ?2",
    )?;

    let rows = statement.query_map(params![i64::from(limit), i64::from(offset)], |row| {
        Ok(SessionRecord {
            id: row.get(0)?,
            started_at: row.get(1)?,
            finished_at: row.get(2)?,
            duration_ms: row.get(3)?,
            energy_kwh: row.get(4)?,
            source: row.get(5)?,
            status: row.get(6)?,
            started_reason: row.get(7)?,
            finished_reason: row.get(8)?,
            poll_interval_ms: row.get(9)?,
            debounce_samples: row.get(10)?,
            error_count_during_session: row.get(11)?,
            station_id: row.get(12)?,
            created_at: row.get(13)?,
            raw_report2_start: row.get(14)?,
            raw_report3_start: row.get(15)?,
            raw_report2_end: row.get(16)?,
            raw_report3_end: row.get(17)?,
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }

    Ok(sessions)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rusqlite::params;

    use super::{
        LATEST_SCHEMA_VERSION, NewLogEventRecord, NewSessionRecord, count_log_events,
        count_session_log_events, get_latest_session, get_latest_session_since, insert_log_event,
        insert_session, link_session_log_events, list_sessions, open_connection, run_migrations,
        schema_version,
    };

    fn temp_db_path(name: &str) -> PathBuf {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let path = dir.path().join(name);
        std::mem::forget(dir);
        path
    }

    fn sample_new_session(
        started_at: Option<&str>,
        finished_at: &str,
        created_at: &str,
        energy_kwh: f64,
    ) -> NewSessionRecord {
        let started_ms = started_at.map(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .expect("started_at should parse")
                .timestamp_millis()
        });
        let finished_ms = chrono::DateTime::parse_from_rfc3339(finished_at)
            .expect("finished_at should parse")
            .timestamp_millis();

        NewSessionRecord {
            started_at: started_at.map(ToString::to_string),
            finished_at: finished_at.to_string(),
            duration_ms: started_ms.map_or(0, |value| (finished_ms - value).max(0)),
            energy_kwh,
            source: "debug_file".to_string(),
            status: "completed".to_string(),
            started_reason: "plug_state_transition".to_string(),
            finished_reason: "plug_state_transition".to_string(),
            poll_interval_ms: 1000,
            debounce_samples: 2,
            error_count_during_session: 0,
            station_id: Some("station-a".to_string()),
            created_at: created_at.to_string(),
            raw_report2_start: Some("{\"Plug\":7}".to_string()),
            raw_report3_start: Some("{\"E pres\":0}".to_string()),
            raw_report2_end: Some("{\"Plug\":0}".to_string()),
            raw_report3_end: Some("{\"E pres\":10830}".to_string()),
        }
    }

    #[test]
    fn migrates_fresh_database_to_latest_version() {
        let db_path = temp_db_path("fresh.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");

        run_migrations(&mut connection).expect("migrations should succeed");

        let version = schema_version(&connection).expect("schema version should be queryable");
        assert_eq!(version, LATEST_SCHEMA_VERSION);

        let table_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='charging_sessions'",
                [],
                |row| row.get(0),
            )
            .expect("charging_sessions table check should work");
        assert_eq!(table_exists, 1);

        let log_events_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='log_events'",
                [],
                |row| row.get(0),
            )
            .expect("log_events table check should work");
        assert_eq!(log_events_exists, 1);

        let session_log_events_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='charging_session_log_events'",
                [],
                |row| row.get(0),
            )
            .expect("charging_session_log_events table check should work");
        assert_eq!(session_log_events_exists, 1);

        let old_table_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions'",
                [],
                |row| row.get(0),
            )
            .expect("sessions table check should work");
        assert_eq!(old_table_exists, 0);

        let started_at_notnull: i64 = connection
            .query_row(
                "SELECT \"notnull\" FROM pragma_table_info('charging_sessions') WHERE name = 'started_at'",
                [],
                |row| row.get(0),
            )
            .expect("started_at column metadata query should succeed");
        assert_eq!(started_at_notnull, 0);

        let index_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_charging_sessions_created_at_desc'",
                [],
                |row| row.get(0),
            )
            .expect("charging_sessions index check should work");
        assert_eq!(index_exists, 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let db_path = temp_db_path("idempotent.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");

        run_migrations(&mut connection).expect("first migration run should succeed");
        run_migrations(&mut connection).expect("second migration run should succeed");

        let version = schema_version(&connection).expect("schema version should be queryable");
        assert_eq!(version, LATEST_SCHEMA_VERSION);
    }

    #[test]
    fn keeps_existing_data_when_migrations_rerun() {
        let db_path = temp_db_path("rerun.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");

        connection
            .execute_batch(
                r#"
                PRAGMA user_version = 1;
                CREATE TABLE IF NOT EXISTS sessions (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    plugged_at TEXT NOT NULL,
                    unplugged_at TEXT NOT NULL,
                    kwh REAL NOT NULL,
                    created_at TEXT NOT NULL,
                    raw_report2 TEXT,
                    raw_report3 TEXT
                );
                "#,
            )
            .expect("legacy schema setup should succeed");
        connection
            .execute(
                "INSERT INTO sessions (plugged_at, unplugged_at, kwh, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    "2026-02-20T18:12:03.120Z",
                    "2026-02-20T22:45:10.002Z",
                    10.83_f64,
                    "2026-02-20T22:45:10.002Z"
                ],
            )
            .expect("insert should succeed");

        run_migrations(&mut connection).expect("migration run should succeed");
        run_migrations(&mut connection).expect("rerun migration should succeed");

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM charging_sessions", [], |row| {
                row.get(0)
            })
            .expect("count query should succeed");
        assert_eq!(count, 1);
    }

    #[test]
    fn returns_none_for_latest_session_when_empty() {
        let db_path = temp_db_path("latest-empty.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        let latest = get_latest_session(&connection).expect("query should succeed");
        assert_eq!(latest, None);
    }

    #[test]
    fn inserts_and_reads_latest_session() {
        let db_path = temp_db_path("latest.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        let inserted_id = insert_session(
            &connection,
            &sample_new_session(
                Some("2026-02-20T18:12:03.120Z"),
                "2026-02-20T22:45:10.002Z",
                "2026-02-20T22:45:10.002Z",
                10.83,
            ),
        )
        .expect("insert should succeed");

        let latest = get_latest_session(&connection)
            .expect("query should succeed")
            .expect("session should exist");

        assert_eq!(latest.id, inserted_id);
        assert_eq!(latest.energy_kwh, 10.83);
        assert_eq!(latest.status, "completed");
        assert_eq!(latest.raw_report2_start.as_deref(), Some("{\"Plug\":7}"));
        assert_eq!(latest.raw_report2_end.as_deref(), Some("{\"Plug\":0}"));
    }

    #[test]
    fn inserts_and_reads_latest_session_with_null_started_at() {
        let db_path = temp_db_path("latest-null-started-at.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        insert_session(
            &connection,
            &sample_new_session(
                None,
                "2026-02-20T22:45:10.002Z",
                "2026-02-20T22:45:10.002Z",
                10.83,
            ),
        )
        .expect("insert should succeed");

        let latest = get_latest_session(&connection)
            .expect("query should succeed")
            .expect("session should exist");

        assert_eq!(latest.started_at, None);
    }

    #[test]
    fn lists_sessions_with_limit_and_offset() {
        let db_path = temp_db_path("list.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        let sessions = [
            sample_new_session(
                Some("2026-02-20T10:00:00.000Z"),
                "2026-02-20T11:00:00.000Z",
                "2026-02-20T11:00:00.000Z",
                5.0,
            ),
            sample_new_session(
                Some("2026-02-21T10:00:00.000Z"),
                "2026-02-21T11:00:00.000Z",
                "2026-02-21T11:00:00.000Z",
                6.0,
            ),
            sample_new_session(
                Some("2026-02-22T10:00:00.000Z"),
                "2026-02-22T11:00:00.000Z",
                "2026-02-22T11:00:00.000Z",
                7.0,
            ),
        ];

        for session in sessions {
            insert_session(&connection, &session).expect("insert should succeed");
        }

        let page = list_sessions(&connection, 2, 1).expect("query should succeed");

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].energy_kwh, 6.0);
        assert_eq!(page[1].energy_kwh, 5.0);
    }

    #[test]
    fn returns_latest_session_since_threshold() {
        let db_path = temp_db_path("latest-since.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        insert_session(
            &connection,
            &sample_new_session(
                Some("2026-02-20T10:00:00.000Z"),
                "2026-02-20T11:00:00.000Z",
                "2026-02-20T11:00:00.000Z",
                5.0,
            ),
        )
        .expect("insert should succeed");
        insert_session(
            &connection,
            &sample_new_session(
                Some("2026-02-20T11:30:00.000Z"),
                "2026-02-20T11:35:00.000Z",
                "2026-02-20T11:35:00.000Z",
                2.0,
            ),
        )
        .expect("insert should succeed");

        let found = get_latest_session_since(&connection, "2026-02-20T11:34:59.000Z")
            .expect("query should succeed")
            .expect("latest recent session should exist");
        assert_eq!(found.energy_kwh, 2.0);

        let not_found = get_latest_session_since(&connection, "2026-02-20T11:35:01.000Z")
            .expect("query should succeed");
        assert_eq!(not_found, None);
    }

    #[test]
    fn inserts_log_events_and_links_them_to_session() {
        let db_path = temp_db_path("logs-linking.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        let session_id = insert_session(
            &connection,
            &sample_new_session(
                Some("2026-02-20T10:00:00.000Z"),
                "2026-02-20T11:00:00.000Z",
                "2026-02-20T11:00:00.000Z",
                5.0,
            ),
        )
        .expect("insert session should succeed");

        let first_log_id = insert_log_event(
            &connection,
            &NewLogEventRecord {
                created_at: "2026-02-20T10:10:00.000Z".to_string(),
                level: "warn".to_string(),
                code: "poll.fetch_report2".to_string(),
                message: "failed to fetch report 2".to_string(),
                source: "debug_file".to_string(),
                station_id: Some("station-a".to_string()),
                details_json: Some("{\"attempt\":1}".to_string()),
            },
        )
        .expect("insert log event should succeed");
        let second_log_id = insert_log_event(
            &connection,
            &NewLogEventRecord {
                created_at: "2026-02-20T10:10:01.000Z".to_string(),
                level: "warn".to_string(),
                code: "poll.parse_report3".to_string(),
                message: "failed to parse report 3".to_string(),
                source: "debug_file".to_string(),
                station_id: Some("station-a".to_string()),
                details_json: None,
            },
        )
        .expect("insert second log event should succeed");

        link_session_log_events(&connection, &session_id, &[first_log_id, second_log_id])
            .expect("linking should succeed");

        assert_eq!(
            count_log_events(&connection).expect("count should succeed"),
            2
        );
        assert_eq!(
            count_session_log_events(&connection, &session_id).expect("count should succeed"),
            2
        );
    }
}
