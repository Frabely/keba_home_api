use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::adapters::db::{open_connection, run_migrations};

static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn open_test_connection(test_name: &str) -> Connection {
    let template = ensure_template_db();
    let test_db_path = unique_test_db_path(test_name);

    if let Some(parent) = test_db_path.parent() {
        std::fs::create_dir_all(parent).expect("test db dir should be creatable");
    }

    std::fs::copy(&template, &test_db_path).expect("template db should be copied");
    open_connection(test_db_path.to_string_lossy().as_ref()).expect("test db should open")
}

fn ensure_template_db() -> PathBuf {
    static TEMPLATE_PATH: OnceLock<PathBuf> = OnceLock::new();

    TEMPLATE_PATH
        .get_or_init(|| {
            let template_path = std::env::var("TEST_DB_TEMPLATE_PATH")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(default_template_path);

            if let Some(parent) = template_path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent).expect("template parent dir should be creatable");
            }

            let mut connection = open_connection(template_path.to_string_lossy().as_ref())
                .expect("template db opens");
            run_migrations(&mut connection).expect("template migrations should succeed");

            template_path
        })
        .clone()
}

fn default_template_path() -> PathBuf {
    if cfg!(windows) {
        Path::new(".\\data\\keba_test.db").to_path_buf()
    } else {
        Path::new("./data/keba_test.db").to_path_buf()
    }
}

fn unique_test_db_path(test_name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let counter = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    Path::new("./target/testdb")
        .join(format!("{test_name}-{now}-{counter}.sqlite"))
        .to_path_buf()
}
