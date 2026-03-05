use rusqlite::{Connection, OpenFlags, params};
use thiserror::Error;

pub use crate::domain::models::{
    LogEventRecord, NewLogEventRecord, NewSessionRecord, NewUnplugLogRecord, SessionRecord,
    UnplugLogRecord,
};
use uuid::Uuid;

pub const LATEST_SCHEMA_VERSION: u32 = 9;

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
    (
        6,
        r#"
CREATE TABLE IF NOT EXISTS unplug_log_events (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL,
    station TEXT NOT NULL,
    started TEXT NOT NULL,
    ended TEXT NOT NULL,
    kwh TEXT NOT NULL,
    card_id TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_unplug_log_events_timestamp_desc
ON unplug_log_events (timestamp DESC, id DESC);
"#,
    ),
    (
        7,
        r#"
DROP TABLE IF EXISTS sessions;
DROP TABLE IF EXISTS charging_sessions_v2;
DROP TABLE IF EXISTS charging_sessions_v3;
DROP TABLE IF EXISTS log_events_v2;
DROP TABLE IF EXISTS charging_session_log_events_v2;
"#,
    ),
    (
        8,
        r#"
DROP TABLE IF EXISTS charging_session_log_events;
DROP TABLE IF EXISTS log_events;
DROP TABLE IF EXISTS charging_sessions;
"#,
    ),
    (
        9,
        r#"
CREATE TABLE IF NOT EXISTS unplug_log_events_v2 (
    Id TEXT PRIMARY KEY,
    Timestamp TEXT NOT NULL,
    Station TEXT NOT NULL,
    Started TEXT NOT NULL,
    Ended TEXT NOT NULL,
    Wh TEXT NOT NULL,
    CardId TEXT NOT NULL
);

INSERT INTO unplug_log_events_v2 (Id, Timestamp, Station, Started, Ended, Wh, CardId)
SELECT
    id,
    timestamp,
    station,
    started,
    ended,
    CASE
        WHEN lower(trim(kwh)) = 'n/a' THEN 'n/a'
        ELSE printf('%.1f', CAST(replace(kwh, ',', '.') AS REAL) * 1000.0)
    END,
    card_id
FROM unplug_log_events;

DROP TABLE unplug_log_events;
ALTER TABLE unplug_log_events_v2 RENAME TO unplug_log_events;

CREATE INDEX IF NOT EXISTS idx_unplug_log_events_timestamp_desc
ON unplug_log_events (Timestamp DESC, Id DESC);
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

pub fn insert_unplug_log_event(
    connection: &Connection,
    new_event: &NewUnplugLogRecord,
) -> Result<String, DbError> {
    let id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO unplug_log_events (
            Id, Timestamp, Station, Started, Ended, Wh, CardId
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            new_event.timestamp,
            new_event.station,
            new_event.started,
            new_event.ended,
            new_event.wh,
            new_event.card_id,
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

pub fn list_recent_unplug_log_events(
    connection: &Connection,
    limit: u32,
) -> Result<Vec<UnplugLogRecord>, DbError> {
    let mut statement = connection.prepare(
        "SELECT Id, Timestamp, Station, Started, Ended, Wh, CardId
         FROM unplug_log_events
         ORDER BY Timestamp DESC, Id DESC
         LIMIT ?1",
    )?;

    let rows = statement.query_map(params![i64::from(limit)], |row| {
        Ok(UnplugLogRecord {
            id: row.get(0)?,
            timestamp: row.get(1)?,
            station: row.get(2)?,
            started: row.get(3)?,
            ended: row.get(4)?,
            wh: row.get(5)?,
            card_id: row.get(6)?,
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
        LATEST_SCHEMA_VERSION, NewUnplugLogRecord, insert_unplug_log_event, open_connection,
        run_migrations, schema_version,
    };

    fn temp_db_path(name: &str) -> PathBuf {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let path = dir.path().join(name);
        std::mem::forget(dir);
        path
    }

    #[test]
    fn migrates_fresh_database_to_latest_version_with_unplug_table_only() {
        let db_path = temp_db_path("fresh.sqlite");
        let mut connection = open_connection(db_path.to_str().expect("db path should be utf8"))
            .expect("connection should open");

        run_migrations(&mut connection).expect("migrations should succeed");
        let version = schema_version(&connection).expect("version should be readable");
        assert_eq!(version, LATEST_SCHEMA_VERSION);

        let unplug_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='unplug_log_events'",
                [],
                |row| row.get(0),
            )
            .expect("unplug_log_events table check should work");
        assert_eq!(unplug_exists, 1);

        let charging_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='charging_sessions'",
                [],
                |row| row.get(0),
            )
            .expect("charging_sessions table check should work");
        assert_eq!(charging_exists, 0);

        let log_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='log_events'",
                [],
                |row| row.get(0),
            )
            .expect("log_events table check should work");
        assert_eq!(log_exists, 0);
    }

    #[test]
    fn migrations_are_idempotent() {
        let db_path = temp_db_path("idempotent.sqlite");
        let mut connection = open_connection(db_path.to_str().expect("db path should be utf8"))
            .expect("connection should open");

        run_migrations(&mut connection).expect("first migration run should succeed");
        run_migrations(&mut connection).expect("second migration run should succeed");

        let version = schema_version(&connection).expect("version should be readable");
        assert_eq!(version, LATEST_SCHEMA_VERSION);
    }

    #[test]
    fn inserts_unplug_log_event() {
        let db_path = temp_db_path("unplug.sqlite");
        let mut connection = open_connection(db_path.to_str().expect("db path should be utf8"))
            .expect("connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        let event = NewUnplugLogRecord {
            timestamp: "2026-02-20 21:49".to_string(),
            station: "Carport".to_string(),
            started: "2026-02-20 20:10".to_string(),
            ended: "2026-02-20 21:49".to_string(),
            wh: "12.3".to_string(),
            card_id: "ABC123".to_string(),
        };

        let id = insert_unplug_log_event(&connection, &event).expect("insert should succeed");

        let row: (String, String, String, String, String, String, String) = connection
            .query_row(
                "SELECT Id, Timestamp, Station, Started, Ended, Wh, CardId FROM unplug_log_events WHERE Id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .expect("inserted event should be readable");

        assert_eq!(row.1, event.timestamp);
        assert_eq!(row.2, event.station);
        assert_eq!(row.3, event.started);
        assert_eq!(row.4, event.ended);
        assert_eq!(row.5, event.wh);
        assert_eq!(row.6, event.card_id);
    }
}
