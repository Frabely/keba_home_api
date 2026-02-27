use std::net::{ToSocketAddrs, UdpSocket};
use std::time::Duration;

use serde_json::Value;
use thiserror::Error;

const UDP_TIMEOUT_SECONDS: u64 = 2;
const UDP_BUFFER_SIZE: usize = 4096;
const UDP_SOURCE_PORT_DEFAULT: u16 = 7090;

pub trait KebaClient: Send + Sync + 'static {
    fn get_report2(&self) -> Result<Value, KebaClientError>;
    fn get_report3(&self) -> Result<Value, KebaClientError>;
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

    fn send_command(&self, command: &str) -> Result<Value, KebaClientError> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", self.source_port))
            .or_else(|_| UdpSocket::bind("0.0.0.0:0"))?;
        socket.set_read_timeout(Some(self.timeout))?;
        socket.set_write_timeout(Some(self.timeout))?;
        let payload_with_crlf = format!("{command}\r\n");

        match self.send_payload(&socket, payload_with_crlf.as_bytes()) {
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
                self.send_payload(&socket, command.as_bytes())
            }
            Err(error) => Err(error),
        }
    }
}

impl KebaClient for KebaUdpClient {
    fn get_report2(&self) -> Result<Value, KebaClientError> {
        self.send_command("report 2")
    }

    fn get_report3(&self) -> Result<Value, KebaClientError> {
        self.send_command("report 3")
    }
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
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
}
