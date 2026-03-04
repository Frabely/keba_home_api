use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

const UDP_TIMEOUT_SECONDS: u64 = 6;
const UDP_BUFFER_SIZE: usize = 4096;
const UDP_SOURCE_PORT_DEFAULT: u16 = 7090;
const UDP_TRANSIENT_RETRY_ATTEMPTS: usize = 1;

pub trait KebaClient: Send + Sync + 'static {
    fn get_report2(&self) -> Result<Value, KebaClientError>;
    fn get_report3(&self) -> Result<Value, KebaClientError>;
    fn get_report100(&self) -> Result<Value, KebaClientError>;
    fn get_report101(&self) -> Result<Value, KebaClientError>;

    fn get_report(&self, report_id: u16) -> Result<Value, KebaClientError> {
        match report_id {
            100 => self.get_report100(),
            101 => self.get_report101(),
            _ => Err(KebaClientError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("report {report_id} is not supported by this client"),
            ))),
        }
    }
}

#[derive(Debug, Error)]
pub enum KebaClientError {
    #[error("failed to resolve KEBA endpoint: {0}")]
    Resolve(std::io::Error),
    #[error("transport communication failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse KEBA response as JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct KebaUdpClient {
    target: std::net::SocketAddr,
    timeout: Duration,
    source_port: u16,
}

impl KebaUdpClient {
    pub fn new(host: &str, port: u16) -> Result<Self, KebaClientError> {
        let mut addrs = format!("{host}:{port}")
            .to_socket_addrs()
            .map_err(KebaClientError::Resolve)?;
        let target = addrs.next().ok_or_else(|| {
            KebaClientError::Resolve(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "no socket address resolved for KEBA endpoint",
            ))
        })?;

        Ok(Self {
            target,
            timeout: Duration::from_secs(UDP_TIMEOUT_SECONDS),
            source_port: UDP_SOURCE_PORT_DEFAULT,
        })
    }

    #[cfg(test)]
    fn with_timeout_for_tests(
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<Self, KebaClientError> {
        let mut addrs = format!("{host}:{port}")
            .to_socket_addrs()
            .map_err(KebaClientError::Resolve)?;
        let target = addrs.next().ok_or_else(|| {
            KebaClientError::Resolve(std::io::Error::new(
                std::io::ErrorKind::AddrNotAvailable,
                "no socket address resolved for KEBA endpoint",
            ))
        })?;

        Ok(Self {
            target,
            timeout,
            source_port: 0,
        })
    }

    fn send_payload(&self, socket: &UdpSocket, payload: &[u8]) -> Result<Value, KebaClientError> {
        socket.send_to(payload, self.target)?;

        let mut buffer = [0_u8; UDP_BUFFER_SIZE];
        let (size, _) = socket.recv_from(&mut buffer)?;
        serde_json::from_slice(&buffer[..size]).map_err(KebaClientError::from)
    }

    fn is_transient_io_error(error: &KebaClientError) -> bool {
        matches!(
            error,
            KebaClientError::Io(io)
                if matches!(
                    io.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                )
        )
    }

    fn send_command_once(
        &self,
        socket: &UdpSocket,
        command: &str,
    ) -> Result<Value, KebaClientError> {
        let payload_with_crlf = format!("{command}\r\n");
        match self.send_payload(socket, payload_with_crlf.as_bytes()) {
            Ok(response) => Ok(response),
            Err(KebaClientError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                tracing::debug!(
                    command,
                    "udp command with CRLF timed out, retrying without line ending"
                );
                self.send_payload(socket, command.as_bytes())
            }
            Err(error) => Err(error),
        }
    }

    fn send_command(&self, command: &str) -> Result<Value, KebaClientError> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", self.source_port))
            .or_else(|_| UdpSocket::bind("0.0.0.0:0"))?;
        socket.set_read_timeout(Some(self.timeout))?;
        socket.set_write_timeout(Some(self.timeout))?;
        let mut retries = 0_usize;

        loop {
            match self.send_command_once(&socket, command) {
                Ok(response) => return Ok(response),
                Err(error)
                    if retries < UDP_TRANSIENT_RETRY_ATTEMPTS
                        && Self::is_transient_io_error(&error) =>
                {
                    retries += 1;
                    tracing::debug!(
                        command,
                        retry = retries,
                        max_retries = UDP_TRANSIENT_RETRY_ATTEMPTS,
                        "udp command transient timeout, retrying full command"
                    );
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn get_report(&self, report_id: u16) -> Result<Value, KebaClientError> {
        self.send_command(&format!("report {report_id}"))
    }

    pub fn get_report100(&self) -> Result<Value, KebaClientError> {
        self.get_report(100)
    }

    pub fn get_report101(&self) -> Result<Value, KebaClientError> {
        self.get_report(101)
    }
}

impl KebaClient for KebaUdpClient {
    fn get_report2(&self) -> Result<Value, KebaClientError> {
        self.send_command("report 2")
    }

    fn get_report3(&self) -> Result<Value, KebaClientError> {
        self.send_command("report 3")
    }

    fn get_report100(&self) -> Result<Value, KebaClientError> {
        self.get_report(100)
    }

    fn get_report101(&self) -> Result<Value, KebaClientError> {
        self.get_report(101)
    }

    fn get_report(&self, report_id: u16) -> Result<Value, KebaClientError> {
        KebaUdpClient::get_report(self, report_id)
    }
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::{KebaClient, KebaUdpClient};

    #[test]
    fn retries_without_line_ending_when_crlf_variant_times_out() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be set");
        let responder_port = responder
            .local_addr()
            .expect("responder addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 256];
            loop {
                let Ok((size, from)) = responder.recv_from(&mut buffer) else {
                    break;
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).to_string();

                if cmd == "shutdown-test-responder" {
                    break;
                }

                // Emulates a stricter responder that only accepts exact commands.
                let payload = match cmd.as_str() {
                    "report 2" => Some(r#"{"Plug":7,"Seconds":12}"#),
                    _ => None,
                };

                if let Some(payload) = payload {
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let client = KebaUdpClient::with_timeout_for_tests(
            "127.0.0.1",
            responder_port,
            Duration::from_millis(40),
        )
        .expect("client should be created");

        let report2 = client.get_report2().expect("report2 should be fetched");
        assert_eq!(report2["Plug"], 7);
        assert_eq!(report2["Seconds"], 12);

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate");
    }

    #[test]
    fn retries_full_command_once_for_transient_timeouts_on_all_reports() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be set");
        let responder_port = responder
            .local_addr()
            .expect("responder addr should be available")
            .port();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_thread = Arc::clone(&attempts);

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 256];
            loop {
                let Ok((size, from)) = responder.recv_from(&mut buffer) else {
                    break;
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).to_string();

                if cmd == "shutdown-test-responder" {
                    break;
                }

                let current = attempts_for_thread.fetch_add(1, Ordering::Relaxed) + 1;
                if current < 3 {
                    continue;
                }

                if cmd == "report 100\r\n" || cmd == "report 100" {
                    responder
                        .send_to(br#"{"ended":10,"Sec":12}"#, from)
                        .expect("responder send should succeed");
                }
            }
        });

        let client = KebaUdpClient::with_timeout_for_tests(
            "127.0.0.1",
            responder_port,
            Duration::from_millis(40),
        )
        .expect("client should be created");

        let report100 = client.get_report100().expect("report100 should be fetched");
        assert_eq!(report100["ended"], 10);
        assert_eq!(attempts.load(Ordering::Relaxed), 3);

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate");
    }
}
