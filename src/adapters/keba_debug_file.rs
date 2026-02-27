use std::fs;
use std::io;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::Value;

use crate::adapters::keba_udp::{KebaClient, KebaClientError};

#[derive(Debug, Clone, Deserialize)]
struct ScriptFile {
    #[serde(default = "default_loop")]
    loop_forever: bool,
    report2: Vec<ScriptEvent>,
    report3: Vec<ScriptEvent>,
}

#[derive(Debug, Clone, Deserialize)]
struct ScriptEvent {
    ok: Option<Value>,
    error: Option<String>,
}

#[derive(Debug)]
struct ReplayState {
    report2_idx: usize,
    report3_idx: usize,
}

#[derive(Debug)]
pub struct KebaDebugFileClient {
    script: ScriptFile,
    state: Mutex<ReplayState>,
}

fn default_loop() -> bool {
    true
}

impl KebaDebugFileClient {
    pub fn from_file(path: &str) -> Result<Self, KebaClientError> {
        let content = fs::read_to_string(path).map_err(KebaClientError::Io)?;
        let script: ScriptFile = serde_json::from_str(&content).map_err(KebaClientError::Json)?;

        if script.report2.is_empty() {
            return Err(KebaClientError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "debug script must contain at least one report2 event",
            )));
        }
        if script.report3.is_empty() {
            return Err(KebaClientError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "debug script must contain at least one report3 event",
            )));
        }

        Ok(Self {
            script,
            state: Mutex::new(ReplayState {
                report2_idx: 0,
                report3_idx: 0,
            }),
        })
    }

    fn next_event(&self, for_report2: bool) -> Result<ScriptEvent, KebaClientError> {
        let mut state = self.state.lock().map_err(|_| {
            KebaClientError::Io(io::Error::other("debug replay state lock poisoned"))
        })?;

        let events = if for_report2 {
            &self.script.report2
        } else {
            &self.script.report3
        };

        let idx_ref = if for_report2 {
            &mut state.report2_idx
        } else {
            &mut state.report3_idx
        };

        if *idx_ref >= events.len() {
            if self.script.loop_forever {
                *idx_ref = 0;
            } else {
                return Err(KebaClientError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "debug replay finished",
                )));
            }
        }

        let event = events.get(*idx_ref).cloned().ok_or_else(|| {
            KebaClientError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "debug script event index out of bounds",
            ))
        })?;

        *idx_ref = idx_ref.saturating_add(1);

        Ok(event)
    }

    fn execute_event(event: ScriptEvent) -> Result<Value, KebaClientError> {
        match (event.ok, event.error) {
            (Some(payload), None) => Ok(payload),
            (None, Some(error)) => Err(map_script_error(&error)),
            _ => Err(KebaClientError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "script event must contain exactly one of: ok or error",
            ))),
        }
    }
}

fn map_script_error(kind: &str) -> KebaClientError {
    let normalized = kind.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "timeout" => KebaClientError::Io(io::Error::new(io::ErrorKind::TimedOut, kind)),
        "network_unreachable" | "internet_down" => {
            KebaClientError::Io(io::Error::new(io::ErrorKind::NetworkUnreachable, kind))
        }
        "host_unreachable" | "wallbox_unreachable" => {
            KebaClientError::Io(io::Error::new(io::ErrorKind::HostUnreachable, kind))
        }
        "connection_refused" => {
            KebaClientError::Io(io::Error::new(io::ErrorKind::ConnectionRefused, kind))
        }
        "broken_pipe" => KebaClientError::Io(io::Error::new(io::ErrorKind::BrokenPipe, kind)),
        "invalid_json" => {
            let parse_err = serde_json::from_str::<Value>("not json")
                .expect_err("invalid json literal must fail");
            KebaClientError::Json(parse_err)
        }
        _ => KebaClientError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown scripted error kind: {kind}"),
        )),
    }
}

impl KebaClient for KebaDebugFileClient {
    fn get_report2(&self) -> Result<Value, KebaClientError> {
        Self::execute_event(self.next_event(true)?)
    }

    fn get_report3(&self) -> Result<Value, KebaClientError> {
        Self::execute_event(self.next_event(false)?)
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use crate::adapters::keba_udp::KebaClient;

    use super::KebaDebugFileClient;

    fn fixture(path: &str) -> String {
        format!(
            "{}/testdata/debug/{path}",
            env!("CARGO_MANIFEST_DIR").replace("\\", "/")
        )
    }

    #[test]
    fn replays_and_loops_scripted_payloads() {
        let client = KebaDebugFileClient::from_file(&fixture("happy_loop.json"))
            .expect("script should load");

        let r2_a = client.get_report2().expect("report2 #1 should succeed");
        let r2_b = client.get_report2().expect("report2 #2 should succeed");
        let r2_c = client
            .get_report2()
            .expect("report2 should loop to first event");

        assert_eq!(r2_a["Plug"], 0);
        assert_eq!(r2_b["Plug"], 7);
        assert_eq!(r2_c["Plug"], 0);
    }

    #[test]
    fn simulates_network_and_host_failures() {
        let client =
            KebaDebugFileClient::from_file(&fixture("network_failures.json")).expect("script");

        let err1 = client
            .get_report2()
            .expect_err("first event should be network unreachable");
        let err2 = client
            .get_report2()
            .expect_err("second event should be host unreachable");

        match err1 {
            crate::adapters::keba_udp::KebaClientError::Io(io) => {
                assert_eq!(io.kind(), ErrorKind::NetworkUnreachable)
            }
            _ => panic!("expected io error"),
        }
        match err2 {
            crate::adapters::keba_udp::KebaClientError::Io(io) => {
                assert_eq!(io.kind(), ErrorKind::HostUnreachable)
            }
            _ => panic!("expected io error"),
        }
    }

    #[test]
    fn simulates_invalid_json_error() {
        let client =
            KebaDebugFileClient::from_file(&fixture("invalid_json_error.json")).expect("script");

        let err = client
            .get_report3()
            .expect_err("script should inject invalid json error");
        match err {
            crate::adapters::keba_udp::KebaClientError::Json(_) => {}
            _ => panic!("expected json error"),
        }
    }

    #[test]
    fn rejects_script_with_missing_sequences() {
        let err = KebaDebugFileClient::from_file(&fixture("missing_sequences.json"))
            .expect_err("missing report3 should fail");

        match err {
            crate::adapters::keba_udp::KebaClientError::Io(io) => {
                assert_eq!(io.kind(), ErrorKind::InvalidData)
            }
            _ => panic!("expected invalid data io error"),
        }
    }

    #[test]
    fn rejects_script_with_invalid_top_level_json() {
        let err = KebaDebugFileClient::from_file(&fixture("invalid_top_level_json.json"))
            .expect_err("invalid json should fail");

        match err {
            crate::adapters::keba_udp::KebaClientError::Json(_) => {}
            _ => panic!("expected json parse error"),
        }
    }

    #[test]
    fn rejects_unknown_error_kind() {
        let client =
            KebaDebugFileClient::from_file(&fixture("unknown_error_kind.json")).expect("script");

        let err = client
            .get_report2()
            .expect_err("unknown scripted error must fail");
        match err {
            crate::adapters::keba_udp::KebaClientError::Io(io) => {
                assert_eq!(io.kind(), ErrorKind::InvalidInput)
            }
            _ => panic!("expected invalid input io error"),
        }
    }
}
