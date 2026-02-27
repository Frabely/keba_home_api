#[derive(Debug, Clone, PartialEq)]
pub struct SessionRecord {
    pub id: String,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: i64,
    pub energy_kwh: f64,
    pub source: String,
    pub status: String,
    pub started_reason: String,
    pub finished_reason: String,
    pub poll_interval_ms: i64,
    pub debounce_samples: i64,
    pub error_count_during_session: i64,
    pub station_id: Option<String>,
    pub created_at: String,
    pub raw_report2_start: Option<String>,
    pub raw_report3_start: Option<String>,
    pub raw_report2_end: Option<String>,
    pub raw_report3_end: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSessionRecord {
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: i64,
    pub energy_kwh: f64,
    pub source: String,
    pub status: String,
    pub started_reason: String,
    pub finished_reason: String,
    pub poll_interval_ms: i64,
    pub debounce_samples: i64,
    pub error_count_during_session: i64,
    pub station_id: Option<String>,
    pub created_at: String,
    pub raw_report2_start: Option<String>,
    pub raw_report3_start: Option<String>,
    pub raw_report2_end: Option<String>,
    pub raw_report3_end: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewLogEventRecord {
    pub created_at: String,
    pub level: String,
    pub code: String,
    pub message: String,
    pub source: String,
    pub station_id: Option<String>,
    pub details_json: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEventRecord {
    pub id: String,
    pub created_at: String,
    pub level: String,
    pub code: String,
    pub message: String,
    pub source: String,
    pub station_id: Option<String>,
    pub details_json: Option<String>,
}
