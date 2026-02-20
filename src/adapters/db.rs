use rusqlite::{Connection, params};
use thiserror::Error;

pub const LATEST_SCHEMA_VERSION: u32 = 1;

const MIGRATIONS: &[(u32, &str)] = &[(
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
)];

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unsupported schema version {current}; latest supported is {latest}")]
    UnsupportedSchemaVersion { current: u32, latest: u32 },
}

pub fn open_connection(path: &str) -> Result<Connection, DbError> {
    Connection::open(path).map_err(DbError::from)
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

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub id: i64,
    pub plugged_at: String,
    pub unplugged_at: String,
    pub kwh: f64,
    pub created_at: String,
    pub raw_report2: Option<String>,
    pub raw_report3: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSessionRecord {
    pub plugged_at: String,
    pub unplugged_at: String,
    pub kwh: f64,
    pub created_at: String,
    pub raw_report2: Option<String>,
    pub raw_report3: Option<String>,
}

pub fn insert_session(
    connection: &Connection,
    new_session: &NewSessionRecord,
) -> Result<i64, DbError> {
    connection.execute(
        "INSERT INTO sessions (plugged_at, unplugged_at, kwh, created_at, raw_report2, raw_report3) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            new_session.plugged_at,
            new_session.unplugged_at,
            new_session.kwh,
            new_session.created_at,
            new_session.raw_report2,
            new_session.raw_report3,
        ],
    )?;

    Ok(connection.last_insert_rowid())
}

pub fn get_latest_session(connection: &Connection) -> Result<Option<SessionRecord>, DbError> {
    let mut statement = connection.prepare(
        "SELECT id, plugged_at, unplugged_at, kwh, created_at, raw_report2, raw_report3
         FROM sessions
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )?;

    let mut rows = statement.query([])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(SessionRecord {
            id: row.get(0)?,
            plugged_at: row.get(1)?,
            unplugged_at: row.get(2)?,
            kwh: row.get(3)?,
            created_at: row.get(4)?,
            raw_report2: row.get(5)?,
            raw_report3: row.get(6)?,
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
        "SELECT id, plugged_at, unplugged_at, kwh, created_at, raw_report2, raw_report3
         FROM sessions
         ORDER BY created_at DESC, id DESC
         LIMIT ?1 OFFSET ?2",
    )?;

    let rows = statement.query_map(params![i64::from(limit), i64::from(offset)], |row| {
        Ok(SessionRecord {
            id: row.get(0)?,
            plugged_at: row.get(1)?,
            unplugged_at: row.get(2)?,
            kwh: row.get(3)?,
            created_at: row.get(4)?,
            raw_report2: row.get(5)?,
            raw_report3: row.get(6)?,
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
        LATEST_SCHEMA_VERSION, NewSessionRecord, get_latest_session, insert_session, list_sessions,
        open_connection, run_migrations, schema_version,
    };

    fn temp_db_path(name: &str) -> PathBuf {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let path = dir.path().join(name);
        std::mem::forget(dir);
        path
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
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions'",
                [],
                |row| row.get(0),
            )
            .expect("sessions table check should work");
        assert_eq!(table_exists, 1);

        let index_exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_sessions_created_at_desc'",
                [],
                |row| row.get(0),
            )
            .expect("sessions index check should work");
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

        run_migrations(&mut connection).expect("first migration run should succeed");

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

        run_migrations(&mut connection).expect("second migration run should succeed");

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
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
            &NewSessionRecord {
                plugged_at: "2026-02-20T18:12:03.120Z".to_string(),
                unplugged_at: "2026-02-20T22:45:10.002Z".to_string(),
                kwh: 10.83,
                created_at: "2026-02-20T22:45:10.002Z".to_string(),
                raw_report2: Some("{\"Plug\":7}".to_string()),
                raw_report3: Some("{\"E pres\":10830}".to_string()),
            },
        )
        .expect("insert should succeed");

        let latest = get_latest_session(&connection)
            .expect("query should succeed")
            .expect("session should exist");

        assert_eq!(latest.id, inserted_id);
        assert_eq!(latest.kwh, 10.83);
        assert_eq!(latest.raw_report2.as_deref(), Some("{\"Plug\":7}"));
    }

    #[test]
    fn lists_sessions_with_limit_and_offset() {
        let db_path = temp_db_path("list.sqlite");
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db connection should open");
        run_migrations(&mut connection).expect("migrations should succeed");

        let sessions = [
            NewSessionRecord {
                plugged_at: "2026-02-20T10:00:00.000Z".to_string(),
                unplugged_at: "2026-02-20T11:00:00.000Z".to_string(),
                kwh: 5.0,
                created_at: "2026-02-20T11:00:00.000Z".to_string(),
                raw_report2: None,
                raw_report3: None,
            },
            NewSessionRecord {
                plugged_at: "2026-02-21T10:00:00.000Z".to_string(),
                unplugged_at: "2026-02-21T11:00:00.000Z".to_string(),
                kwh: 6.0,
                created_at: "2026-02-21T11:00:00.000Z".to_string(),
                raw_report2: None,
                raw_report3: None,
            },
            NewSessionRecord {
                plugged_at: "2026-02-22T10:00:00.000Z".to_string(),
                unplugged_at: "2026-02-22T11:00:00.000Z".to_string(),
                kwh: 7.0,
                created_at: "2026-02-22T11:00:00.000Z".to_string(),
                raw_report2: None,
                raw_report3: None,
            },
        ];

        for session in sessions {
            insert_session(&connection, &session).expect("insert should succeed");
        }

        let page = list_sessions(&connection, 2, 1).expect("query should succeed");

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].kwh, 6.0);
        assert_eq!(page[1].kwh, 5.0);
    }
}
